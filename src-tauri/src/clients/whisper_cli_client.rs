//! Sentence-based whisper.cpp adapter with bounded private temporary audio.
use super::local_program::{command, validate_files};
use super::provider_events::ProviderEventSender;
use super::recognition_client::RecognitionClientError;
use crate::core::local_program::LocalProgramConfiguration;
use crate::core::models::SourceLanguage;
use crate::core::protocols::custom_speech::{AudioTurnAction, PcmTurnGate};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

const MAX_TURN_BYTES: usize = 16_000 * 2 * 8;
const MAX_TEXT_BYTES: usize = 32 * 1024;
const INFERENCE_TIMEOUT: Duration = Duration::from_secs(45);

struct Turn {
    id: u64,
    revision: u64,
    pcm: Vec<u8>,
}
struct Content {
    gate: PcmTurnGate,
    pcm: Vec<u8>,
    next_id: u64,
    events: Option<ProviderEventSender>,
    input: Option<mpsc::Sender<Turn>>,
}
impl Default for Content {
    fn default() -> Self {
        Self {
            gate: PcmTurnGate::with_sample_rate(16_000),
            pcm: vec![],
            next_id: 0,
            events: None,
            input: None,
        }
    }
}
struct Inner {
    config: LocalProgramConfiguration,
    source: SourceLanguage,
    content: Mutex<Content>,
    worker: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    generation: AtomicU64,
    connected: AtomicBool,
    cancel: watch::Sender<u64>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.get_mut().take() {
            worker.abort();
        }
    }
}
#[derive(Clone)]
pub struct WhisperCliClient {
    inner: Arc<Inner>,
}
impl WhisperCliClient {
    pub fn new(config: LocalProgramConfiguration, source: SourceLanguage) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                source,
                content: Mutex::new(Content::default()),
                worker: tokio::sync::Mutex::new(None),
                generation: AtomicU64::new(0),
                connected: AtomicBool::new(false),
                cancel: watch::channel(0).0,
            }),
        }
    }
    pub fn set_event_sender(&self, sender: ProviderEventSender) {
        self.inner.content.lock().unwrap().events = Some(sender);
    }
    pub async fn connect(&self) -> Result<(), RecognitionClientError> {
        self.disconnect().await;
        validate_files(&self.inner.config).map_err(error)?;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let mut cancel = self.inner.cancel.subscribe();
        // A real silent inference verifies that both program and model load.
        run(
            &self.inner.config,
            self.inner.source,
            &[0; 32_000],
            &mut cancel,
        )
        .await
        .map_err(error)?;
        if generation != self.inner.generation.load(Ordering::SeqCst) {
            return Err(error("local_model_not_connected"));
        }
        let (input, mut turns) = mpsc::channel::<Turn>(1);
        {
            let mut content = self.inner.content.lock().unwrap();
            content.gate = PcmTurnGate::with_sample_rate(16_000);
            content.pcm.clear();
            content.input = Some(input);
            if let Some(events) = &content.events {
                events
                    .send(LiveTranslateServerEvent::SessionCreated)
                    .map_err(|_| error("local_model_result_backlog"))?;
            }
        }
        self.inner.connected.store(true, Ordering::SeqCst);
        let weak = Arc::downgrade(&self.inner);
        let mut cancel = self.inner.cancel.subscribe();
        let config = self.inner.config.clone();
        let source = self.inner.source;
        let worker = tokio::spawn(async move {
            loop {
                let turn = tokio::select! {
                    turn = turns.recv() => match turn { Some(turn) => turn, None => break },
                    _ = cancel.changed() => {
                        if weak.upgrade().is_none_or(|inner| generation != inner.generation.load(Ordering::SeqCst)) { return; }
                        continue;
                    },
                };
                let Some(inner) = weak.upgrade() else { return };
                let events = inner.content.lock().unwrap().events.clone();
                let Some(events) = events else { continue };
                if generation != inner.generation.load(Ordering::SeqCst) {
                    return;
                }
                if turn.revision != events.content_revision() {
                    continue;
                }
                drop(inner);
                let result = run(&config, source, &turn.pcm, &mut cancel).await;
                if result == Err("local_program_cancelled") {
                    continue;
                }
                let Some(inner) = weak.upgrade() else { return };
                if generation != inner.generation.load(Ordering::SeqCst) {
                    return;
                }
                match result {
                    Ok(text) if !text.is_empty() => {
                        let _ = events.send_if(
                            LiveTranslateServerEvent::SourceUtteranceFinal {
                                utterance_id: turn.id,
                                text,
                                language: (source != SourceLanguage::Automatic)
                                    .then(|| source.raw_value().to_owned()),
                            },
                            || {
                                generation == inner.generation.load(Ordering::SeqCst)
                                    && turn.revision == events.content_revision()
                            },
                        );
                    }
                    Ok(_) => {}
                    Err(label) => {
                        if turn.revision != events.content_revision() {
                            continue;
                        }
                        inner.connected.store(false, Ordering::SeqCst);
                        let _ = events.send(failure(label));
                        return;
                    }
                }
            }
            if let Some(inner) = weak.upgrade() {
                if generation == inner.generation.load(Ordering::SeqCst) {
                    inner.connected.store(false, Ordering::SeqCst);
                    if let Some(events) = &inner.content.lock().unwrap().events {
                        let _ = events.send(LiveTranslateServerEvent::SessionFinished);
                    }
                }
            }
        });
        *self.inner.worker.lock().await = Some(worker);
        Ok(())
    }
    pub fn send_audio(&self, pcm: &[u8]) -> Result<(), RecognitionClientError> {
        if !self.inner.connected.load(Ordering::SeqCst) {
            return Err(error("local_model_not_connected"));
        }
        let mut content = self.inner.content.lock().unwrap();
        let actions = content
            .gate
            .push(pcm)
            .map_err(|_| error("local_model_audio_invalid"))?;
        queue(&mut content, actions).map_err(error)
    }
    pub async fn ping(&self) -> Result<(), RecognitionClientError> {
        if self.inner.connected.load(Ordering::SeqCst)
            && self
                .inner
                .worker
                .lock()
                .await
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
        {
            Ok(())
        } else {
            Err(error("local_model_not_connected"))
        }
    }
    pub async fn clear_content(&self) -> u64 {
        self.inner
            .cancel
            .send_modify(|value| *value = value.wrapping_add(1));
        let mut content = self.inner.content.lock().unwrap();
        content.gate = PcmTurnGate::with_sample_rate(16_000);
        content.pcm.clear();
        content
            .events
            .as_ref()
            .map_or(0, ProviderEventSender::advance_content_revision)
    }
    pub fn content_revision(&self) -> u64 {
        self.inner
            .content
            .lock()
            .unwrap()
            .events
            .as_ref()
            .map_or(0, ProviderEventSender::content_revision)
    }
    pub async fn finish(&self, timeout: Duration) {
        {
            let mut content = self.inner.content.lock().unwrap();
            let actions = content.gate.finish();
            if queue(&mut content, actions).is_err() {
                if let Some(events) = &content.events {
                    let _ = events.send(failure("local_model_audio_backlog"));
                }
            }
            content.input = None;
        }
        if let Some(mut worker) = self.inner.worker.lock().await.take() {
            if tokio::time::timeout(timeout, &mut worker).await.is_err() {
                self.inner
                    .cancel
                    .send_modify(|value| *value = value.wrapping_add(1));
                if tokio::time::timeout(Duration::from_secs(2), &mut worker)
                    .await
                    .is_err()
                {
                    worker.abort();
                    let _ = worker.await;
                }
            }
        }
        self.disconnect().await;
    }
    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.connected.store(false, Ordering::SeqCst);
        self.inner
            .cancel
            .send_modify(|value| *value = value.wrapping_add(1));
        {
            let mut content = self.inner.content.lock().unwrap();
            content.input = None;
            content.pcm.clear();
        }
        if let Some(mut worker) = self.inner.worker.lock().await.take() {
            // Cancellation lets the owner kill and wait for its child before aborting.
            if tokio::time::timeout(Duration::from_secs(2), &mut worker)
                .await
                .is_err()
            {
                worker.abort();
                let _ = worker.await;
            }
        }
    }
}

fn queue(content: &mut Content, actions: Vec<AudioTurnAction>) -> Result<(), &'static str> {
    for action in actions {
        match action {
            AudioTurnAction::Start => {
                content.pcm.clear();
                content.next_id = content.next_id.wrapping_add(1).max(1);
            }
            AudioTurnAction::Append(pcm) => {
                if content.pcm.len() + pcm.len() > MAX_TURN_BYTES {
                    return Err("local_model_audio_backlog");
                }
                content.pcm.extend_from_slice(&pcm);
            }
            AudioTurnAction::Commit => {
                let pcm = std::mem::take(&mut content.pcm);
                if pcm.is_empty() {
                    continue;
                }
                let revision = content
                    .events
                    .as_ref()
                    .map_or(0, ProviderEventSender::content_revision);
                content
                    .input
                    .as_ref()
                    .ok_or("local_model_not_connected")?
                    .try_send(Turn {
                        id: content.next_id,
                        revision,
                        pcm,
                    })
                    .map_err(|_| "local_model_audio_backlog")?;
            }
        }
    }
    Ok(())
}

async fn run(
    config: &LocalProgramConfiguration,
    source: SourceLanguage,
    pcm: &[u8],
    cancel: &mut watch::Receiver<u64>,
) -> Result<String, &'static str> {
    if !config.accepts_language(source.raw_value()) {
        return Err("local_program_language_unsupported");
    }
    if pcm.len() > MAX_TURN_BYTES || !pcm.len().is_multiple_of(2) {
        return Err("local_model_audio_invalid");
    }
    let mut audio = tempfile::Builder::new()
        .prefix("mimi-speech-")
        .suffix(".wav")
        .tempfile()
        .map_err(|_| "local_program_temporary_audio_failed")?;
    audio
        .write_all(&wave(pcm))
        .and_then(|()| audio.flush())
        .map_err(|_| "local_program_temporary_audio_failed")?;
    let mut child = command(Path::new(&config.executable))
        .args(&config.arguments)
        .arg("-m")
        .arg(&config.model_path)
        .arg("-f")
        .arg(audio.path())
        .arg("-l")
        .arg(source.raw_value())
        .args(["-nt", "-np"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "local_program_start_failed")?;
    let output = child.stdout.take().ok_or("local_program_start_failed")?;
    let operation = async {
        let mut bytes = vec![];
        output
            .take((MAX_TEXT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "local_program_output_invalid")?;
        if bytes.len() > MAX_TEXT_BYTES {
            return Err("local_program_output_invalid");
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "local_program_start_failed")?;
        if !status.success() {
            return Err("local_program_inference_failed");
        }
        let text = String::from_utf8(bytes).map_err(|_| "local_program_output_invalid")?;
        Ok(text.trim().to_owned())
    };
    let result = tokio::select! {
        result = tokio::time::timeout(INFERENCE_TIMEOUT, operation) => result.unwrap_or(Err("local_program_inference_timeout")),
        _ = cancel.changed() => Err("local_program_cancelled"),
    };
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

fn wave(pcm: &[u8]) -> Vec<u8> {
    let length = pcm.len() as u32;
    let mut data = Vec::with_capacity(pcm.len() + 44);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(length + 36).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16_u32.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&16_000_u32.to_le_bytes());
    data.extend_from_slice(&32_000_u32.to_le_bytes());
    data.extend_from_slice(&2_u16.to_le_bytes());
    data.extend_from_slice(&16_u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&length.to_le_bytes());
    data.extend_from_slice(pcm);
    data
}
fn error(label: &str) -> RecognitionClientError {
    RecognitionClientError::Local(label.into())
}
fn failure(label: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::Error {
        code: label.into(),
        message: label.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_is_exactly_one_bounded_pcm16_turn() {
        let pcm = [1, 2, 3, 4];
        let encoded = wave(&pcm);
        assert_eq!(&encoded[0..4], b"RIFF");
        assert_eq!(&encoded[8..12], b"WAVE");
        assert_eq!(&encoded[44..], &pcm);
        assert_eq!(
            u32::from_le_bytes(encoded[24..28].try_into().unwrap()),
            16_000
        );
        assert_eq!(
            u32::from_le_bytes(encoded[40..44].try_into().unwrap()),
            pcm.len() as u32
        );
    }
    #[test]
    fn stalled_inference_cannot_accumulate_unbounded_turns() {
        let (input, _receiver) = mpsc::channel(1);
        let mut content = Content {
            input: Some(input),
            ..Content::default()
        };
        for expected in [Ok(()), Err("local_model_audio_backlog")] {
            assert_eq!(
                queue(
                    &mut content,
                    vec![
                        AudioTurnAction::Start,
                        AudioTurnAction::Append(vec![0; 320]),
                        AudioTurnAction::Commit
                    ]
                ),
                expected
            );
        }
        assert!(content.pcm.is_empty());
    }
    #[cfg(unix)]
    fn fixture(body: &str) -> (tempfile::TempDir, LocalProgramConfiguration) {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("program with spaces");
        let model = root.path().join("model.bin");
        std::fs::write(&model, b"user owned model").unwrap();
        std::fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = LocalProgramConfiguration {
            engine: crate::core::local_program::LocalProgramEngine::WhisperCpp,
            executable: executable.to_string_lossy().into(),
            model_path: model.to_string_lossy().into(),
            arguments: vec![],
        };
        (root, config)
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cli_output_is_bounded_utf8_and_failures_are_sanitized() {
        for (body, expected) in [
            (
                "printf '  Synthetic transcript  '",
                Ok("Synthetic transcript"),
            ),
            ("printf '\\377'", Err("local_program_output_invalid")),
            (
                "head -c 32769 /dev/zero",
                Err("local_program_output_invalid"),
            ),
            (
                "printf 'private process diagnostic' >&2; exit 3",
                Err("local_program_inference_failed"),
            ),
        ] {
            let (_root, config) = fixture(body);
            let (_cancel, mut receiver) = watch::channel(0);
            assert_eq!(
                run(&config, SourceLanguage::English, &[0; 320], &mut receiver).await,
                expected.map(str::to_owned)
            );
            assert_eq!(
                std::fs::read(&config.model_path).unwrap(),
                b"user owned model"
            );
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_reaps_the_process_and_removes_private_temporary_audio() {
        use std::os::unix::fs::PermissionsExt;
        let (root, mut config) = fixture("");
        let state = root.path().join("state");
        std::fs::write(&config.executable, format!("#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = '-f' ]; then wav=\"$2\"; fi; shift; done\nprintf '%s\\n%s\\n' \"$$\" \"$wav\" > '{}'\nexec /bin/sleep 30\n", state.display())).unwrap();
        config.arguments = vec!["-t".into(), "4".into()];
        let (cancel, mut receiver) = watch::channel(0);
        let operation = tokio::spawn(async move {
            run(&config, SourceLanguage::English, &[0; 320], &mut receiver).await
        });
        let values = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(values) = std::fs::read_to_string(&state) {
                    if values.lines().count() >= 2 {
                        break values;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut lines = values.lines();
        let pid = lines.next().unwrap();
        let wav = Path::new(lines.next().unwrap());
        assert_eq!(
            std::fs::metadata(wav).unwrap().permissions().mode() & 0o777,
            0o600
        );
        cancel.send_replace(1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), operation)
                .await
                .unwrap()
                .unwrap(),
            Err("local_program_cancelled")
        );
        assert!(!wav.exists());
        assert!(!std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
    }
}
