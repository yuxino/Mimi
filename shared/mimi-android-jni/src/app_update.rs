//! Android's manual update request. The only destination is the public release
//! endpoint; this adapter never accepts a provider configuration or credential.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use mimi_runtime::{clients::provider_network::ProviderNetwork, core::network_proxy::ProxyConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle};

const LATEST_URL: &str = "https://api.github.com/repos/yuxino/Mimi/releases/latest";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INVALID: &str = "app_update_invalid_request";
const FAILED: &str = "app_update_failed";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Network {
    proxy: ProxyConfig,
    trust_roots: Vec<String>,
}

impl Network {
    fn resolve(self) -> Result<ProviderNetwork, &'static str> {
        let roots = self
            .trust_roots
            .iter()
            .map(|root| STANDARD.decode(root).map_err(|_| INVALID))
            .collect::<Result<Vec<_>, _>>()?;
        ProviderNetwork::resolve(&self.proxy)
            .and_then(|network| network.with_trust_roots(roots))
            .map_err(|_| INVALID)
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
enum Request {
    #[serde(rename = "app_update_endpoint")]
    Endpoint {},
    #[serde(rename = "app_update_start")]
    Start { network: Network },
    #[serde(rename = "app_update_poll")]
    Poll { handle: u64 },
    #[serde(rename = "app_update_cancel")]
    Cancel { handle: u64 },
}

struct Pending {
    response: oneshot::Receiver<Result<String, ()>>,
    task: JoinHandle<()>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct Requests {
    next_id: u64,
    pending: HashMap<u64, Pending>,
}

impl Requests {
    fn start(
        &mut self,
        fetch: impl Future<Output = Result<String, ()>> + Send + 'static,
    ) -> Result<u64, &'static str> {
        if self.pending.len() >= 2 {
            return Err("app_update_capacity");
        }
        self.next_id = self.next_id.checked_add(1).ok_or(FAILED)?;
        let (sender, response) = oneshot::channel();
        let task = crate::runtime::executor().spawn(async move {
            let _ = sender.send(fetch.await);
        });
        self.pending
            .insert(self.next_id, Pending { response, task });
        Ok(self.next_id)
    }

    fn poll(&mut self, handle: u64) -> Result<Value, &'static str> {
        let request = self.pending.get_mut(&handle).ok_or(INVALID)?;
        let response = match request.response.try_recv() {
            Ok(result) => result.ok(),
            Err(oneshot::error::TryRecvError::Empty) => return Ok(json!({"finished": false})),
            Err(oneshot::error::TryRecvError::Closed) => None,
        };
        self.pending.remove(&handle);
        Ok(json!({"finished": true, "response": response}))
    }
}

fn requests() -> &'static Mutex<Requests> {
    static REQUESTS: OnceLock<Mutex<Requests>> = OnceLock::new();
    REQUESTS.get_or_init(Default::default)
}

pub(crate) fn exchange(value: &Value) -> Result<Value, &'static str> {
    let request: Request = serde_json::from_value(value.clone()).map_err(|_| INVALID)?;
    match request {
        Request::Endpoint {} => Ok(json!({"endpoint": LATEST_URL})),
        Request::Start { network } => {
            let network = network.resolve()?;
            let handle = requests()
                .lock()
                .map_err(|_| FAILED)?
                .start(fetch_release(network, LATEST_URL))?;
            Ok(json!({"handle": handle}))
        }
        Request::Poll { handle } => requests().lock().map_err(|_| FAILED)?.poll(handle),
        Request::Cancel { handle } => {
            requests()
                .lock()
                .map_err(|_| FAILED)?
                .pending
                .remove(&handle);
            Ok(json!({}))
        }
    }
}

async fn fetch_release(network: ProviderNetwork, endpoint: &str) -> Result<String, ()> {
    tokio::time::timeout(REQUEST_TIMEOUT, async {
        let endpoint = endpoint.parse().map_err(|_| ())?;
        let client = network
            .http_client_builder(&endpoint)
            .map_err(|_| ())?
            .connect_timeout(Duration::from_secs(5))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| ())?;
        let mut response = client
            .get(endpoint)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "Mimi-Android")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|_| ())?;
        if response.status().as_u16() != 200
            || response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimi_runtime::core::network_proxy::ProxyMode;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn response(body: Vec<u8>) -> Result<String, ()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/release", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(3), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.ends_with(b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0 && request.len() + count <= 8192);
                    request.extend_from_slice(&buffer[..count]);
                }
                assert!(std::str::from_utf8(&request)
                    .unwrap()
                    .starts_with("GET /release HTTP/1.1\r\n"));
                let _ = socket.write_all(&body).await;
            })
            .await
            .unwrap();
        });
        let network = ProviderNetwork::resolve(&ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        })
        .unwrap();
        let result = fetch_release(network, &endpoint).await;
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn accepts_only_bounded_success_responses_and_does_not_follow_redirects() {
        assert_eq!(
            response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()).await,
            Ok("{}".into())
        );
        for response_bytes in [
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\n\r\n".to_vec(),
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_vec(),
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_RESPONSE_BYTES + 1).into_bytes(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n\xff".to_vec(),
        ] {
            assert_eq!(response(response_bytes).await, Err(()));
        }
        let mut chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        chunked.extend_from_slice(format!("{:x}\r\n", MAX_RESPONSE_BYTES + 1).as_bytes());
        chunked.extend(vec![b'x'; MAX_RESPONSE_BYTES + 1]);
        chunked.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(response(chunked).await, Err(()));
    }

    #[tokio::test]
    async fn tickets_bound_work_cancel_owned_tasks_and_consume_completed_responses() {
        let mut registry = Requests::default();
        let dropped = Arc::new(tokio::sync::Notify::new());
        struct DropNotice(Arc<tokio::sync::Notify>);
        impl Drop for DropNotice {
            fn drop(&mut self) {
                self.0.notify_one();
            }
        }
        let notice = DropNotice(dropped.clone());
        let pending = registry
            .start(async move {
                let _notice = notice;
                std::future::pending().await
            })
            .unwrap();
        let complete = registry.start(async { Ok("{}".to_owned()) }).unwrap();
        assert_eq!(
            registry.start(async { Ok(String::new()) }),
            Err("app_update_capacity")
        );
        assert_eq!(registry.poll(pending).unwrap()["finished"], false);
        registry.pending.remove(&pending);
        tokio::time::timeout(Duration::from_secs(1), dropped.notified())
            .await
            .unwrap();
        assert!(registry.poll(pending).is_err());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let result = registry.poll(complete).unwrap();
            if result["finished"] == true {
                assert_eq!(result["response"], "{}");
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
        assert!(registry.poll(complete).is_err());
        assert!(registry.pending.is_empty());
    }

    #[test]
    fn fixed_destination_has_no_configuration_or_arbitrary_url_input() {
        assert_eq!(
            exchange(&json!({"operation": "app_update_endpoint"})).unwrap()["endpoint"],
            LATEST_URL
        );
        for value in [
            json!({"operation": "app_update_endpoint", "endpoint": "https://untrusted.invalid"}),
            json!({"operation": "app_update_start", "network": {"proxy": {"mode": "direct"}, "trustRoots": []}}),
            json!({"operation": "app_update_start", "configuration": {"apiKey": "synthetic"}}),
        ] {
            assert_eq!(exchange(&value), Err(INVALID));
        }
    }
}
