//! Explicit private audio checks, excluded from normal tests and production.
use super::{select, DEVELOPMENT_APPLICATION_IDENTIFIER, DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE};
use crate::clients::gemini_live_client::GeminiLiveClient;
use crate::clients::provider_events::provider_event_channel;
use crate::core::models::TargetLanguage;
use crate::core::protocols::gemini_live::GeminiLiveEndpoint;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
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
}

#[derive(Default)]
struct Evidence {
    finals: Vec<Value>,
    errors: Vec<String>,
    first_translation_ms: Option<u128>,
    last_final_ms: Option<u128>,
    characters: usize,
}

impl Evidence {
    fn observe(&mut self, event: LiveTranslateServerEvent, started: Instant) {
        match event {
            LiveTranslateServerEvent::TranslationDraft(ref text) if !text.is_empty() => {
                self.first_translation_ms
                    .get_or_insert(started.elapsed().as_millis());
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
    let secret = select(&directory, false, DEVELOPMENT_APPLICATION_IDENTIFIER).unwrap();
    let key = secret
        .load(
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            "provider-profile:gemini-local-dev:googleGeminiLive:api-key",
        )
        .unwrap()
        .expect("Gemini dev key required");
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
        let pcm = std::fs::read(&clip.pcm_path).unwrap();
        assert!(!pcm.is_empty() && pcm.len() <= 60 * 32_000 && pcm.len() % 2 == 0);
        let (sender, mut receiver) = provider_event_channel();
        let client = GeminiLiveClient::new(&key, clip.target, sender).unwrap();
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
        // Let VAD finish the utterance before the existing bounded close path.
        for _ in 0..20 {
            client
                .send_audio(&vec![0; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        client.finish(Duration::from_secs(12)).await;
        let _ = stop_tx.send(());
        let evidence = reader.await.unwrap();
        let summary = json!({"label":clip.label,"setupMs":setup_ms,"audioMs":pcm.len() / 32,
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
                serde_json::to_string_pretty(&json!({"summary":summary,"finals":evidence.finals}))
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
