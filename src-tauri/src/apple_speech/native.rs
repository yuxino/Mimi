use super::{AppleSpeechCapabilities, AppleSpeechError, AppleSpeechEvent, AppleSpeechLocale};
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

const EVENT_CAPACITY: usize = 64;
const MAX_NATIVE_BYTES: usize = 262_144;
const MAX_PCM_BYTES: usize = 32_000;
const START_TIMEOUT: Duration = Duration::from_secs(15);
const FINISH_TIMEOUT: Duration = Duration::from_secs(3);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const PREPARE_TIMEOUT: Duration = Duration::from_secs(600);

type Callback = unsafe extern "C" fn(u64, *const u8, usize) -> i32;
unsafe extern "C" {
    fn mimi_apple_speech_query(id: u64, callback: Callback);
    fn mimi_apple_speech_prepare(id: u64, locale: *const c_char, callback: Callback);
    fn mimi_apple_speech_start(id: u64, locale: *const c_char, callback: Callback);
    fn mimi_apple_speech_push(id: u64, pcm: *const u8, count: usize) -> i32;
    fn mimi_apple_speech_finish(id: u64);
    fn mimi_apple_speech_cancel(id: u64);
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum NativeEvent {
    Capabilities {
        available: bool,
        locales: Vec<AppleSpeechLocale>,
    },
    Prepared,
    Ready,
    Result {
        text: String,
        start_ms: f64,
        end_ms: f64,
        #[serde(rename = "final")]
        is_final: bool,
    },
    Done,
    Error {
        domain: String,
        code: i64,
    },
}

enum Response {
    Capabilities(AppleSpeechCapabilities),
    Prepared,
}

#[derive(Clone)]
enum Status {
    Starting,
    Ready,
    Done,
    Cancelled,
    Failed(AppleSpeechError),
}

enum Route {
    Request(Mutex<Option<oneshot::Sender<Result<Response, AppleSpeechError>>>>),
    Session {
        events: mpsc::Sender<AppleSpeechEvent>,
        status: watch::Sender<Status>,
        delivery: Mutex<()>,
    },
}

fn routes() -> &'static Mutex<HashMap<u64, Arc<Route>>> {
    static ROUTES: OnceLock<Mutex<HashMap<u64, Arc<Route>>>> = OnceLock::new();
    ROUTES.get_or_init(Mutex::default)
}

fn register(route: Route) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    routes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, Arc::new(route));
    id
}

fn remove(id: u64) -> Option<Arc<Route>> {
    routes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id)
}

fn cancel(id: u64) {
    // Remove routing before asking Swift to stop. A late callback has only an ID
    // and cannot access a freed Rust context or publish a stale generation.
    if let Some(route) = remove(id) {
        if let Route::Session {
            status, delivery, ..
        } = route.as_ref()
        {
            let _guard = delivery.lock().unwrap_or_else(|e| e.into_inner());
            status.send_replace(Status::Cancelled);
        }
    }
    unsafe { mimi_apple_speech_cancel(id) };
}

struct RequestGuard(u64);
impl Drop for RequestGuard {
    fn drop(&mut self) {
        cancel(self.0);
    }
}

fn native_error(domain: String, code: i64) -> AppleSpeechError {
    if domain == "MimiAppleSpeech" {
        return match code {
            1 => AppleSpeechError::Unavailable,
            2 => AppleSpeechError::AssetsNotInstalled,
            3 => AppleSpeechError::InvalidLocale,
            4 => AppleSpeechError::IncompatibleFormat,
            5 => AppleSpeechError::InvalidPcm,
            6 => AppleSpeechError::QueueOverflow,
            7 => AppleSpeechError::Closed,
            10 => AppleSpeechError::ReservationLimit,
            11 => AppleSpeechError::ResourcesUnavailable,
            12 => AppleSpeechError::ServiceUnavailable,
            13 => AppleSpeechError::DownloadCancelled,
            14 => AppleSpeechError::DownloadNetwork,
            15 => AppleSpeechError::DownloadStorage,
            16 => AppleSpeechError::StatusUnavailable,
            17 => AppleSpeechError::Timeout,
            _ => AppleSpeechError::InvalidResult,
        };
    }
    let domain = match domain.as_str() {
        "SFSpeechErrorDomain"
        | "NSOSStatusErrorDomain"
        | "NSCocoaErrorDomain"
        | "NSPOSIXErrorDomain"
        | "NSURLErrorDomain" => domain,
        _ => "SpeechFrameworkError".to_owned(),
    };
    AppleSpeechError::Native { domain, code }
}

fn dispatch(route: &Route, event: Result<NativeEvent, AppleSpeechError>) -> bool {
    match route {
        Route::Request(sender) => {
            let result = match event {
                Ok(NativeEvent::Capabilities {
                    available,
                    mut locales,
                }) if locales.len() <= 256 => {
                    if locales
                        .iter()
                        .any(|locale| valid_locale(&locale.identifier).is_err())
                    {
                        Err(AppleSpeechError::InvalidResult)
                    } else {
                        locales.sort_by(|a, b| a.identifier.cmp(&b.identifier));
                        locales.dedup_by(|a, b| a.identifier == b.identifier);
                        Ok(Response::Capabilities(AppleSpeechCapabilities {
                            available,
                            locales,
                        }))
                    }
                }
                Ok(NativeEvent::Prepared) => Ok(Response::Prepared),
                Ok(NativeEvent::Error { domain, code }) => Err(native_error(domain, code)),
                Err(error) => Err(error),
                _ => Err(AppleSpeechError::InvalidResult),
            };
            sender
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .is_some_and(|sender| sender.send(result).is_ok())
        }
        Route::Session {
            events,
            status,
            delivery,
        } => {
            let _guard = delivery.lock().unwrap_or_else(|e| e.into_inner());
            // A callback may have copied the Arc immediately before cancel
            // removed the ID. Serialize this final gate with cancellation so
            // it cannot restore Ready/Done and expose old queued results.
            if matches!(
                *status.borrow(),
                Status::Cancelled | Status::Done | Status::Failed(_)
            ) {
                return false;
            }
            let result = match event {
                Ok(NativeEvent::Ready) => {
                    status.send_replace(Status::Ready);
                    return true;
                }
                Ok(NativeEvent::Done) => {
                    status.send_replace(Status::Done);
                    return true;
                }
                Ok(NativeEvent::Result {
                    text,
                    start_ms,
                    end_ms,
                    is_final,
                }) if text.len() <= 65_536
                    && start_ms.is_finite()
                    && end_ms.is_finite()
                    && start_ms >= 0.0
                    && end_ms >= start_ms =>
                {
                    let value = AppleSpeechEvent {
                        text,
                        start_ms,
                        end_ms,
                        is_final,
                    };
                    match events.try_send(value) {
                        Ok(()) => return true,
                        Err(mpsc::error::TrySendError::Full(_)) => AppleSpeechError::QueueOverflow,
                        Err(mpsc::error::TrySendError::Closed(_)) => AppleSpeechError::Closed,
                    }
                }
                Ok(NativeEvent::Error { domain, code }) => native_error(domain, code),
                Err(error) => error,
                _ => AppleSpeechError::InvalidResult,
            };
            // Status is an independent bounded channel, so a full result queue
            // cannot swallow the overflow error or turn dropped finals into success.
            status.send_replace(Status::Failed(result));
            false
        }
    }
}

unsafe extern "C" fn callback(id: u64, data: *const u8, length: usize) -> i32 {
    // Never allow a Rust unwind to cross Swift/C. No native text is logged.
    std::panic::catch_unwind(|| {
        let route = routes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        let Some(route) = route else {
            return 0;
        };
        let event = if data.is_null() || length == 0 || length > MAX_NATIVE_BYTES {
            Err(AppleSpeechError::InvalidResult)
        } else {
            // Swift borrows this buffer only for the synchronous callback.
            serde_json::from_slice(unsafe { std::slice::from_raw_parts(data, length) })
                .map_err(|_| AppleSpeechError::InvalidResult)
        };
        let terminal = matches!(
            &event,
            Ok(NativeEvent::Done
                | NativeEvent::Error { .. }
                | NativeEvent::Prepared
                | NativeEvent::Capabilities { .. })
        ) || event.is_err();
        let accepted = dispatch(&route, event);
        if terminal || !accepted {
            remove(id);
        }
        i32::from(accepted)
    })
    .unwrap_or(0)
}

fn valid_locale(locale: &str) -> Result<CString, AppleSpeechError> {
    if locale.is_empty()
        || locale.len() > 64
        || !locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(AppleSpeechError::InvalidLocale);
    }
    CString::new(locale).map_err(|_| AppleSpeechError::InvalidLocale)
}

pub async fn capabilities() -> Result<AppleSpeechCapabilities, AppleSpeechError> {
    let (tx, rx) = oneshot::channel();
    let guard = RequestGuard(register(Route::Request(Mutex::new(Some(tx)))));
    unsafe { mimi_apple_speech_query(guard.0, callback) };
    match tokio::time::timeout(QUERY_TIMEOUT, rx)
        .await
        .map_err(|_| AppleSpeechError::Timeout)?
        .map_err(|_| AppleSpeechError::Closed)??
    {
        Response::Capabilities(value) => Ok(value),
        _ => Err(AppleSpeechError::InvalidResult),
    }
}

pub async fn prepare(locale: &str) -> Result<(), AppleSpeechError> {
    let locale = valid_locale(locale)?;
    let (tx, rx) = oneshot::channel();
    let guard = RequestGuard(register(Route::Request(Mutex::new(Some(tx)))));
    unsafe { mimi_apple_speech_prepare(guard.0, locale.as_ptr(), callback) };
    match tokio::time::timeout(PREPARE_TIMEOUT, rx)
        .await
        .map_err(|_| AppleSpeechError::Timeout)?
        .map_err(|_| AppleSpeechError::Closed)??
    {
        Response::Prepared => Ok(()),
        _ => Err(AppleSpeechError::InvalidResult),
    }
}

pub struct AppleSpeechSession {
    id: u64,
    status: watch::Receiver<Status>,
    active: bool,
}

pub struct AppleSpeechEvents {
    events: mpsc::Receiver<AppleSpeechEvent>,
    status: watch::Receiver<Status>,
}

pub async fn start(
    locale: &str,
) -> Result<(AppleSpeechSession, AppleSpeechEvents), AppleSpeechError> {
    let locale = valid_locale(locale)?;
    let (events_tx, events) = mpsc::channel(EVENT_CAPACITY);
    let (status_tx, status) = watch::channel(Status::Starting);
    let id = register(Route::Session {
        events: events_tx,
        status: status_tx,
        delivery: Mutex::new(()),
    });
    let mut session = AppleSpeechSession {
        id,
        status: status.clone(),
        active: true,
    };
    unsafe { mimi_apple_speech_start(id, locale.as_ptr(), callback) };
    tokio::time::timeout(START_TIMEOUT, session.wait_ready())
        .await
        .map_err(|_| AppleSpeechError::Timeout)??;
    Ok((session, AppleSpeechEvents { events, status }))
}

impl AppleSpeechSession {
    async fn wait_ready(&mut self) -> Result<(), AppleSpeechError> {
        loop {
            let state = self.status.borrow().clone();
            match state {
                Status::Ready => return Ok(()),
                Status::Failed(error) => return Err(error),
                Status::Done | Status::Cancelled => return Err(AppleSpeechError::Closed),
                Status::Starting => {}
            }
            self.status
                .changed()
                .await
                .map_err(|_| AppleSpeechError::Closed)?;
        }
    }

    pub fn send_pcm(&self, pcm: &[u8]) -> Result<(), AppleSpeechError> {
        if !self.active {
            return Err(AppleSpeechError::Closed);
        }
        if let Status::Failed(error) = self.status.borrow().clone() {
            return Err(error);
        }
        if pcm.is_empty() {
            return Ok(());
        }
        if pcm.len() > MAX_PCM_BYTES || !pcm.len().is_multiple_of(2) {
            return Err(AppleSpeechError::InvalidPcm);
        }
        match unsafe { mimi_apple_speech_push(self.id, pcm.as_ptr(), pcm.len()) } {
            0 => Ok(()),
            1 => Err(AppleSpeechError::InvalidPcm),
            2 => Err(AppleSpeechError::QueueOverflow),
            _ => Err(AppleSpeechError::Closed),
        }
    }

    pub async fn finish(&mut self) -> Result<(), AppleSpeechError> {
        if !self.active {
            return Ok(());
        }
        unsafe { mimi_apple_speech_finish(self.id) };
        let result = tokio::time::timeout(FINISH_TIMEOUT, async {
            loop {
                let state = self.status.borrow().clone();
                match state {
                    Status::Done => return Ok(()),
                    Status::Failed(error) => return Err(error),
                    Status::Cancelled => return Err(AppleSpeechError::Closed),
                    Status::Starting | Status::Ready => {}
                }
                self.status
                    .changed()
                    .await
                    .map_err(|_| AppleSpeechError::Closed)?;
            }
        })
        .await
        .map_err(|_| AppleSpeechError::Timeout)
        .and_then(|result| result);
        if result.is_ok() {
            self.active = false;
            remove(self.id);
        } else {
            self.cancel();
        }
        result
    }

    pub fn cancel(&mut self) {
        if self.active {
            self.active = false;
            cancel(self.id);
        }
    }
}

impl Drop for AppleSpeechSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl AppleSpeechEvents {
    pub async fn recv(&mut self) -> Result<Option<AppleSpeechEvent>, AppleSpeechError> {
        loop {
            let state = self.status.borrow().clone();
            match state {
                Status::Failed(error) => return Err(error),
                Status::Cancelled => return Ok(None),
                Status::Done => return Ok(self.events.try_recv().ok()),
                Status::Starting | Status::Ready => {}
            }
            tokio::select! {
                biased;
                changed = self.status.changed() => {
                    if changed.is_err() {
                        let state = self.status.borrow().clone();
                        return match state {
                            Status::Failed(error) => Err(error),
                            Status::Done => Ok(self.events.try_recv().ok()),
                            Status::Cancelled => Ok(None),
                            _ => Err(AppleSpeechError::Closed),
                        };
                    }
                }
                event = self.events.recv() => return Ok(event),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_route(
        capacity: usize,
    ) -> (
        Route,
        mpsc::Receiver<AppleSpeechEvent>,
        watch::Receiver<Status>,
    ) {
        let (events, receiver) = mpsc::channel(capacity);
        let (status, state) = watch::channel(Status::Ready);
        (
            Route::Session {
                events,
                status,
                delivery: Mutex::new(()),
            },
            receiver,
            state,
        )
    }
    fn result(is_final: bool) -> NativeEvent {
        NativeEvent::Result {
            text: "fixture".into(),
            start_ms: 0.0,
            end_ms: 100.0,
            is_final,
        }
    }

    #[tokio::test]
    async fn full_results_queue_reports_failure_outside_that_queue() {
        let (route, events, status) = session_route(1);
        assert!(dispatch(&route, Ok(result(false))));
        assert!(!dispatch(&route, Ok(result(true))));
        let mut stream = AppleSpeechEvents { events, status };
        assert!(matches!(
            stream.recv().await,
            Err(AppleSpeechError::QueueOverflow)
        ));
    }

    #[tokio::test]
    async fn done_drains_tail_final_before_end_of_stream() {
        let (route, events, status) = session_route(2);
        assert!(dispatch(&route, Ok(result(true))));
        assert!(dispatch(&route, Ok(NativeEvent::Done)));
        let mut stream = AppleSpeechEvents { events, status };
        assert!(stream.recv().await.unwrap().unwrap().is_final);
        assert!(stream.recv().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancellation_discards_queued_old_generation_results() {
        let (route, events, status) = session_route(2);
        assert!(dispatch(&route, Ok(result(true))));
        if let Route::Session { status, .. } = &route {
            status.send_replace(Status::Cancelled);
        }
        assert!(!dispatch(&route, Ok(NativeEvent::Done)));
        assert!(!dispatch(&route, Ok(NativeEvent::Ready)));
        assert!(!dispatch(&route, Ok(result(true))));
        let mut stream = AppleSpeechEvents { events, status };
        assert!(stream.recv().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn callback_copies_borrowed_bytes_and_drop_rejects_late_delivery() {
        let (route, events, status) = session_route(2);
        let id = register(route);
        let session = AppleSpeechSession {
            id,
            status: status.clone(),
            active: true,
        };
        let mut bytes =
            br#"{"event":"result","text":"fixture","start_ms":0,"end_ms":100,"final":true}"#
                .to_vec();
        assert_eq!(unsafe { callback(id, bytes.as_ptr(), bytes.len()) }, 1);
        bytes.fill(0);
        let mut stream = AppleSpeechEvents { events, status };
        assert_eq!(stream.recv().await.unwrap().unwrap().text, "fixture");
        // No native session was started for this ID. Drop still closes routing
        // before the native cancellation call, so no stale pointer is retained.
        drop(session);
        assert_eq!(unsafe { callback(id, bytes.as_ptr(), bytes.len()) }, 0);
        assert!(stream.recv().await.unwrap().is_none());
    }

    #[test]
    fn adapter_handles_are_send_and_session_is_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<AppleSpeechSession>();
        assert_sync::<AppleSpeechSession>();
        assert_send::<AppleSpeechEvents>();
        assert_send::<AppleSpeechError>();
        assert_sync::<AppleSpeechError>();
    }

    #[test]
    fn malformed_ranges_and_private_error_domains_are_sanitized() {
        let (route, _, status) = session_route(1);
        assert!(!dispatch(
            &route,
            Ok(NativeEvent::Result {
                text: "fixture".into(),
                start_ms: 5.0,
                end_ms: 1.0,
                is_final: true
            })
        ));
        assert!(matches!(
            *status.borrow(),
            Status::Failed(AppleSpeechError::InvalidResult)
        ));
        assert_eq!(
            native_error("private/path/content".into(), 4).to_string(),
            "Apple Speech failed (SpeechFrameworkError, 4)"
        );
        assert!(valid_locale("en-US\0private").is_err());
    }
}
