//! Explicit private audio checks, excluded from normal tests and production.
use super::SettingsStore;
use crate::clients::gemini_live_client::GeminiLiveClient;
use crate::clients::provider_events::provider_event_channel;
use crate::clients::provider_network::ProviderNetwork;
use crate::core::models::TargetLanguage;
use crate::core::protocols::gemini_live::GeminiLiveEndpoint;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::provider::ProviderKind;
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    clips: Vec<Clip>,
    output_directory: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clip {
    label: String,
    pcm_path: PathBuf,
    target: TargetLanguage,
    expected_speech: bool,
    #[serde(default = "default_tail_silence_ms")]
    tail_silence_ms: u64,
}

fn default_tail_silence_ms() -> u64 {
    2_000
}

#[derive(Default)]
struct Evidence {
    finals: Vec<Value>,
    errors: Vec<String>,
    first_translation_ms: Option<u128>,
    first_lexical_source_draft_ms: Option<u128>,
    source_draft_events: u64,
    source_lexical_draft_events: u64,
    translation_draft_events: u64,
    translation_lexical_draft_events: u64,
    last_final_ms: Option<u128>,
    last_source_draft: Option<Value>,
    last_translation_draft: Option<Value>,
    characters: usize,
}

impl Evidence {
    fn observe(&mut self, event: LiveTranslateServerEvent, started: Instant) {
        match event {
            LiveTranslateServerEvent::SourceDraft {
                ref text,
                ref language,
            }
            | LiveTranslateServerEvent::SourceUtteranceDraft {
                ref text,
                ref language,
                ..
            } => {
                self.last_source_draft =
                    Some(json!({"text":text.chars().take(4096).collect::<String>(),
                    "language":language,"truncated":text.chars().count() > 4096,
                    "elapsedMs":started.elapsed().as_millis()}));
                self.source_draft_events += 1;
                if text.chars().any(char::is_alphanumeric) {
                    self.source_lexical_draft_events += 1;
                    self.first_lexical_source_draft_ms
                        .get_or_insert(started.elapsed().as_millis());
                }
            }
            LiveTranslateServerEvent::TranslationDraft(ref text) => {
                self.last_translation_draft = Some(
                    json!({"text":text.chars().take(4096).collect::<String>(),
                    "truncated":text.chars().count() > 4096,"elapsedMs":started.elapsed().as_millis()}),
                );
                self.translation_draft_events += 1;
                self.translation_lexical_draft_events +=
                    u64::from(text.chars().any(char::is_alphanumeric));
                if !text.is_empty() {
                    self.first_translation_ms
                        .get_or_insert(started.elapsed().as_millis());
                }
            }
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            } => {
                self.characters += source.len() + translation.len();
                assert!(
                    self.finals.len() < 128 && self.characters <= 128 * 1024,
                    "private evidence limit"
                );
                if !translation.is_empty() {
                    self.first_translation_ms
                        .get_or_insert(started.elapsed().as_millis());
                }
                let elapsed = started.elapsed().as_millis();
                self.last_final_ms = Some(elapsed);
                self.finals.push(json!({"source":source,"language":language,"translation":translation,"elapsedMs":elapsed}));
            }
            LiveTranslateServerEvent::Error { code, .. } => self.errors.push(code),
            _ => {}
        }
    }
}

#[tokio::test]
#[ignore = "explicit authorized Gemini audio check; reads private manifest and dev key, writes private evidence"]
async fn manual_gemini_translation_audio() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let manifest_path = PathBuf::from(
        std::env::var_os("MIMI_GEMINI_BENCH_MANIFEST").expect("private manifest required"),
    );
    assert!(manifest_path.is_absolute());
    let bytes = std::fs::read(&manifest_path).unwrap();
    assert!(bytes.len() <= 64 * 1024);
    let manifest: Manifest = serde_json::from_slice(&bytes).unwrap();
    assert!(!manifest.clips.is_empty() && manifest.clips.len() <= 8);
    assert!(manifest.output_directory.is_absolute() && manifest.output_directory.is_dir());
    let directory = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/app.yuxino.mimi.dev");
    let settings = SettingsStore::load_for_manual_probe(directory).unwrap();
    let profile = settings.active_profile().unwrap();
    assert_eq!(profile.provider, ProviderKind::GoogleGeminiLive);
    let configuration = settings.configuration_for_profile_probe(&profile).unwrap();
    let key = configuration
        .credentials
        .direct_api_key()
        .expect("saved Gemini key required")
        .to_owned();
    for clip in manifest.clips {
        assert!(
            !clip.label.is_empty()
                && clip.label.len() <= 64
                && clip
                    .label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        );
        assert!(clip.pcm_path.is_absolute());
        assert!(clip.tail_silence_ms <= 2_000 && clip.tail_silence_ms % 100 == 0);
        // Explicit probes remain bounded to three minutes of PCM. Longer
        // fixtures must still respect the production transcript safety limit.
        let pcm = std::fs::read(&clip.pcm_path).unwrap();
        assert!(!pcm.is_empty() && pcm.len() <= 180 * 32_000 && pcm.len() % 2 == 0);
        let (sender, mut receiver) = provider_event_channel();
        let mut client = GeminiLiveClient::new(&key, clip.target, sender).unwrap();
        client
            .set_network(ProviderNetwork::resolve(&configuration.network_proxy).unwrap())
            .unwrap();
        let setup_started = Instant::now();
        client.connect().await.unwrap();
        let setup_ms = setup_started.elapsed().as_millis();
        let started = Instant::now();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let reader = tokio::spawn(async move {
            let mut evidence = Evidence::default();
            loop {
                tokio::select! {
                    event = receiver.recv() => match event {
                        Some(event) => evidence.observe(event, started),
                        None => break,
                    },
                    _ = &mut stop_rx => {
                        while let Ok(event) = receiver.try_recv() { evidence.observe(event, started); }
                        break;
                    }
                }
            }
            evidence
        });
        for (index, frame) in pcm
            .chunks(GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT)
            .enumerate()
        {
            tokio::time::sleep_until(tokio::time::Instant::from_std(
                started + Duration::from_millis(index as u64 * 100),
            ))
            .await;
            client.send_audio(frame).await.unwrap();
        }
        let sent_ms = started.elapsed().as_millis();
        // The default retains the existing 2s VAD tail; zero explicitly probes
        // Stop immediately after the clip, without changing the close deadline.
        for _ in 0..clip.tail_silence_ms / 100 {
            client
                .send_audio(&vec![0; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let finish_started_ms = started.elapsed().as_millis();
        client.finish(Duration::from_secs(12)).await;
        let finish_duration_ms = started.elapsed().as_millis() - finish_started_ms;
        let _ = stop_tx.send(());
        let evidence = reader.await.unwrap();
        let final_events_before_finish = evidence
            .finals
            .iter()
            .filter(|event| {
                event["elapsedMs"]
                    .as_u64()
                    .is_some_and(|ms| u128::from(ms) < finish_started_ms)
            })
            .count();
        let summary = json!({"label":clip.label,"setupMs":setup_ms,"audioMs":pcm.len() / 32,
            "firstLexicalSourceDraftMs":evidence.first_lexical_source_draft_ms,
            "sourceDraftEvents":evidence.source_draft_events,
            "sourceLexicalDraftEvents":evidence.source_lexical_draft_events,
            "translationDraftEvents":evidence.translation_draft_events,
            "translationLexicalDraftEvents":evidence.translation_lexical_draft_events,
            "tailSilenceMs":clip.tail_silence_ms,"finishStartedMs":finish_started_ms,
            "finishDurationMs":finish_duration_ms,"finalEventsBeforeFinish":final_events_before_finish,
            "sendFinishedMs":sent_ms,"firstTranslationMs":evidence.first_translation_ms,
            "lastFinalMs":evidence.last_final_ms,"finalCount":evidence.finals.len(),"errors":evidence.errors});
        let path = manifest
            .output_directory
            .join(format!("{}.json", clip.label));
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        output
            .write_all(
                serde_json::to_string_pretty(&json!({"summary":summary,"finals":evidence.finals,
                    "lastSourceDraft":evidence.last_source_draft,
                    "lastTranslationDraft":evidence.last_translation_draft}))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        println!("{summary}");
        assert!(
            evidence.errors.is_empty(),
            "live provider or finalization error"
        );
        assert_eq!(
            !evidence.finals.is_empty(),
            clip.expected_speech,
            "unexpected final presence"
        );
    }
}

#[test]
fn evidence_distinguishes_lexical_drafts_from_confirmations() {
    let mut evidence = Evidence::default();
    let started = Instant::now();
    for text in ["", " ... ", "Synthetic draft"] {
        evidence.observe(
            LiveTranslateServerEvent::SourceDraft {
                text: text.into(),
                language: None,
            },
            started,
        );
    }
    for text in ["", " ... ", "合成预览"] {
        evidence.observe(
            LiveTranslateServerEvent::TranslationDraft(text.into()),
            started,
        );
    }
    assert_eq!(evidence.source_draft_events, 3);
    assert_eq!(evidence.source_lexical_draft_events, 1);
    assert_eq!(evidence.translation_draft_events, 3);
    assert_eq!(evidence.translation_lexical_draft_events, 1);
    assert!(evidence.first_lexical_source_draft_ms.is_some());
    assert!(
        evidence.finals.is_empty(),
        "zero finals does not imply zero drafts"
    );
    evidence.observe(
        LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Synthetic confirmation".into(),
            language: None,
            translation: "合成确认".into(),
        },
        started,
    );
    assert_eq!(evidence.finals.len(), 1);
    assert_eq!(evidence.source_lexical_draft_events, 1);
}

#[test]
fn legacy_audio_clip_defaults_to_two_seconds_of_tail_silence() {
    let value = json!({"label":"synthetic", "pcm_path":"/private/tmp/synthetic.pcm",
        "target":"zh", "expected_speech":true});
    let legacy: Clip = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(legacy.tail_silence_ms, 2_000);
    let mut value = value;
    value["tail_silence_ms"] = json!(0);
    let immediate: Clip = serde_json::from_value(value).unwrap();
    assert_eq!(immediate.tail_silence_ms, 0);
}
