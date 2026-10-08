//! Explicit bounded text comparison through the production Qwen client.
//! Never compiled into the app; content goes only to opted-in private files.
use super::SettingsStore;
use crate::clients::{provider_network::ProviderNetwork, qwen_mt_client::QwenMTClient};
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::qwen_mt::{QwenMTClientError, QwenMTDomainHint, QwenMTModel};
use crate::core::provider::ProviderKind;
use serde::Deserialize;
use serde_json::json;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comparison {
    output_directory: PathBuf,
    samples: Vec<Sample>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    id: String,
    source: SourceLanguage,
    text: String,
}

#[tokio::test]
#[ignore = "explicit paid Qwen comparison: at most 36 synthetic text requests with private dev credentials"]
async fn manual_qwen_model_comparison() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let manifest = PathBuf::from(
        std::env::var_os("MIMI_QWEN_COMPARISON_MANIFEST").expect("private manifest required"),
    );
    assert!(manifest.is_absolute());
    assert!(std::fs::metadata(&manifest).unwrap().len() <= 64 * 1024);
    let comparison: Comparison = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    assert!(!comparison.samples.is_empty() && comparison.samples.len() <= 6);
    let root = comparison.output_directory.canonicalize().unwrap();
    assert!(root.starts_with("/private/tmp/mimi-debug-benchmark"));
    let metadata = std::fs::symlink_metadata(&comparison.output_directory).unwrap();
    assert!(metadata.is_dir() && metadata.permissions().mode() & 0o777 == 0o700);
    for sample in &comparison.samples {
        assert!(!sample.text.trim().is_empty() && sample.text.len() <= 4096);
        assert!(!sample.id.is_empty() && sample.id.len() <= 64);
        assert!(sample
            .id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-'));
        assert!(matches!(
            sample.source,
            SourceLanguage::English | SourceLanguage::Japanese
        ));
    }
    let directory = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/app.yuxino.mimi.dev");
    let settings = SettingsStore::load_for_manual_probe(directory).unwrap();
    let profile = settings.active_profile().unwrap();
    assert_eq!(profile.provider, ProviderKind::AlibabaCloud);
    let configuration = settings.configuration_for_profile_probe(&profile).unwrap();
    let key = configuration
        .credentials
        .alibaba_key()
        .expect("saved Alibaba key required");
    let models = [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus];
    let mut clients = Vec::new();
    for source in [SourceLanguage::English, SourceLanguage::Japanese] {
        for model in models {
            let target = TargetLanguage::SimplifiedChinese;
            let mut client = QwenMTClient::new(
                key,
                source,
                target,
                model,
                Some(QwenMTDomainHint::spoken_dialogue(source, target)),
                QwenMTDomainHint::filler_terms(source, target),
                Duration::from_secs(8),
            )
            .unwrap();
            client
                .set_network(ProviderNetwork::resolve(&configuration.network_proxy).unwrap())
                .unwrap();
            clients.push((source, model, client, 0usize));
        }
    }
    let mut failed = 0;
    for round in 0..2 {
        for (index, sample) in comparison.samples.iter().enumerate() {
            for offset in 0..3 {
                let model = models[(index + round + offset) % 3];
                let (_, _, client, requests) = clients
                    .iter_mut()
                    .find(|(source, selected, _, _)| *source == sample.source && *selected == model)
                    .unwrap();
                let cold = *requests == 0;
                *requests += 1;
                let started = Instant::now();
                let partials = Mutex::new((None::<u128>, 0usize));
                let result = if model == QwenMTModel::Plus {
                    client
                        .translate(&sample.text, Some(sample.source), &[])
                        .await
                } else {
                    client
                        .translate_streaming(&sample.text, Some(sample.source), &[], |text| {
                            if text.chars().any(char::is_alphanumeric) {
                                let mut timing = partials.lock().unwrap();
                                timing.0.get_or_insert(started.elapsed().as_millis());
                                timing.1 += 1;
                            }
                        })
                        .await
                };
                let complete_ms = started.elapsed().as_millis();
                let (first_ms, updates) = partials.into_inner().unwrap();
                let (translation, status) = match result {
                    Ok(text) if !text.trim().is_empty() => (Some(text), "ok".to_string()),
                    Ok(_) => {
                        failed += 1;
                        (None, "empty".to_string())
                    }
                    Err(error) => {
                        failed += 1;
                        let status = match error {
                            QwenMTClientError::RequestFailed { status_code, .. } => {
                                format!("http-{status_code}")
                            }
                            QwenMTClientError::RequestTimedOut => "timeout".into(),
                            _ => "client-error".into(),
                        };
                        (None, status)
                    }
                };
                let summary = json!({"sample":sample.id,"source":sample.source,"model":model,
                    "round":round + 1,"coldConnection":cold,"status":status,"firstTextMs":
                        if model == QwenMTModel::Plus && translation.is_some() { Some(complete_ms) } else { first_ms },
                    "completeMs":complete_ms,"partialUpdates":updates});
                let path = root.join(format!(
                    "{}-{}-{}.json",
                    sample.id,
                    model.raw_name(),
                    round + 1
                ));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                    .unwrap();
                file.write_all(
                    serde_json::to_string_pretty(&json!({"summary":summary,
                    "input":sample.text,"translation":translation}))
                    .unwrap()
                    .as_bytes(),
                )
                .unwrap();
                println!("{summary}");
            }
        }
    }
    assert_eq!(
        failed, 0,
        "comparison contains failed requests; inspect sanitized summaries"
    );
}
