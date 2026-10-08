//! One immutable proxy snapshot shared by provider HTTP, ASR and WebSockets.
//! Never silently fall back to direct after a configured proxy rejects/fails.
use crate::core::network_proxy::{validated_proxy_url, ProxyConfig, ProxyConfigError, ProxyMode};
use hyper_util::client::proxy::matcher::Matcher;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{
    handshake::client::{Request, Response},
    Error,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type ProviderNetworkError = ProxyConfigError;

#[derive(Clone)]
enum Routing {
    Direct,
    System(Arc<Matcher>),
    Custom(url::Url),
}

#[derive(Clone)]
pub struct ProviderNetwork {
    mode: ProxyMode,
    routing: Routing,
    trust_roots: Option<Arc<PlatformTrustRoots>>,
}

struct PlatformTrustRoots {
    websocket: Arc<rustls::ClientConfig>,
    http: Vec<reqwest::Certificate>,
}

impl std::fmt::Debug for ProviderNetwork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderNetwork")
            .field("mode", &self.mode)
            .finish()
    }
}

impl Default for ProviderNetwork {
    fn default() -> Self {
        Self {
            mode: ProxyMode::System,
            routing: Routing::System(Arc::new(Matcher::from_system())),
            trust_roots: None,
        }
    }
}

impl ProviderNetwork {
    pub fn resolve(config: &ProxyConfig) -> Result<Self, ProviderNetworkError> {
        let config = config.validate()?;
        Ok(match config.mode {
            ProxyMode::Direct => Self {
                mode: config.mode,
                routing: Routing::Direct,
                trust_roots: None,
            },
            ProxyMode::System => Self::default(),
            ProxyMode::Custom => Self {
                mode: config.mode,
                routing: Routing::Custom(validated_proxy_url(
                    config.url.as_deref().ok_or(ProxyConfigError::InvalidUrl)?,
                )?),
                trust_roots: None,
            },
        })
    }

    /// Capture a platform trust snapshot once, using it identically for HTTP
    /// and WebSockets. Invalid or missing platform roots never fall back to
    /// another trust store. Desktop keeps its native-root path unless injected.
    pub fn with_trust_roots(
        mut self,
        certificates: Vec<Vec<u8>>,
    ) -> Result<Self, ProviderNetworkError> {
        if certificates.is_empty()
            || certificates.len() > 512
            || certificates
                .iter()
                .any(|der| der.is_empty() || der.len() > 64 * 1024)
            || certificates.iter().map(Vec::len).sum::<usize>() > 4 * 1024 * 1024
        {
            return Err(ProxyConfigError::InvalidTrustRoots);
        }
        let mut roots = rustls::RootCertStore::empty();
        let mut http = Vec::with_capacity(certificates.len());
        for der in certificates {
            roots
                .add(rustls::pki_types::CertificateDer::from(der.clone()))
                .map_err(|_| ProxyConfigError::InvalidTrustRoots)?;
            http.push(
                reqwest::Certificate::from_der(&der)
                    .map_err(|_| ProxyConfigError::InvalidTrustRoots)?,
            );
        }
        let websocket = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| ProxyConfigError::InvalidTrustRoots)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        self.trust_roots = Some(Arc::new(PlatformTrustRoots {
            websocket: Arc::new(websocket),
            http,
        }));
        Ok(self)
    }

    /// Endpoint-specific builders make unsupported/authenticated system proxy
    /// settings fail closed. A custom reqwest resolver returning None on error
    /// would quietly bypass the configured proxy. Provider redirects are off so
    /// this captured route cannot be applied to a different destination.
    pub fn http_client_builder(
        &self,
        endpoint: &url::Url,
    ) -> Result<reqwest::ClientBuilder, ProviderNetworkError> {
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        if let Some(roots) = &self.trust_roots {
            builder = builder.tls_certs_only(roots.http.iter().cloned());
        }
        if let Some(proxy) = self.route(endpoint)? {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy.as_str()).map_err(|_| ProxyConfigError::BuilderFailed)?,
            );
        }
        Ok(builder)
    }

    fn route(&self, endpoint: &url::Url) -> Result<Option<url::Url>, ProviderNetworkError> {
        match &self.routing {
            Routing::Direct => Ok(None),
            Routing::Custom(proxy) => Ok(Some(proxy.clone())),
            Routing::System(matcher) => {
                let mut destination = endpoint.clone();
                let scheme = match endpoint.scheme() {
                    "ws" => "http",
                    "wss" => "https",
                    other => other,
                };
                destination
                    .set_scheme(scheme)
                    .map_err(|_| ProxyConfigError::InvalidUrl)?;
                let uri = destination
                    .as_str()
                    .parse()
                    .map_err(|_| ProxyConfigError::InvalidUrl)?;
                let Some(proxy) = matcher.intercept(&uri) else {
                    return Ok(None);
                };
                if proxy.basic_auth().is_some() || proxy.raw_auth().is_some() {
                    return Err(ProxyConfigError::AuthenticationUnsupported);
                }
                validated_proxy_url(&proxy.uri().to_string()).map(Some)
            }
        }
    }
}

/// The outer clients retain their provider-specific readiness timeout. This
/// bound also covers TCP, CONNECT/SOCKS negotiation, destination TLS and upgrade.
pub async fn websocket(
    request: Request,
    network: &ProviderNetwork,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error> {
    websocket_configured(request, network, None).await
}

/// Recognition-only protocols may impose a tighter incoming JSON bound.
/// Other providers retain their existing limits, including output audio.
pub async fn websocket_with_message_limit(
    request: Request,
    network: &ProviderNetwork,
    maximum_bytes: usize,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error> {
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(maximum_bytes))
        .max_frame_size(Some(maximum_bytes));
    websocket_configured(request, network, Some(config)).await
}

async fn websocket_configured(
    request: Request,
    network: &ProviderNetwork,
    config: Option<tokio_tungstenite::tungstenite::protocol::WebSocketConfig>,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let destination = url::Url::parse(&request.uri().to_string()).map_err(|_| proxy_error())?;
        let destination_host = host(&destination)?;
        let destination_port = destination
            .port_or_known_default()
            .ok_or_else(proxy_error)?;
        let route = network.route(&destination).map_err(|_| proxy_error())?;
        let mut socket = match route {
            None => TcpStream::connect((destination_host.as_str(), destination_port))
                .await
                .map_err(|_| proxy_error())?,
            Some(proxy) => {
                let proxy_host = host(&proxy)?;
                let proxy_port = proxy.port_or_known_default().ok_or_else(proxy_error)?;
                let mut socket = TcpStream::connect((proxy_host.as_str(), proxy_port))
                    .await
                    .map_err(|_| proxy_error())?;
                match proxy.scheme() {
                    "http" => {
                        connect_tunnel(&mut socket, &destination_host, destination_port).await?
                    }
                    "socks5" | "socks5h" => {
                        socks_tunnel(
                            &mut socket,
                            &destination_host,
                            destination_port,
                            proxy.scheme() == "socks5h",
                        )
                        .await?
                    }
                    _ => return Err(proxy_error()),
                }
                socket
            }
        };
        // The destination's WSS hostname and native CA verification remain
        // unchanged: the proxy provides a byte tunnel, never TLS termination.
        socket.flush().await.map_err(|_| proxy_error())?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let connector = network
            .trust_roots
            .as_ref()
            .map(|roots| tokio_tungstenite::Connector::Rustls(Arc::clone(&roots.websocket)));
        tokio_tungstenite::client_async_tls_with_config(request, socket, config, connector).await
    })
    .await
    .map_err(|_| proxy_error())?
}

fn proxy_error() -> Error {
    Error::Io(std::io::Error::other("provider_network_connection_failed"))
}

fn host(endpoint: &url::Url) -> Result<String, Error> {
    match endpoint.host().ok_or_else(proxy_error)? {
        url::Host::Domain(domain) => Ok(domain.to_owned()),
        url::Host::Ipv4(address) => Ok(address.to_string()),
        url::Host::Ipv6(address) => Ok(address.to_string()),
    }
}

async fn connect_tunnel(socket: &mut TcpStream, host: &str, port: u16) -> Result<(), Error> {
    let authority = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    socket
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await
        .map_err(|_| proxy_error())?;
    let mut header = Vec::with_capacity(512);
    // Read exactly through the header so no early TLS/WebSocket byte is lost.
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() == 8 * 1024 {
            return Err(proxy_error());
        }
        header.push(socket.read_u8().await.map_err(|_| proxy_error())?);
    }
    let status = std::str::from_utf8(&header)
        .ok()
        .and_then(|header| header.lines().next())
        .and_then(|line| {
            let mut fields = line.split_whitespace();
            matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1"))
                .then(|| fields.next()?.parse::<u16>().ok())
                .flatten()
        });
    if status != Some(200) {
        return Err(proxy_error());
    }
    Ok(())
}

async fn socks_tunnel(
    socket: &mut TcpStream,
    host: &str,
    port: u16,
    remote_dns: bool,
) -> Result<(), Error> {
    socket
        .write_all(&[5, 1, 0])
        .await
        .map_err(|_| proxy_error())?;
    let mut greeting = [0; 2];
    socket
        .read_exact(&mut greeting)
        .await
        .map_err(|_| proxy_error())?;
    if greeting != [5, 0] {
        return Err(proxy_error());
    }
    let mut request = vec![5, 1, 0];
    let address = host.parse::<std::net::IpAddr>().ok();
    let address = if remote_dns || address.is_some() {
        address
    } else {
        Some(
            tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| proxy_error())?
                .next()
                .ok_or_else(proxy_error)?
                .ip(),
        )
    };
    match address {
        Some(std::net::IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Some(std::net::IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        None => {
            let length = u8::try_from(host.len()).map_err(|_| proxy_error())?;
            request.extend_from_slice(&[3, length]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    socket
        .write_all(&request)
        .await
        .map_err(|_| proxy_error())?;
    let mut response = [0; 4];
    socket
        .read_exact(&mut response)
        .await
        .map_err(|_| proxy_error())?;
    if response[..3] != [5, 0, 0] {
        return Err(proxy_error());
    }
    let length = match response[3] {
        1 => 4,
        4 => 16,
        3 => socket.read_u8().await.map_err(|_| proxy_error())? as usize,
        _ => return Err(proxy_error()),
    };
    let mut bound_address = [0; 257];
    socket
        .read_exact(&mut bound_address[..length + 2])
        .await
        .map_err(|_| proxy_error())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    fn custom(endpoint: String) -> ProviderNetwork {
        ProviderNetwork::resolve(&ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some(endpoint),
        })
        .unwrap()
    }

    fn direct() -> ProviderNetwork {
        ProviderNetwork::resolve(&ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        })
        .unwrap()
    }

    #[test]
    fn invalid_or_missing_platform_roots_fail_without_a_native_fallback() {
        for certificates in [
            vec![],
            vec![vec![]],
            vec![vec![0x30, 0x01, 0x00]],
            vec![vec![0; 64 * 1024 + 1]],
        ] {
            assert_eq!(
                direct().with_trust_roots(certificates).unwrap_err(),
                ProxyConfigError::InvalidTrustRoots
            );
        }
        assert!(direct().trust_roots.is_none());
    }

    #[tokio::test]
    async fn recognition_only_websocket_enforces_its_explicit_incoming_limit() {
        use futures_util::SinkExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    "x".repeat(257).into(),
                ))
                .await
                .unwrap();
        });
        let (mut socket, _) = websocket_with_message_limit(
            format!("ws://{address}/audio3")
                .into_client_request()
                .unwrap(),
            &direct(),
            256,
        )
        .await
        .unwrap();
        assert_eq!(socket.get_config().max_message_size, Some(256));
        assert_eq!(socket.get_config().max_frame_size, Some(256));
        assert!(matches!(socket.next().await, Some(Err(Error::Capacity(_)))));
        server.await.unwrap();
    }

    async fn read_header(socket: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            assert!(bytes.len() < 8192);
            bytes.push(socket.read_u8().await.unwrap());
        }
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn system_snapshot_uses_same_https_route_for_wss_and_http_bypass_for_ws() {
        let network = ProviderNetwork {
            mode: ProxyMode::System,
            routing: Routing::System(Arc::new(
                Matcher::builder()
                    .http("http://127.0.0.1:7890")
                    .https("socks5h://127.0.0.1:1080")
                    .no("localhost,127.0.0.1")
                    .build(),
            )),
            trust_roots: None,
        };
        for scheme in ["http", "ws"] {
            let endpoint = url::Url::parse(&format!("{scheme}://fixture.invalid/path")).unwrap();
            assert_eq!(
                network.route(&endpoint).unwrap().unwrap().port(),
                Some(7890)
            );
            assert!(network
                .route(&url::Url::parse(&format!("{scheme}://127.0.0.1:9090/path")).unwrap())
                .unwrap()
                .is_none());
        }
        for scheme in ["https", "wss"] {
            let endpoint = url::Url::parse(&format!("{scheme}://fixture.invalid/path")).unwrap();
            assert_eq!(
                network.route(&endpoint).unwrap().unwrap().scheme(),
                "socks5h"
            );
        }
        assert!(direct()
            .route(&url::Url::parse("wss://fixture.invalid/path").unwrap())
            .unwrap()
            .is_none());
        assert!(!format!("{network:?}").contains("127.0.0.1"));
    }

    #[test]
    fn unsupported_or_authenticated_system_routes_are_errors_not_direct_fallbacks() {
        for (proxy, error) in [
            (
                "http://private:secret@127.0.0.1:7890",
                ProxyConfigError::AuthenticationUnsupported,
            ),
            (
                "https://127.0.0.1:7890",
                ProxyConfigError::UnsupportedScheme,
            ),
            (
                "socks4://127.0.0.1:1080",
                ProxyConfigError::UnsupportedScheme,
            ),
        ] {
            let network = ProviderNetwork {
                mode: ProxyMode::System,
                routing: Routing::System(Arc::new(Matcher::builder().all(proxy).build())),
                trust_roots: None,
            };
            for endpoint in ["https://fixture.invalid/", "wss://fixture.invalid/"] {
                assert_eq!(
                    network.route(&url::Url::parse(endpoint).unwrap()),
                    Err(error)
                );
            }
            assert!(network
                .http_client_builder(&url::Url::parse("https://fixture.invalid/").unwrap())
                .is_err());
        }
    }

    #[tokio::test]
    async fn custom_http_proxy_receives_absolute_http_request_without_origin_dns() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_header(&mut socket).await;
            assert!(request.starts_with("GET http://fixture.invalid/path HTTP/1.1\r\n"));
            assert!(!request.to_lowercase().contains("proxy-authorization"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let endpoint = url::Url::parse("http://fixture.invalid/path").unwrap();
        let client = network
            .http_client_builder(&endpoint)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            client
                .get(endpoint)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn direct_http_and_websocket_reach_the_explicit_loopback_origin() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(read_header(&mut socket)
                .await
                .starts_with("GET /path HTTP/1.1"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            let (socket, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
            assert!(matches!(
                websocket.next().await,
                None | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | Some(Err(_))
            ));
        });
        let network = direct();
        let endpoint = url::Url::parse(&format!("http://{address}/path")).unwrap();
        assert_eq!(
            network
                .http_client_builder(&endpoint)
                .unwrap()
                .build()
                .unwrap()
                .get(endpoint)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        let (socket, _) = websocket(
            format!("ws://{address}/live")
                .into_client_request()
                .unwrap(),
            &network,
        )
        .await
        .unwrap();
        drop(socket);
        server.await.unwrap();
    }

    #[tokio::test]
    // Tungstenite's handshake callback requires its unboxed ErrorResponse.
    #[allow(clippy::result_large_err)]
    async fn http_connect_tunnels_websocket_and_keeps_service_headers_out_of_connect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_header(&mut socket).await;
            assert!(request.starts_with("CONNECT fixture.invalid:80 HTTP/1.1"));
            assert!(!request.to_lowercase().contains("authorization"));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            let mut websocket =
                tokio_tungstenite::accept_hdr_async(socket, |request: &Request, response| {
                    assert_eq!(
                        request.headers().get("authorization").unwrap(),
                        "synthetic-service-key"
                    );
                    assert_eq!(request.uri().path(), "/live");
                    Ok(response)
                })
                .await
                .unwrap();
            let _ = websocket.next().await;
        });
        let mut request = "ws://fixture.invalid/live".into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "synthetic-service-key".parse().unwrap());
        let (socket, _) = websocket(request, &network).await.unwrap();
        drop(socket);
        server.await.unwrap();
    }

    async fn socks_handshake(socket: &mut TcpStream) -> (u8, Vec<u8>, u16) {
        let mut greeting = [0; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, 0]);
        socket.write_all(&[5, 0]).await.unwrap();
        let mut header = [0; 4];
        socket.read_exact(&mut header).await.unwrap();
        assert_eq!(header[..3], [5, 1, 0]);
        let count = match header[3] {
            1 => 4,
            4 => 16,
            3 => socket.read_u8().await.unwrap() as usize,
            _ => panic!("invalid address kind"),
        };
        let mut address = vec![0; count];
        socket.read_exact(&mut address).await.unwrap();
        let port = socket.read_u16().await.unwrap();
        socket
            .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
            .await
            .unwrap();
        (header[3], address, port)
    }

    #[tokio::test]
    async fn socks5h_uses_remote_dns_for_both_http_and_websocket() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("socks5h://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (kind, address, port) = socks_handshake(&mut socket).await;
            assert_eq!(
                (kind, address.as_slice(), port),
                (3, b"fixture.invalid".as_slice(), 80)
            );
            assert!(read_header(&mut socket)
                .await
                .starts_with("GET /path HTTP/1.1"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            let (mut socket, _) = listener.accept().await.unwrap();
            let (kind, address, port) = socks_handshake(&mut socket).await;
            assert_eq!(
                (kind, address.as_slice(), port),
                (3, b"fixture.invalid".as_slice(), 80)
            );
            let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let _ = websocket.next().await;
        });
        let endpoint = url::Url::parse("http://fixture.invalid/path").unwrap();
        assert_eq!(
            network
                .http_client_builder(&endpoint)
                .unwrap()
                .build()
                .unwrap()
                .get(endpoint)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "ok"
        );
        let (socket, _) = websocket(
            "ws://fixture.invalid/live".into_client_request().unwrap(),
            &network,
        )
        .await
        .unwrap();
        drop(socket);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_local_address_and_http_connect_rejection_never_fall_back() {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = destination.local_addr().unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("http://{}", proxy.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let _ = read_header(&mut socket).await;
            socket.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 22\r\n\r\nprivate-proxy-response").await.unwrap();
        });
        let error = websocket(
            format!("ws://{origin}/live").into_client_request().unwrap(),
            &network,
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().contains("private-proxy-response"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), destination.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("socks5://{}", proxy.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let (kind, address, port) = socks_handshake(&mut socket).await;
            assert_eq!(
                (kind, address, port),
                (1, vec![127, 0, 0, 1], origin.port())
            );
            // The fixture accepted the tunnel but deliberately refuses the WS.
            let _ = read_header(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        assert!(websocket(
            format!("ws://{origin}/live").into_client_request().unwrap(),
            &network
        )
        .await
        .is_err());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), destination.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn destination_tls_is_attempted_after_connect_and_bad_tls_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(read_header(&mut socket)
                .await
                .starts_with("CONNECT fixture.invalid:443 HTTP/1.1"));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), 22); // TLS Handshake, not plaintext GET.
            socket.write_all(b"invalid-tls-response").await.unwrap();
        });
        assert!(websocket(
            "wss://fixture.invalid/live".into_client_request().unwrap(),
            &network
        )
        .await
        .is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_negotiation_closes_the_proxy_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let network = custom(format!("http://{}", listener.local_addr().unwrap()));
        let (entered, wait_for_entered) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_header(&mut socket).await;
            entered.send(()).unwrap();
            let mut byte = [0; 1];
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(1), socket.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
        let client = tokio::spawn(async move {
            websocket(
                "ws://fixture.invalid/live".into_client_request().unwrap(),
                &network,
            )
            .await
        });
        wait_for_entered.await.unwrap();
        client.abort();
        assert!(client.await.unwrap_err().is_cancelled());
        server.await.unwrap();
    }
}
