//! Synthetic native fixtures shared by lifecycle, snapshot and bridge-contract tests.

use super::probes::text_probe_configuration;
use super::*;
use std::time::Duration;

mod configuration;
mod contracts;
mod lifecycle;
mod snapshots;

fn fixture(configuration: LiveTranslationConfiguration) -> Arc<Session> {
    let direct = ProxyConfig {
        mode: mimi_runtime::core::services::network_proxy::ProxyMode::Direct,
        url: None,
    };
    Arc::new(Session {
        audio_source: AudioSource::System,
        history: Mutex::new(None),
        history_status: Mutex::new(Default::default()),
        configuration,
        speech_network: ProviderNetwork::resolve(&direct).unwrap(),
        text_network: ProviderNetwork::resolve(&direct).unwrap(),
        controller: Mutex::new(Default::default()),
        connection: Mutex::new(None),
        cleanup_owners: Default::default(),
        cleanup_sequence: AtomicU64::new(0),
        task: Mutex::new(None),
        epoch: AtomicU64::new(1),
        version: AtomicU64::new(1),
        started: AtomicBool::new(false),
        stopping: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        signal: Notify::new(),
        error_code: Mutex::new(None),
        budget: Mutex::new(Default::default()),
        recovery_continuity: Mutex::new(Default::default()),
    })
}

fn configuration() -> LiveTranslationConfiguration {
    serde_json::from_value(json!({"provider":"alibabaCloud","qwenMtModel":"lite","credentials":{"kind":"apiKey","apiKey":"synthetic-key"},"textCredentials":null,"sourceLanguage":"en","targetLanguage":"zh","translationMode":"turbo","networkProxy":{"mode":"direct"},"textNetworkProxy":{"mode":"direct"}})).unwrap()
}

struct PollSession {
    handle: u64,
    native: Arc<Session>,
}
impl PollSession {
    fn new() -> Self {
        static HANDLE: AtomicU64 = AtomicU64::new(u64::MAX - 100);
        let handle = HANDLE.fetch_sub(1, Ordering::SeqCst);
        let native = fixture(configuration());
        sessions().lock().unwrap().insert(handle, native.clone());
        Self { handle, native }
    }
    fn poll(&self, since: Option<Value>) -> Result<Value, &'static str> {
        let mut request = json!({"operation":"poll","handle":self.handle});
        if let Some(since) = since {
            request["sinceVersion"] = since;
        }
        exchange(&request.to_string()).map(|response| serde_json::from_str(&response).unwrap())
    }
}
impl Drop for PollSession {
    fn drop(&mut self) {
        sessions().lock().unwrap().remove(&self.handle);
    }
}

// Compatibility oracle for the complete pre-conditional-poll wire shape.
fn legacy_full_response(native: &Session) -> Value {
    let controller = native.controller.lock().unwrap();
    let state = &controller.state;
    let history = *native.history_status.lock().unwrap();
    let connection_status = match state.status {
        SessionStatus::Idle => "idle",
        SessionStatus::Connecting => "connecting",
        SessionStatus::Listening => "listening",
        SessionStatus::Stopping => "stopping",
        SessionStatus::Error(_) => "error",
    };
    json!({"version":native.version.load(Ordering::SeqCst),"historySaveError":history.save_error,"historyLimited":history.limited,"connectionStatus":connection_status,"snapshot":state.subtitles,"atomicPreview":state.atomic_preview,"detectedLanguage":state.detected_language.as_ref().map(|language|language.code.as_str()),"isTranslationPending":state.is_translation_pending,"isTranslationPreviewPending":state.is_translation_preview_pending,"isTranslationTimedOut":state.is_translation_timed_out,"translationRecovery":state.translation_recovery,"finished":native.finished.load(Ordering::SeqCst),"errorCode":*native.error_code.lock().unwrap()})
}
