//! Explicit, bounded same-PCM translation experiment. Never compiled into the app.
//! Accepted provider events are private evidence, not capture-to-screen timings.

use super::{
    prepare_manifest, private_evidence, PreparedClip, BYTES_PER_SECOND, FRAME_BYTES, TAIL_FRAMES,
};
use crate::clients::translation::translation_client::TranslationClient;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use crate::core::audio::pending_pcm::PendingPcmGate;
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode, UtteranceRole};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::translation::qwen_mt::QwenMTModel;
use crate::core::services::configuration::LiveTranslationConfiguration;
use crate::core::services::provider::AlibabaRealtimeMode;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

const BASELINE_MODEL: &str = "qwen-audio-3.0-asr-flash-streaming + qwen-mt-lite (Turbo)";
const MAX_CLIP_BYTES: usize = 20 * BYTES_PER_SECOND;
const MAX_OBSERVATIONS: usize = 2048;
const MAX_OBSERVATION_TEXT_BYTES: usize = 512 * 1024;
const MAX_SINGLE_TEXT_BYTES: usize = 64 * 1024;
const MAX_STREAM_ID_BYTES: usize = 256;
const MAX_PRIVATE_REPORT_BYTES: usize = 3 * 1024 * 1024;
const CONNECT_BOUND: Duration = Duration::from_secs(15);
const FINISH_BOUND: Duration = Duration::from_secs(20);
const DISCONNECT_BOUND: Duration = Duration::from_secs(5);

#[derive(Serialize)]
pub(super) struct Observation {
    pub(super) at_ms: u64,
    pub(super) kind: String,
    pub(super) stream_id: String,
    pub(super) text: String,
    pub(super) speech_end_ms: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct Trial {
    pub(super) model: String,
    pub(super) observations: Vec<Observation>,
    pub(super) failure: Option<String>,
}

#[derive(Default)]
struct Observer {
    observations: Vec<Observation>,
    latest: HashMap<(String, String), String>,
    text_bytes: usize,
    finished: bool,
}

impl Observer {
    fn push(
        &mut self,
        at_ms: u64,
        kind: &str,
        stream_id: String,
        text: String,
    ) -> Result<(), &'static str> {
        if text.len() > MAX_SINGLE_TEXT_BYTES {
            return Err("text_limit");
        }
        if stream_id.len() > MAX_STREAM_ID_BYTES {
            return Err("identity_limit");
        }
        let key = (kind.to_owned(), stream_id.clone());
        if self.latest.get(&key) == Some(&text) {
            return Ok(());
        }
        if self.observations.len() >= MAX_OBSERVATIONS
            || self.text_bytes.saturating_add(text.len()) > MAX_OBSERVATION_TEXT_BYTES
        {
            return Err("observation_limit");
        }
        self.text_bytes += text.len();
        self.latest.insert(key, text.clone());
        self.observations.push(Observation {
            at_ms,
            kind: kind.into(),
            stream_id,
            text,
            // The production facade does not expose Audio3 audio-clock ends.
            // A confirmed source arrives with the MT pair, not the raw ASR final.
            speech_end_ms: None,
        });
        Ok(())
    }

    fn record(&mut self, event: LiveTranslateServerEvent, at_ms: u64) -> Result<(), &'static str> {
        let source_id = |id: Option<u64>| {
            id.map_or_else(|| "unidentified-source".into(), |id| format!("source-{id}"))
        };
        match event {
            LiveTranslateServerEvent::SourceDraft { text, .. } => {
                self.push(at_ms, "source_preview", source_id(None), text)
            }
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id, text, ..
            } => self.push(at_ms, "source_preview", source_id(Some(utterance_id)), text),
            LiveTranslateServerEvent::SourceFinal { text, .. } => {
                self.push(at_ms, "source_final", source_id(None), text)
            }
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id, text, ..
            } => self.push(at_ms, "source_final", source_id(Some(utterance_id)), text),
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id,
                source,
                translation,
                ..
            } => {
                let id = source_id(source_utterance_id);
                self.push(at_ms, "source_preview", id.clone(), source)?;
                self.push(at_ms, "translation_preview", id, translation)
            }
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                translation,
                ..
            } => {
                let id = source_utterance_id.map_or_else(
                    || format!("confirmation-{utterance_id}"),
                    |id| source_id(Some(id)),
                );
                self.push(at_ms, "source_final", id.clone(), source)?;
                self.push(at_ms, "translation_final", id, translation)
            }
            LiveTranslateServerEvent::TranslationDraft(text) => self.push(
                at_ms,
                "translation_preview",
                "unidentified-translation".into(),
                text,
            ),
            LiveTranslateServerEvent::TranslationFinal(text) => self.push(
                at_ms,
                "translation_final",
                "unidentified-translation".into(),
                text,
            ),
            LiveTranslateServerEvent::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
                ..
            } => {
                let kind = match (role, is_final) {
                    (UtteranceRole::Source, false) => "source_preview",
                    (UtteranceRole::Source, true) => "source_final",
                    (UtteranceRole::Translation, false) => "translation_preview",
                    (UtteranceRole::Translation, true) => "translation_final",
                };
                self.push(at_ms, kind, format!("item-{utterance_id}"), text)
            }
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id,
                source,
                translation,
                ..
            } => {
                let id = format!("item-{utterance_id}");
                self.push(at_ms, "source_final", id.clone(), source)?;
                self.push(at_ms, "translation_final", id, translation)
            }
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                translation,
                ..
            } => {
                self.push(at_ms, "source_final", "unidentified-source".into(), source)?;
                self.push(
                    at_ms,
                    "translation_final",
                    "unidentified-translation".into(),
                    translation,
                )
            }
            LiveTranslateServerEvent::SessionFinished => {
                self.finished = true;
                self.push(at_ms, "finished", "session".into(), String::new())
            }
            // Cancel before the production retry cooldown can open a new request.
            LiveTranslateServerEvent::TranslationDeferred(_) => {
                Err("translation_deferred_no_retry")
            }
            LiveTranslateServerEvent::Error { code, .. } => Err(
                if code == "credential_authentication_failed" || code.contains(".authentication.") {
                    "credential_authentication_failed"
                } else {
                    "provider_error"
                },
            ),
            _ => Ok(()),
        }
    }

    fn drain(
        &mut self,
        receiver: &mut ProviderEventReceiver,
        started: Instant,
    ) -> Result<(), &'static str> {
        while let Ok(event) = receiver.try_recv_with_revision() {
            self.record(event.event, started.elapsed().as_millis() as u64)?;
        }
        Ok(())
    }
}

/// Runs the actual shared provider facade against fixed PCM. No settings save,
/// capture, reconnect or experiment-level retry belongs to this execution path.
pub(super) async fn compare_client(
    configuration: &LiveTranslationConfiguration,
    clip: &PreparedClip,
) -> Trial {
    let mut observer = Observer::default();
    let model = match configuration.alibaba_realtime_mode {
        AlibabaRealtimeMode::Disabled => BASELINE_MODEL,
        AlibabaRealtimeMode::LiveTranslate => "qwen3.8-livetranslate-flash-realtime",
    };
    if clip.pcm.is_empty() || clip.pcm.len() > MAX_CLIP_BYTES || !clip.pcm.len().is_multiple_of(2) {
        return Trial {
            model: model.into(),
            observations: vec![],
            failure: Some("one_short_clip_required".into()),
        };
    }
    if configuration.alibaba_realtime_mode != AlibabaRealtimeMode::Disabled
        && configuration.alibaba_workspace_id.trim().is_empty()
    {
        return Trial {
            model: model.into(),
            observations: vec![],
            failure: Some("workspace_required".into()),
        };
    }
    let (events, mut receiver) = provider_event_channel();
    let client = match TranslationClient::new(configuration, events) {
        Ok(client) => client,
        Err(_) => {
            return Trial {
                model: model.into(),
                observations: vec![],
                failure: Some("configuration_invalid".into()),
            }
        }
    };
    let pending = PendingPcmGate::default();
    client.set_audio_pending_gate(pending.clone());
    // All scheduled PCM is pending before readiness and through finish. This
    // suppresses Audio3's automatic idle-zero writer so the two PCM arms match.
    let _pending_guard =
        matches!(&client, TranslationClient::HighQuality(_)).then(|| pending.acquire());
    let connected = match tokio::time::timeout(CONNECT_BOUND, client.connect()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.safe_code()),
        Err(_) => Err("connect_timeout"),
    };
    let operation = match connected {
        Err(error) => Err(error),
        Ok(()) => {
            let started = Instant::now();
            let deadline =
                started + Duration::from_millis(clip.pcm.len() as u64 / 32 + 2000) + FINISH_BOUND;
            let (complete_tx, mut complete_rx) = watch::channel(false);
            let sender = async {
                let zeros = [0u8; FRAME_BYTES];
                let mut scheduled = started;
                for bytes in clip
                    .pcm
                    .chunks(FRAME_BYTES)
                    .chain((0..TAIL_FRAMES).map(|_| zeros.as_slice()))
                {
                    tokio::time::sleep_until(scheduled).await;
                    tokio::time::timeout(super::SEND_TIMEOUT, client.send_audio(bytes))
                        .await
                        .map_err(|_| "send_timeout")?
                        .map_err(|_| "send_failed")?;
                    scheduled += Duration::from_micros(
                        bytes.len() as u64 * 1_000_000 / BYTES_PER_SECOND as u64,
                    );
                }
                tokio::time::sleep_until(scheduled).await;
                tokio::time::timeout(FINISH_BOUND, client.finish())
                    .await
                    .map_err(|_| "finish_timeout")?;
                complete_tx.send_replace(true);
                Ok::<_, &'static str>(())
            };
            let receiving = async {
                loop {
                    tokio::select! {
                        biased;
                        changed = complete_rx.changed() => {
                            changed.map_err(|_| "completion_channel_closed")?;
                            if *complete_rx.borrow() {
                                observer.drain(&mut receiver, started)?;
                                return Ok::<_, &'static str>(());
                            }
                        }
                        event = receiver.recv_with_revision() => {
                            let event = event.ok_or("provider_channel_closed")?;
                            observer.record(event.event, started.elapsed().as_millis() as u64)?;
                        }
                    }
                }
            };
            match tokio::time::timeout_at(deadline, async { tokio::try_join!(sender, receiving) })
                .await
            {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(error),
                Err(_) => Err("trial_timeout"),
            }
        }
    };
    // A failed receiver cancels the sender/finish future, then disconnect aborts
    // pending MT work. There is no reconnect or experiment-level retry.
    let disconnected = tokio::time::timeout(DISCONNECT_BOUND, client.disconnect())
        .await
        .is_ok();
    let failure = operation.err().or(if !disconnected {
        Some("disconnect_timeout")
    } else if !observer.finished {
        Some("finish_not_confirmed")
    } else if !observer
        .observations
        .iter()
        .any(|o| o.kind == "translation_final" && !o.text.trim().is_empty())
    {
        Some("translation_final_missing")
    } else {
        None
    });
    Trial {
        model: model.into(),
        observations: observer.observations,
        failure: failure.map(str::to_owned),
    }
}

fn require_approval(value: Option<&str>) -> Result<(), &'static str> {
    if value == Some("yes") {
        Ok(())
    } else {
        Err("explicit_budget_approval_required")
    }
}

fn validate_clip(clips: &[PreparedClip]) -> Result<(), &'static str> {
    if clips.len() != 1 || clips[0].pcm.is_empty() || clips[0].pcm.len() > MAX_CLIP_BYTES {
        Err("one_short_clip_required")
    } else {
        Ok(())
    }
}

fn selected_arms(value: Option<&str>) -> Result<Vec<AlibabaRealtimeMode>, &'static str> {
    match value {
        None | Some("baseline") => Ok(vec![AlibabaRealtimeMode::Disabled]),
        Some("liveTranslate") => Ok(vec![AlibabaRealtimeMode::LiveTranslate]),
        Some("all") => Ok(vec![
            AlibabaRealtimeMode::Disabled,
            AlibabaRealtimeMode::LiveTranslate,
        ]),
        _ => Err("arm_invalid"),
    }
}

/// Use the system hash utility through stdin, never a media pathname or shell.
/// The command receives no inherited credentials or interpreter environment.
fn pcm_sha256(pcm: &[u8]) -> Result<String, &'static str> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "hash_failed")?;
    let written = child.stdin.take().ok_or("hash_failed")?.write_all(pcm);
    if written.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("hash_failed");
    }
    let result = child.wait_with_output().map_err(|_| "hash_failed")?;
    if !result.status.success() || result.stdout.len() > 128 {
        return Err("hash_failed");
    }
    let text = std::str::from_utf8(&result.stdout).map_err(|_| "hash_failed")?;
    let digest = text.split_ascii_whitespace().next().ok_or("hash_failed")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'a'..=b'f'))
    {
        return Err("hash_failed");
    }
    Ok(digest.into())
}

fn trial_value(trial: &Trial, hash: &str) -> Result<serde_json::Value, &'static str> {
    let mut value = serde_json::to_value(trial).map_err(|_| "report_invalid")?;
    value["input_pcm_sha256"] = json!(hash);
    Ok(value)
}

fn private_json(path: &Path, value: &serde_json::Value) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "report_invalid")?;
    if bytes.len() > MAX_PRIVATE_REPORT_BYTES {
        return Err("report_limit");
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| "evidence_write_failed")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "evidence_write_failed")
}

#[cfg(all(feature = "development-debugger", target_os = "macos"))]
#[tokio::test]
#[ignore = "explicit budget approval required; one selected paid audio arm by default, all only by explicit request"]
async fn manual_three_arm_translation_comparison() {
    use crate::core::services::provider::{ProviderKind, TextTranslation};
    use crate::settings_store::{SettingsStore, DEVELOPMENT_APPLICATION_IDENTIFIER};
    let run = async {
        require_approval(std::env::var("MIMI_TRANSLATION_COMPARISON_APPROVED").ok().as_deref())?;
        let arms = selected_arms(std::env::var("MIMI_TRANSLATION_COMPARISON_ARM").ok().as_deref())?;
        let manifest = std::env::var_os("MIMI_ASR_BENCH_MANIFEST").map(PathBuf::from).ok_or("manifest_required")?;
        let clips = prepare_manifest(&manifest).map_err(|_| "manifest_or_input_invalid")?;
        validate_clip(&clips)?;
        let clip = &clips[0];
        let hash = pcm_sha256(&clip.pcm)?;
        let workspace = std::env::var("MIMI_TRANSLATION_COMPARISON_WORKSPACE").unwrap_or_default();
        if workspace.len() > 63 || workspace.starts_with('-') || workspace.ends_with('-')
            || !workspace.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
            return Err("workspace_invalid");
        }
        let root = std::env::var_os("MIMI_ASR_BENCH_PRIVATE_OUTPUT_DIR").map(PathBuf::from).ok_or("private_output_required")?;
        private_evidence::validate_root(&root).map_err(|_| "private_output_invalid")?;
        let output = root.join(format!("translation-comparison-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&output).map_err(|_| "evidence_write_failed")?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).map_err(|_| "evidence_write_failed")?;
        }
        let directory = std::env::var_os("MIMI_ASR_BENCH_CONFIG_DIR").map(PathBuf::from).or_else(||
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support").join(DEVELOPMENT_APPLICATION_IDENTIFIER))
        ).ok_or("credentials_unavailable")?;
        let settings = SettingsStore::load_for_read_only_manual_probe(directory).map_err(|_| "credentials_unavailable")?;
        let profile = settings.active_profile().map_err(|_| "configuration_unavailable")?;
        if profile.provider != ProviderKind::AlibabaCloud || profile.text_translation() != TextTranslation::FollowService {
            return Err("saved_alibaba_default_route_required");
        }
        let mut configuration = settings.configuration_for_profile_probe(&profile).map_err(|_| "configuration_unavailable")?;
        if configuration.credentials.alibaba_key().is_none_or(|key| key.is_empty()) { return Err("credentials_unavailable"); }
        configuration.source_language = clip.language.source();
        configuration.target_language = TargetLanguage::SimplifiedChinese;
        configuration.translation_mode = TranslationMode::Turbo;
        configuration.qwen_mt_model = QwenMTModel::Lite;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut trials = Vec::with_capacity(arms.len());
        let mut failures = 0usize;
        let mut terminal_failure = None;
        for arm in &arms {
            configuration.alibaba_realtime_mode = *arm;
            configuration.alibaba_workspace_id = if *arm == AlibabaRealtimeMode::Disabled { String::new() } else { workspace.clone() };
            configuration.source_language = if *arm == AlibabaRealtimeMode::Disabled { clip.language.source() } else { SourceLanguage::Automatic };
            let trial = compare_client(&configuration, clip).await;
            let mut value = trial_value(&trial, &hash)?;
            value["configured_source_language"] = json!(configuration.source_language);
            let name = match arm {
                AlibabaRealtimeMode::Disabled => "arm-a.json",
                AlibabaRealtimeMode::LiveTranslate => "arm-b.json",
            };
            private_json(&output.join(name), &value)?;
            failures += usize::from(trial.failure.is_some());
            terminal_failure = match trial.failure.as_deref() {
                Some("credential_authentication_failed") => Some("credential_authentication_failed"),
                Some("disconnect_timeout") => Some("cleanup_failed"),
                _ => None,
            };
            trials.push(value);
            if terminal_failure.is_some() { break; }
        }
        private_json(&output.join("results.json"), &json!({
            "schema_version": 1,
            "input_duration_ms": clip.pcm.len() as u64 / 32,
            "input_pcm_sha256": hash,
            "explicit_tail_ms": 2000,
            "reference_language": clip.language.source(),
            "target_language": "zh",
            "timing_origin": "first scheduled PCM send after provider readiness; setup excluded",
            "baseline_event_boundary": "production provider acceptance; drafts may coalesce; source final arrives with translated pair; audio-clock speech ends unavailable",
            "source_hint_boundary": "Audio3 uses the manifest ASR language hint; LiveTranslate uses automatic source detection as required by the 3.8 realtime protocol; the PCM is identical but source-language configuration differs",
            "requested_arms": arms.len(),
            "completed_arms": trials.len(),
            "trials": trials,
        }))?;
        println!("TRANSLATION_COMPARISON_JSON {}", json!({"arms": trials.len(), "failures": failures, "inputDurationMs": clip.pcm.len() as u64 / 32, "runnerRetries": 0}));
        if let Some(error) = terminal_failure { Err(error) }
        else if failures == 0 { Ok::<_, &'static str>(()) }
        else { Err("one_or_more_trial_failures") }
    }.await;
    if let Err(error) = run {
        panic!("translation_comparison_failed:{error}");
    }
}

#[test]
fn approval_is_required_before_provider_or_credentials_work() {
    assert_eq!(
        require_approval(None),
        Err("explicit_budget_approval_required")
    );
    assert_eq!(
        require_approval(Some("true")),
        Err("explicit_budget_approval_required")
    );
    assert_eq!(require_approval(Some("yes")), Ok(()));
}

#[test]
fn accepted_pairs_preserve_source_identity_and_do_not_duplicate_unchanged_text() {
    let mut observer = Observer::default();
    let preview = || LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: Some(7),
        source: "synthetic source".into(),
        language: None,
        translation: "synthetic preview".into(),
    };
    observer.record(preview(), 10).unwrap();
    observer.record(preview(), 20).unwrap();
    observer
        .record(
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id: 2,
                source_utterance_id: Some(7),
                source: "synthetic source".into(),
                language: None,
                translation: "synthetic final".into(),
            },
            30,
        )
        .unwrap();
    assert_eq!(observer.observations.len(), 4);
    assert!(observer
        .observations
        .iter()
        .all(|o| o.stream_id == "source-7" && o.speech_end_ms.is_none()));
    assert_eq!(observer.observations[3].kind, "translation_final");
    assert_eq!(observer.observations[3].at_ms, 30);
}

#[test]
fn arbitrary_error_content_does_not_cross_the_result_boundary() {
    let mut observer = Observer::default();
    assert_eq!(
        observer.record(
            LiveTranslateServerEvent::Error {
                code: "arbitrary-private-provider-value".into(),
                message: "arbitrary-private-message".into(),
            },
            0
        ),
        Err("provider_error")
    );
    assert!(observer.observations.is_empty());
    assert_eq!(
        observer.push(
            0,
            "translation_preview",
            "synthetic".into(),
            "x".repeat(MAX_SINGLE_TEXT_BYTES + 1)
        ),
        Err("text_limit")
    );
    assert_eq!(
        observer.push(
            0,
            "source_preview",
            "i".repeat(MAX_STREAM_ID_BYTES + 1),
            String::new()
        ),
        Err("identity_limit")
    );
}

#[test]
fn completion_can_precede_a_late_confirmed_translation() {
    let mut observer = Observer::default();
    observer
        .record(LiveTranslateServerEvent::SessionFinished, 10)
        .unwrap();
    observer
        .record(
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: "synthetic-item".into(),
                source: "synthetic source".into(),
                language: None,
                translation: "synthetic translation".into(),
            },
            20,
        )
        .unwrap();
    assert!(observer.finished);
    assert_eq!(observer.observations[2].kind, "translation_final");
    assert_eq!(observer.observations[2].stream_id, "item-synthetic-item");
    assert_eq!(observer.observations[2].at_ms, 20);
}

#[test]
fn each_arm_requires_its_own_explicit_selection() {
    assert_eq!(
        selected_arms(None).unwrap(),
        vec![AlibabaRealtimeMode::Disabled]
    );
    assert_eq!(
        selected_arms(Some("liveTranslate")).unwrap(),
        vec![AlibabaRealtimeMode::LiveTranslate]
    );
    assert!(selected_arms(Some("omni")).is_err());
    assert_eq!(selected_arms(Some("all")).unwrap().len(), 2);
    assert!(selected_arms(Some("both")).is_err());
}

#[test]
fn the_system_hash_receipt_matches_a_known_sha256_vector() {
    assert_eq!(
        pcm_sha256(b"abc").unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn one_short_clip_limit_does_not_expand_the_audio_budget() {
    let mut clip = super::synthetic_clip();
    clip.pcm = vec![0; MAX_CLIP_BYTES];
    assert_eq!(validate_clip(std::slice::from_ref(&clip)), Ok(()));
    clip.pcm.extend_from_slice(&[0, 0]);
    assert_eq!(
        validate_clip(std::slice::from_ref(&clip)),
        Err("one_short_clip_required")
    );
    assert_eq!(validate_clip(&[]), Err("one_short_clip_required"));
}
