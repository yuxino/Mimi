//! Bounded private stdio adapter for the bundled, local-only MLX worker.
use super::local_program::LocalWorkerRuntime;
use super::provider_events::ProviderEventSender;
use super::recognition_client::RecognitionClientError;
use crate::core::local_speech::LocalSpeechModel;
use crate::core::models::SourceLanguage;
use crate::core::protocols::custom_speech::{AudioTurnAction, PcmTurnGate};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::local_models::ModelLease;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const MAX_PENDING_PCM_BYTES: usize = 64_000; // Two seconds at 16 kHz PCM16.
struct InputBatch {
    commands: Vec<Value>,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl InputBatch {
    fn control(command: Value) -> Self {
        Self {
            commands: vec![command],
            _permit: None,
        }
    }
}
struct Content {
    gate: PcmTurnGate,
    next_id: u64,
    sender: Option<ProviderEventSender>,
    input: Option<mpsc::Sender<InputBatch>>,
    pending: Arc<tokio::sync::Semaphore>,
}
impl Default for Content {
    fn default() -> Self {
        Self {
            gate: PcmTurnGate::with_sample_rate(16_000),
            next_id: 0,
            sender: None,
            input: None,
            pending: Arc::new(tokio::sync::Semaphore::new(MAX_PENDING_PCM_BYTES)),
        }
    }
}
#[derive(Default)]
struct Lifecycle {
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    lease: Option<ModelLease>,
}
struct Inner {
    runtime: LocalWorkerRuntime,
    source: SourceLanguage,
    content: Mutex<Content>,
    lifecycle: tokio::sync::Mutex<Lifecycle>,
    generation: AtomicU64,
    connected: AtomicBool,
    finishing: AtomicBool,
    ping_sequence: AtomicU64,
    pong: tokio::sync::watch::Sender<u64>,
    cancel: tokio::sync::watch::Sender<u64>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        let life = self.lifecycle.get_mut();
        if let Some(task) = life.reader.take() {
            task.abort();
        }
        if let Some(task) = life.writer.take() {
            task.abort();
        }
        if let Some(child) = life.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}
#[derive(Clone)]
pub struct LocalSpeechClient {
    inner: Arc<Inner>,
}
impl LocalSpeechClient {
    pub fn new(model: LocalSpeechModel, source: SourceLanguage) -> Self {
        Self::with_runtime(LocalWorkerRuntime::Managed(model), source)
    }
    pub fn external(
        config: crate::core::local_program::LocalProgramConfiguration,
        source: SourceLanguage,
    ) -> Self {
        Self::with_runtime(LocalWorkerRuntime::Program(config), source)
    }
    fn with_runtime(runtime: LocalWorkerRuntime, source: SourceLanguage) -> Self {
        Self {
            inner: Arc::new(Inner {
                runtime,
                source,
                content: Mutex::new(Content::default()),
                lifecycle: tokio::sync::Mutex::new(Lifecycle::default()),
                generation: AtomicU64::new(0),
                connected: AtomicBool::new(false),
                finishing: AtomicBool::new(false),
                ping_sequence: AtomicU64::new(0),
                pong: tokio::sync::watch::channel(0).0,
                cancel: tokio::sync::watch::channel(0).0,
            }),
        }
    }
    pub fn set_event_sender(&self, sender: ProviderEventSender) {
        self.inner.content.lock().unwrap().sender = Some(sender);
    }
    pub async fn connect(&self) -> Result<(), RecognitionClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let mut cancel = self.inner.cancel.subscribe();
        let launch = self
            .inner
            .runtime
            .prepare(self.inner.source)
            .map_err(error)?;
        let mut child = super::local_program::command(&launch.executable)
            .args(&launch.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| error("local_model_runtime_failed"))?;
        let mut output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| error("local_model_runtime_failed"))?,
        );
        let ready = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(90), read_event(&mut output)) => result.map_err(|_| error("local_model_setup_timeout"))?,
            _ = cancel.changed() => return Err(error("local_model_not_connected")),
        }.map_err(error)?;
        if ready.get("type").and_then(Value::as_str) != Some("ready") {
            return Err(error("local_model_runtime_failed"));
        }
        let mut lifecycle = self.inner.lifecycle.lock().await;
        if generation != self.inner.generation.load(Ordering::SeqCst) {
            return Err(error("local_model_not_connected"));
        }
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| error("local_model_runtime_failed"))?;
        let (sender, mut commands) = mpsc::channel::<InputBatch>(128);
        {
            let mut content = self.inner.content.lock().unwrap();
            content.gate = PcmTurnGate::with_sample_rate(16_000);
            content.pending = Arc::new(tokio::sync::Semaphore::new(MAX_PENDING_PCM_BYTES));
            content.input = Some(sender);
            if let Some(sender) = &content.sender {
                sender
                    .send(LiveTranslateServerEvent::SessionCreated)
                    .map_err(|_| error("local_model_result_backlog"))?;
            }
        }
        self.inner.finishing.store(false, Ordering::SeqCst);
        self.inner.connected.store(true, Ordering::SeqCst);
        let weak = Arc::downgrade(&self.inner);
        lifecycle.writer = Some(tokio::spawn(async move {
            while let Some(batch) = commands.recv().await {
                for command in &batch.commands {
                    let mut bytes = serde_json::to_vec(&command).unwrap();
                    bytes.push(b'\n');
                    if !matches!(
                        tokio::time::timeout(Duration::from_secs(30), input.write_all(&bytes))
                            .await,
                        Ok(Ok(()))
                    ) {
                        report(&weak, generation, "local_model_audio_backlog");
                        return;
                    }
                }
            }
        }));
        let weak = Arc::downgrade(&self.inner);
        lifecycle.reader = Some(tokio::spawn(async move {
            loop {
                let event = read_event(&mut output).await;
                let Some(inner) = weak.upgrade() else { return };
                if generation != inner.generation.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(event) = event else {
                    report(&weak, generation, "local_model_runtime_failed");
                    return;
                };
                let content = inner.content.lock().unwrap();
                let Some(sender) = &content.sender else {
                    continue;
                };
                let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
                let result = match kind {
                    "draft" | "final" => {
                        let revision = event.get("revision").and_then(Value::as_u64);
                        if revision != Some(sender.content_revision()) {
                            continue;
                        }
                        let Some(id) = event.get("id").and_then(Value::as_u64).filter(|id| *id > 0)
                        else {
                            drop(content);
                            report(&weak, generation, "local_model_runtime_failed");
                            return;
                        };
                        let Some(text) = event
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|s| s.len() <= 32 * 1024)
                        else {
                            drop(content);
                            report(&weak, generation, "local_model_runtime_failed");
                            return;
                        };
                        let language = event
                            .get("language")
                            .and_then(Value::as_str)
                            .filter(|value| inner.runtime.accepts_language(value))
                            .map(str::to_owned);
                        let value = if kind == "draft" {
                            LiveTranslateServerEvent::SourceUtteranceDraft {
                                utterance_id: id,
                                text: text.to_owned(),
                                language,
                            }
                        } else {
                            LiveTranslateServerEvent::SourceUtteranceFinal {
                                utterance_id: id,
                                text: text.to_owned(),
                                language,
                            }
                        };
                        sender.send_if(value, || {
                            generation == inner.generation.load(Ordering::SeqCst)
                                && revision == Some(sender.content_revision())
                        })
                    }
                    "finished" => {
                        inner.connected.store(false, Ordering::SeqCst);
                        let _ = sender.send(LiveTranslateServerEvent::SessionFinished);
                        return;
                    }
                    "pong" => {
                        if let Some(id) = event.get("id").and_then(Value::as_u64) {
                            inner.pong.send_replace(id);
                        }
                        continue;
                    }
                    _ => {
                        drop(content);
                        report(&weak, generation, "local_model_runtime_failed");
                        return;
                    }
                };
                if result.is_err() {
                    inner.connected.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }));
        lifecycle.child = Some(child);
        lifecycle.lease = launch.lease;
        Ok(())
    }
    pub fn send_audio(&self, pcm: &[u8]) -> Result<(), RecognitionClientError> {
        if !self.inner.connected.load(Ordering::SeqCst)
            || self.inner.finishing.load(Ordering::SeqCst)
        {
            return Err(error("local_model_not_connected"));
        }
        let mut content = self.inner.content.lock().unwrap();
        let actions = content
            .gate
            .push(pcm)
            .map_err(|_| error("local_model_audio_failed"))?;
        queue_actions(&mut content, actions)
    }
    pub async fn ping(&self, timeout: Duration) -> Result<(), RecognitionClientError> {
        if !self.inner.connected.load(Ordering::SeqCst) {
            return Err(error("local_model_not_connected"));
        }
        let id = self.inner.ping_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let mut pong = self.inner.pong.subscribe();
        let mut cancel = self.inner.cancel.subscribe();
        {
            let content = self.inner.content.lock().unwrap();
            content
                .input
                .as_ref()
                .ok_or_else(|| error("local_model_not_connected"))?
                .try_send(InputBatch::control(json!({"type":"ping","id":id})))
                .map_err(|_| error("local_model_audio_backlog"))?;
        }
        tokio::select! {
            result = tokio::time::timeout(timeout.max(Duration::from_secs(15)), async {
                loop {
                    if *pong.borrow_and_update() >= id { return Ok(()); }
                    pong.changed().await.map_err(|_| error("local_model_not_connected"))?;
                }
            }) => result.map_err(|_| error("local_model_health_timeout"))?,
            _ = cancel.changed() => Err(error("local_model_not_connected")),
        }
    }
    pub async fn clear_content(&self) -> u64 {
        let mut content = self.inner.content.lock().unwrap();
        let revision = content
            .sender
            .as_ref()
            .map_or(0, ProviderEventSender::advance_content_revision);
        content.gate = PcmTurnGate::with_sample_rate(16_000);
        if let Some(input) = &content.input {
            if input
                .try_send(InputBatch::control(json!({"type":"clear"})))
                .is_err()
            {
                self.inner.connected.store(false, Ordering::SeqCst);
                if let Some(sender) = &content.sender {
                    let _ = sender.send(error_event("local_model_audio_backlog"));
                }
            }
        }
        revision
    }
    pub fn content_revision(&self) -> u64 {
        self.inner
            .content
            .lock()
            .unwrap()
            .sender
            .as_ref()
            .map_or(0, ProviderEventSender::content_revision)
    }
    pub async fn finish(&self, timeout: Duration) {
        self.inner.finishing.store(true, Ordering::SeqCst);
        self.inner.cancel.send_modify(|v| *v = v.wrapping_add(1));
        {
            let mut content = self.inner.content.lock().unwrap();
            let actions = content.gate.finish();
            let result = queue_actions(&mut content, actions).and_then(|()| {
                content
                    .input
                    .as_ref()
                    .ok_or_else(|| error("local_model_not_connected"))?
                    .try_send(InputBatch::control(json!({"type":"finish"})))
                    .map_err(|_| error("local_model_audio_backlog"))
            });
            if result.is_err() {
                if let Some(sender) = &content.sender {
                    let _ = sender.send(error_event("local_model_finalize_timeout"));
                }
            }
        }
        let mut lifecycle = self.inner.lifecycle.lock().await;
        if let Some(mut reader) = lifecycle.reader.take() {
            if tokio::time::timeout(timeout, &mut reader).await.is_err() {
                reader.abort();
                let _ = reader.await;
                if let Some(sender) = &self.inner.content.lock().unwrap().sender {
                    let _ = sender.send(error_event("local_model_finalize_timeout"));
                }
            }
        }
        drop(lifecycle);
        self.disconnect().await;
    }
    pub async fn disconnect(&self) {
        self.inner.cancel.send_modify(|v| *v = v.wrapping_add(1));
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.connected.store(false, Ordering::SeqCst);
        self.inner.content.lock().unwrap().input = None;
        let mut life = self.inner.lifecycle.lock().await;
        if let Some(task) = life.reader.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = life.writer.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(mut child) = life.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        life.lease = None;
    }
}
fn queue_actions(
    content: &mut Content,
    actions: Vec<AudioTurnAction>,
) -> Result<(), RecognitionClientError> {
    if actions.is_empty() {
        return Ok(());
    }
    let pcm_bytes: usize = actions
        .iter()
        .map(|action| match action {
            AudioTurnAction::Append(pcm) => pcm.len(),
            _ => 0,
        })
        .sum();
    let permit = content
        .pending
        .clone()
        .try_acquire_many_owned(
            u32::try_from(pcm_bytes).map_err(|_| error("local_model_audio_backlog"))?,
        )
        .map_err(|_| error("local_model_audio_backlog"))?;
    let revision = content
        .sender
        .as_ref()
        .map_or(0, ProviderEventSender::content_revision);
    let batch = actions
        .into_iter()
        .map(|action| match action {
            AudioTurnAction::Start => {
                content.next_id = content.next_id.saturating_add(1);
                json!({"type":"start", "id":content.next_id,"revision":revision})
            }
            AudioTurnAction::Append(pcm) => json!({"type":"audio","audio":STANDARD.encode(pcm)}),
            AudioTurnAction::Commit => json!({"type":"commit"}),
        })
        .collect();
    content
        .input
        .as_ref()
        .ok_or_else(|| error("local_model_not_connected"))?
        .try_send(InputBatch {
            commands: batch,
            _permit: Some(permit),
        })
        .map_err(|_| error("local_model_audio_backlog"))
}
async fn read_event<R: tokio::io::AsyncBufRead + Unpin>(
    output: &mut R,
) -> Result<Value, &'static str> {
    let mut bytes = Vec::new();
    loop {
        let chunk = output
            .fill_buf()
            .await
            .map_err(|_| "local_model_runtime_failed")?;
        if chunk.is_empty() {
            return Err("local_model_runtime_failed");
        }
        let end = chunk.iter().position(|b| *b == b'\n');
        let size = end.map_or(chunk.len(), |p| p + 1);
        if bytes.len() + size > 64 * 1024 {
            return Err("local_model_runtime_failed");
        }
        bytes.extend_from_slice(&chunk[..size]);
        output.consume(size);
        if end.is_some() {
            return serde_json::from_slice(&bytes).map_err(|_| "local_model_runtime_failed");
        }
    }
}
fn error(label: &str) -> RecognitionClientError {
    RecognitionClientError::Local(label.to_owned())
}
fn error_event(label: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::Error {
        code: label.to_owned(),
        message: label.to_owned(),
    }
}
fn report(weak: &Weak<Inner>, generation: u64, label: &str) {
    if let Some(inner) = weak.upgrade() {
        if generation == inner.generation.load(Ordering::SeqCst) {
            inner.connected.store(false, Ordering::SeqCst);
            if let Some(sender) = &inner.content.lock().unwrap().sender {
                let _ = sender.send(error_event(label));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_audio_has_a_two_second_budget_and_releases_it_when_consumed() {
        let (sender, mut receiver) = mpsc::channel(128);
        let mut content = Content {
            input: Some(sender),
            ..Content::default()
        };
        let audio = || vec![AudioTurnAction::Append(vec![0; MAX_PENDING_PCM_BYTES])];
        assert!(queue_actions(&mut content, audio()).is_ok());
        assert!(queue_actions(&mut content, vec![AudioTurnAction::Append(vec![0; 2])]).is_err());
        drop(receiver.try_recv().unwrap());
        assert!(queue_actions(&mut content, audio()).is_ok());
    }
    #[tokio::test]
    async fn worker_output_is_bounded_and_rejects_partial_or_invalid_messages() {
        for bytes in [
            vec![b'x'; 65 * 1024],
            b"{\"type\":\"ready\"}".to_vec(),
            b"not-json\n".to_vec(),
        ] {
            assert!(read_event(&mut bytes.as_slice()).await.is_err());
        }
        let bytes = b"{\"type\":\"ready\"}\n";
        assert_eq!(
            read_event(&mut bytes.as_slice()).await.unwrap()["type"],
            "ready"
        );
    }
}
