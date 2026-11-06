//! Opted-in public-PCM service checks. No native capture, content log or settings write.
//! The caller verifies the archived public sample and limits execution to two processes.

use crate::clients::baidu_translate_client::BaiduTranslateClientError;
use crate::clients::provider_events::provider_event_channel;
use crate::clients::tencent_cloud_client::TencentCloudClientError;
use crate::clients::translation_client::{ConnectError, TranslationClient};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode, UtteranceRole};
use crate::core::protocols::live_translate::LiveTranslateServerEvent as Event;
use crate::core::provider::{
    AlibabaRealtimeMode, ProfileLanguagePreset, ProviderKind, SpeechTranslationMode,
    TextTranslation,
};
use crate::core::support_diagnostics::SafeFailure;
use crate::settings_store::{SettingsStore, DEVELOPMENT_APPLICATION_IDENTIFIER};
use mimi_runtime::core::health::{
    probe_connection_health, DISCONNECT_TIMEOUT, HEALTH_CHECK_INTERVAL,
};
use mimi_runtime::core::recovery::normalize_provider_error_code;
use serde::Serialize;
use serde_json::json;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PHASES: [&str; 4] = ["speech", "silence", "resumed_speech", "finish"];

#[derive(Default, Serialize)]
struct PhaseCounts {
    source_events: u64,
    source_finals: u64,
    source_clear_events: u64,
    translation_events: u64,
    translation_finals: u64,
    translation_clear_events: u64,
    confirmed_pairs: u64,
    sent_frames: u64,
    sent_bytes: u64,
    health_checks: u64,
    maximum_health_ms: u64,
    first_content_ms: Option<u64>,
    last_content_ms: Option<u64>,
}

impl PhaseCounts {
    fn source(&mut self, text: &str, final_event: bool) {
        if !text.trim().is_empty() {
            self.source_events += 1;
            self.source_finals += u64::from(final_event);
        }
    }

    fn translation(&mut self, text: &str, final_event: bool) {
        if !text.trim().is_empty() {
            self.translation_events += 1;
            self.translation_finals += u64::from(final_event);
        }
    }

    fn observe(&mut self, event: &Event, at_ms: u64) {
        let before = self.source_events + self.translation_events;
        match event {
            Event::SourceDraft { text, .. } | Event::SourceUtteranceDraft { text, .. } => {
                self.source_clear_events += u64::from(text.is_empty());
                self.source(text, false);
            }
            Event::SourceFinal { text, .. } | Event::SourceUtteranceFinal { text, .. } => {
                self.source(text, true);
            }
            Event::TranslationDraft(text) => {
                self.translation_clear_events += u64::from(text.is_empty());
                self.translation(text, false);
            }
            Event::TranslationFinal(text) => self.translation(text, true),
            Event::UtteranceText {
                role,
                text,
                is_final,
                ..
            } => match role {
                UtteranceRole::Source => self.source(text, *is_final),
                UtteranceRole::Translation => self.translation(text, *is_final),
            },
            Event::SubtitlePreviewPair {
                source,
                translation,
                ..
            }
            | Event::SubtitleIdentifiedPreviewPair {
                source,
                translation,
                ..
            } => {
                self.source(source, false);
                self.translation(translation, false);
            }
            Event::SubtitleFinalPair {
                source,
                translation,
                ..
            }
            | Event::SubtitleIdentifiedFinalPair {
                source,
                translation,
                ..
            }
            | Event::SubtitleConfirmedPair {
                source,
                translation,
                ..
            } => {
                self.source(source, true);
                self.translation(translation, true);
                self.confirmed_pairs += u64::from(!source.is_empty() && !translation.is_empty());
            }
            _ => {}
        }
        if self.source_events + self.translation_events > before {
            self.first_content_ms.get_or_insert(at_ms);
            self.last_content_ms = Some(at_ms);
        }
    }
}

#[derive(Serialize)]
struct Failure {
    boundary: &'static str,
    code: &'static str,
    diagnostic: Option<SafeFailure>,
    provider_status: Option<ProviderStatus>,
    at_ms: u64,
}

#[derive(Serialize)]
struct ProviderStatus {
    code: &'static str,
    provider_status_code: i64,
}

#[derive(Default, Serialize)]
struct Observation {
    phases: [PhaseCounts; 4],
    first_failure: Option<Failure>,
    session_finished: u64,
}

impl Observation {
    fn fail(&mut self, boundary: &'static str, code: &'static str, started: Instant) {
        self.fail_with_diagnostic(boundary, code, None, None, started);
    }

    fn fail_with_diagnostic(
        &mut self,
        boundary: &'static str,
        code: &'static str,
        diagnostic: Option<SafeFailure>,
        provider_status: Option<ProviderStatus>,
        started: Instant,
    ) {
        self.first_failure.get_or_insert(Failure {
            boundary,
            code,
            diagnostic,
            provider_status,
            at_ms: started.elapsed().as_millis() as u64,
        });
    }

    fn fail_connection(&mut self, boundary: &'static str, error: &ConnectError, started: Instant) {
        let provider_status = match error {
            ConnectError::BaiduTranslate(
                BaiduTranslateClientError::SessionSetupRejectedWithCode(code),
            ) => Some(ProviderStatus {
                code: "BAIDU_PROVIDER_REJECTED",
                provider_status_code: *code,
            }),
            ConnectError::TencentCloud(TencentCloudClientError::ProviderRejected { code }) => {
                Some(ProviderStatus {
                    code: "TENCENT_PROVIDER_REJECTED",
                    provider_status_code: *code,
                })
            }
            _ => None,
        };
        self.fail_with_diagnostic(
            boundary,
            error.safe_code(),
            Some(SafeFailure::from_error(
                error
                    .recognition_failure_label()
                    .unwrap_or(&error.to_string()),
            )),
            provider_status,
            started,
        );
    }

    fn observe(&mut self, event: &Event, phase: usize, closing: bool, started: Instant) {
        self.phases[phase].observe(event, started.elapsed().as_millis() as u64);
        match event {
            Event::Error { code, message } => self.fail_with_diagnostic(
                "provider_event",
                match code.as_str() {
                    "openai_close_timeout" => "openai_close_timeout",
                    "openai_session_close_failed" => "openai_session_close_failed",
                    _ => normalize_provider_error_code(code),
                },
                Some(SafeFailure::from_provider_error(code, message)),
                None,
                started,
            ),
            Event::SessionFinished => {
                self.session_finished += 1;
                if !closing {
                    self.fail("provider_event", "unexpected_session_finish", started);
                }
            }
            _ => {}
        }
    }
}

fn saved_configuration(
) -> Result<(String, &'static str, LiveTranslationConfiguration), &'static str> {
    let scope = match std::env::var("MIMI_PROVIDER_PUBLIC_CONFIG_SCOPE").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("development") => {
            DEVELOPMENT_APPLICATION_IDENTIFIER
        }
        Ok("production") => "app.yuxino.mimi",
        _ => return Err("manual_probe_scope_invalid"),
    };
    let directory = PathBuf::from(std::env::var_os("HOME").ok_or("home_unavailable")?)
        .join("Library/Application Support")
        .join(scope);
    let settings = SettingsStore::load_for_scoped_read_only_manual_probe(directory, scope)?;
    let id = std::env::var("MIMI_PROVIDER_PUBLIC_PROFILE").map_err(|_| "profile_required")?;
    let mut profile = settings
        .profile_catalog()
        .map_err(|_| "catalog_unavailable")?
        .1
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or("profile_unavailable")?;
    let mut route = match std::env::var("MIMI_PROVIDER_PUBLIC_TEXT_ROUTE").as_deref() {
        Err(std::env::VarError::NotPresent) => "saved",
        Ok("deepLX")
            if profile.provider == ProviderKind::AlibabaCloud
                && profile.alibaba_realtime_mode == AlibabaRealtimeMode::Disabled =>
        {
            profile.text_translation = Some(TextTranslation::DeepLX);
            "deepLX"
        }
        _ => return Err("route_invalid"),
    };
    let recognition_only = match std::env::var("MIMI_PROVIDER_PUBLIC_RECOGNITION_ONLY").as_deref() {
        Err(std::env::VarError::NotPresent) => false,
        Ok("1") if profile.provider == ProviderKind::OpenAIRealtime && route == "saved" => true,
        _ => return Err("recognition_route_invalid"),
    };
    if recognition_only {
        profile.speech_translation_mode = SpeechTranslationMode::AsrMt;
        profile.language_preset = Some(ProfileLanguagePreset {
            source_language: SourceLanguage::Automatic,
            target_language: TargetLanguage::Original,
        });
        route = "openai_asr_only";
    }
    if !matches!(
        profile.provider,
        ProviderKind::AlibabaCloud
            | ProviderKind::OpenAIRealtime
            | ProviderKind::GoogleGeminiLive
            | ProviderKind::BaiduTranslate
            | ProviderKind::TencentCloud
            | ProviderKind::VolcanoEngine
    ) || (profile.uses_realtime_asr() && !recognition_only)
    {
        return Err("profile_outside_probe_scope");
    }
    let mut configuration = if recognition_only {
        settings.configuration_for_profile_selection(&profile)
    } else {
        settings.configuration_for_profile_probe(&profile)
    }
    .map_err(|_| "saved_configuration_incomplete")?;
    // Only this in-memory run changes languages/mode; no active profile is selected.
    let automatic = matches!(
        profile.provider,
        ProviderKind::OpenAIRealtime | ProviderKind::GoogleGeminiLive
    ) || profile.alibaba_realtime_mode == AlibabaRealtimeMode::LiveTranslate;
    configuration.source_language = requested_source_language(
        std::env::var("MIMI_PROVIDER_PUBLIC_SOURCE_LANGUAGE")
            .ok()
            .as_deref(),
        automatic,
    )?;
    configuration.target_language = if recognition_only {
        TargetLanguage::Original
    } else {
        TargetLanguage::SimplifiedChinese
    };
    if profile.provider == ProviderKind::AlibabaCloud {
        configuration.translation_mode = TranslationMode::Turbo;
    }
    let configuration = configuration
        .validated()
        .map_err(|_| "probe_configuration_invalid")?;
    Ok((id, route, configuration))
}

fn requested_source_language(
    requested: Option<&str>,
    automatic_default: bool,
) -> Result<SourceLanguage, &'static str> {
    match requested {
        Some(code) => SourceLanguage::ALL
            .into_iter()
            .find(|language| language.raw_value() == code)
            .ok_or("probe_source_language_invalid"),
        None if automatic_default => Ok(SourceLanguage::Automatic),
        None => Ok(SourceLanguage::English),
    }
}

fn public_pcm(sample_rate: u32, resumed: bool) -> Result<Vec<u8>, &'static str> {
    let variable = match (sample_rate, resumed) {
        (16_000, false) => "MIMI_PROVIDER_PUBLIC_PCM16",
        (24_000, false) => "MIMI_PROVIDER_PUBLIC_PCM24",
        (16_000, true) => "MIMI_PROVIDER_PUBLIC_RESUMED_PCM16",
        (24_000, true) => "MIMI_PROVIDER_PUBLIC_RESUMED_PCM24",
        _ => return Err("sample_rate_invalid"),
    };
    let path = PathBuf::from(std::env::var_os(variable).ok_or("public_pcm_required")?);
    if !path.is_absolute() {
        return Err("public_pcm_invalid");
    }
    let maximum = u64::from(sample_rate) * 2 * 60;
    let before = std::fs::symlink_metadata(&path).map_err(|_| "public_pcm_unavailable")?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > maximum {
        return Err("public_pcm_invalid");
    }
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x100)
        .open(path)
        .map_err(|_| "public_pcm_unavailable")?; // macOS O_NOFOLLOW
    let after = opened.metadata().map_err(|_| "public_pcm_unavailable")?;
    if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
        return Err("public_pcm_changed");
    }
    let mut pcm = Vec::new();
    opened
        .take(maximum + 1)
        .read_to_end(&mut pcm)
        .map_err(|_| "public_pcm_unavailable")?;
    if pcm.is_empty() || pcm.len() as u64 > maximum || !pcm.len().is_multiple_of(2) {
        return Err("public_pcm_invalid");
    }
    Ok(pcm)
}

fn frame_duration(provider: ProviderKind) -> Duration {
    use mimi_runtime::core::protocols::{
        baidu_translate::BaiduTranslateEndpoint, gemini_live::GeminiLiveEndpoint,
        openai_realtime::OpenAIRealtimeEndpoint, tencent_cloud::TencentCloudEndpoint,
        volcano_engine::VolcanoEngineEndpoint,
    };
    Duration::from_millis(u64::from(match provider {
        ProviderKind::BaiduTranslate => BaiduTranslateEndpoint::FRAME_DURATION_MS,
        ProviderKind::GoogleGeminiLive => GeminiLiveEndpoint::FRAME_DURATION_MS,
        ProviderKind::OpenAIRealtime => OpenAIRealtimeEndpoint::FRAME_DURATION_MS,
        ProviderKind::TencentCloud => TencentCloudEndpoint::FRAME_DURATION_MS,
        ProviderKind::VolcanoEngine => VolcanoEngineEndpoint::FRAME_DURATION_MS,
        _ => 100,
    }))
}

async fn run() -> Result<(), &'static str> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    use mimi_runtime::core::development_debug::{self, DebugEvent, TraceSink};
    let mt_requests = Arc::new(AtomicUsize::new(0));
    let same_language_finals = Arc::new(AtomicUsize::new(0));
    let requests = Arc::clone(&mt_requests);
    let passthrough = Arc::clone(&same_language_finals);
    development_debug::initialize(true);
    development_debug::set_enabled(true)?;
    let counter_sink = TraceSink::new(move |entry| {
        if let DebugEvent::Pipeline { label } = entry.event {
            if label.starts_with("mt request started ") {
                requests.fetch_add(1, Ordering::SeqCst);
            } else if label.starts_with("mt final skipped label=same_language ") {
                passthrough.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    development_debug::set_sink(Some(counter_sink.clone()))?;
    let (profile_id, route, configuration) = saved_configuration()?;
    // A bounded diagnostic comparison, not a production finish-policy change.
    let openai_finish_override =
        match std::env::var("MIMI_PROVIDER_PUBLIC_OPENAI_FINISH_SECONDS").as_deref() {
            Err(std::env::VarError::NotPresent) => None,
            Ok("15") if configuration.provider == ProviderKind::OpenAIRealtime => {
                Some(Duration::from_secs(15))
            }
            _ => return Err("probe_finish_override_invalid"),
        };
    let silence_seconds = match std::env::var("MIMI_PROVIDER_PUBLIC_SILENCE_SECONDS").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("90") => 90,
        Ok("0") => 0,
        Ok("1200") => 1200,
        _ => return Err("silence_duration_invalid"),
    };
    let send_silence_pcm = match std::env::var("MIMI_PROVIDER_PUBLIC_SILENCE_INPUT").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("zero_pcm") => true,
        Ok("no_pcm") => false,
        _ => return Err("silence_input_invalid"),
    };
    let sample_rate = configuration.capabilities().input_sample_rate_hz;
    let pcm = public_pcm(sample_rate, false)?;
    let resumed_pcm = if std::env::var("MIMI_PROVIDER_PUBLIC_SEPARATE_RESUME").as_deref() == Ok("1")
    {
        public_pcm(sample_rate, true)?
    } else {
        pcm.clone()
    };
    let sample = std::env::var("MIMI_PROVIDER_PUBLIC_SAMPLE_LABEL")
        .unwrap_or_else(|_| "fleurs-english-1527".into());
    if sample.is_empty()
        || sample.len() > 64
        || !sample
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("sample_label_invalid");
    }
    let frame_time = frame_duration(configuration.provider);
    let frame_bytes = (u128::from(sample_rate) * 2 * frame_time.as_millis() / 1000) as usize;
    let play_once = std::env::var("MIMI_PROVIDER_PUBLIC_ONCE").as_deref() == Ok("1");
    let speech_frames = |bytes: usize| {
        if play_once {
            bytes.div_ceil(frame_bytes) as u64
        } else {
            60_000 / frame_time.as_millis() as u64
        }
    };
    let phase_frames = [
        speech_frames(pcm.len()),
        silence_seconds * 1000 / frame_time.as_millis() as u64,
        speech_frames(resumed_pcm.len()),
    ];
    let started = Instant::now();
    let observed = Arc::new(Mutex::new(Observation::default()));
    let phase = Arc::new(AtomicUsize::new(0));
    let closing = Arc::new(AtomicBool::new(false));
    let (sender, mut receiver) = provider_event_channel();
    let client = TranslationClient::new(&configuration, sender)
        .map_err(|_| "client_configuration_invalid")?;
    let source_isolation_baseline =
        std::env::var("MIMI_PROVIDER_PUBLIC_SOURCE_BASELINE").as_deref() == Ok("1");
    if source_isolation_baseline {
        let TranslationClient::OpenAIRealtime(openai) = &client else {
            return Err("source_baseline_provider_invalid");
        };
        openai.disable_source_isolation_for_comparison().await;
    }
    let reader_observed = Arc::clone(&observed);
    let reader_phase = Arc::clone(&phase);
    let reader_closing = Arc::clone(&closing);
    let (stop_reader, mut stop_requested) = tokio::sync::oneshot::channel();
    let mut reader = tokio::spawn(async move {
        loop {
            let envelope = tokio::select! {
                event = receiver.recv_with_revision() => match event {
                    Some(envelope) => envelope,
                    None => break,
                },
                _ = &mut stop_requested => {
                    while let Ok(envelope) = receiver.try_recv_with_revision() {
                        reader_observed.lock().unwrap().observe(&envelope.event,
                            reader_phase.load(Ordering::SeqCst), reader_closing.load(Ordering::SeqCst), started);
                    }
                    break;
                }
            };
            reader_observed.lock().unwrap().observe(
                &envelope.event,
                reader_phase.load(Ordering::SeqCst),
                reader_closing.load(Ordering::SeqCst),
                started,
            );
        }
    });
    println!(
        "{}",
        json!({"phase":"preflight","profile_id":profile_id,"provider":configuration.provider,
        "route":route,"sample":sample,"sample_rate":sample_rate,"sample_bytes":pcm.len(),"resumed_sample_bytes":resumed_pcm.len(),
        "frame_ms":frame_time.as_millis(),"source":configuration.source_language.raw_value(),"target":configuration.target_language.raw_value(),
        "silence_input": if send_silence_pcm { "zero_pcm" } else { "no_pcm" },
        "play_once":play_once,"source_isolation_baseline":source_isolation_baseline,"phase_frames":phase_frames,
        "alibaba_realtime_mode":configuration.alibaba_realtime_mode,
        "openai_finish_override_ms":openai_finish_override.map(|value| value.as_millis())})
    );
    match tokio::time::timeout(Duration::from_secs(30), client.connect()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => observed
            .lock()
            .unwrap()
            .fail_connection("connect", &error, started),
        Err(_) => observed
            .lock()
            .unwrap()
            .fail("connect", "connect_timeout", started),
    }
    println!(
        "{}",
        json!({"phase":"connect_result","elapsed_ms":started.elapsed().as_millis(),
        "ready":observed.lock().unwrap().first_failure.is_none()})
    );
    let health_client = client.clone();
    let health_observed = Arc::clone(&observed);
    let health_phase = Arc::clone(&phase);
    let health = tokio::spawn(async move {
        let mut timer = tokio::time::interval(HEALTH_CHECK_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        timer.tick().await;
        loop {
            timer.tick().await;
            match probe_connection_health(&health_client).await {
                Ok(elapsed) => {
                    let mut observed = health_observed.lock().unwrap();
                    let counts = &mut observed.phases[health_phase.load(Ordering::SeqCst)];
                    counts.health_checks += 1;
                    counts.maximum_health_ms =
                        counts.maximum_health_ms.max(elapsed.as_millis() as u64);
                }
                Err(error) => {
                    health_observed
                        .lock()
                        .unwrap()
                        .fail_connection("health", &error, started);
                    break;
                }
            }
        }
    });
    for (index, frames) in phase_frames.into_iter().enumerate() {
        if observed.lock().unwrap().first_failure.is_some() {
            break;
        }
        phase.store(index, Ordering::SeqCst);
        println!(
            "{}",
            json!({"phase":PHASES[index],"boundary":"start","elapsed_ms":started.elapsed().as_millis()})
        );
        let stage_started = tokio::time::Instant::now();
        let mut cursor = 0;
        for frame_index in 0..frames {
            tokio::time::sleep_until(stage_started + frame_time.mul_f64(frame_index as f64)).await;
            if observed.lock().unwrap().first_failure.is_some() {
                break;
            }
            if index == 1 && !send_silence_pcm {
                continue;
            }
            let mut frame = vec![0; frame_bytes];
            if index != 1 {
                let pcm = if index == 2 { &resumed_pcm } else { &pcm };
                for byte in &mut frame {
                    if cursor < pcm.len() {
                        *byte = pcm[cursor];
                        cursor += 1;
                        if cursor == pcm.len() && !play_once {
                            cursor = 0;
                        }
                    }
                }
            }
            match tokio::time::timeout(Duration::from_secs(4), client.send_audio(&frame)).await {
                Ok(Ok(())) => {
                    let mut observed = observed.lock().unwrap();
                    observed.phases[index].sent_frames += 1;
                    observed.phases[index].sent_bytes += frame_bytes as u64;
                }
                Ok(Err(error)) => {
                    observed
                        .lock()
                        .unwrap()
                        .fail_connection("send", &error, started);
                    break;
                }
                Err(_) => {
                    observed
                        .lock()
                        .unwrap()
                        .fail("send", "send_timeout", started);
                    break;
                }
            }
        }
        if observed.lock().unwrap().first_failure.is_none() {
            tokio::time::sleep_until(stage_started + frame_time.mul_f64(frames as f64)).await;
        }
        println!(
            "{}",
            json!({"phase":PHASES[index],"boundary":"end","elapsed_ms":started.elapsed().as_millis(),
            "counts":observed.lock().unwrap().phases[index]})
        );
    }
    health.abort();
    let _ = health.await;
    phase.store(3, Ordering::SeqCst);
    closing.store(true, Ordering::SeqCst);
    let finish_started = Instant::now();
    let can_finish = observed.lock().unwrap().first_failure.is_none();
    let finish = async {
        match (&client, openai_finish_override) {
            (TranslationClient::OpenAIRealtime(openai), Some(timeout)) => {
                openai.finish(timeout).await
            }
            _ => client.finish().await,
        }
    };
    if can_finish
        && tokio::time::timeout(
            openai_finish_override.unwrap_or(client.finish_timeout()) + Duration::from_secs(1),
            finish,
        )
        .await
        .is_err()
    {
        observed
            .lock()
            .unwrap()
            .fail("finish", "finish_timeout", started);
    }
    let finish_ms = finish_started.elapsed().as_millis();
    if tokio::time::timeout(DISCONNECT_TIMEOUT, client.disconnect())
        .await
        .is_err()
    {
        observed
            .lock()
            .unwrap()
            .fail("disconnect", "disconnect_timeout", started);
    }
    // Drain already queued reliable finals after the client's bounded finish.
    let _ = stop_reader.send(());
    if tokio::time::timeout(Duration::from_secs(1), &mut reader)
        .await
        .is_err()
    {
        reader.abort();
        observed
            .lock()
            .unwrap()
            .fail("event_drain", "event_drain_timeout", started);
    }
    development_debug::set_sink(None)?;
    if !counter_sink.wait_idle(Duration::from_secs(1)) {
        return Err("probe_counter_drain_timeout");
    }
    let mut observed = observed.lock().unwrap();
    if observed.first_failure.is_none() {
        for pair in [[0, 1], [2, 3]] {
            if pair
                .iter()
                .map(|&i| observed.phases[i].source_finals)
                .sum::<u64>()
                == 0
            {
                observed.fail("verification", "source_final_missing", started);
            } else if configuration.target_language.translates_audio()
                && pair
                    .iter()
                    .map(|&i| observed.phases[i].translation_finals)
                    .sum::<u64>()
                    == 0
            {
                observed.fail("verification", "translation_final_missing", started);
            }
        }
        if observed.session_finished == 0 {
            observed.fail("verification", "finish_event_missing", started);
        }
    }
    println!(
        "{}",
        json!({"phase":"result","profile_id":profile_id,"provider":configuration.provider,
        "route":route,"elapsed_ms":started.elapsed().as_millis(),"finish_ms":finish_ms,"observation":&*observed,
        "independent_mt_requests":mt_requests.load(Ordering::SeqCst),"same_language_finals":same_language_finals.load(Ordering::SeqCst)})
    );
    if observed.first_failure.is_some() {
        Err("public_audio_probe_failed")
    } else {
        Ok(())
    }
}

#[test]
fn manual_language_selection_does_not_guess_or_replace_requested_languages() {
    assert_eq!(
        requested_source_language(None, true),
        Ok(SourceLanguage::Automatic)
    );
    assert_eq!(
        requested_source_language(None, false),
        Ok(SourceLanguage::English)
    );
    for (code, expected) in [
        ("zh", SourceLanguage::Chinese),
        ("ja", SourceLanguage::Japanese),
        ("auto", SourceLanguage::Automatic),
    ] {
        assert_eq!(requested_source_language(Some(code), false), Ok(expected));
    }
    assert!(requested_source_language(Some(""), true).is_err());
    assert!(requested_source_language(Some("unrecognized"), true).is_err());
}

#[tokio::test]
#[ignore = "paid public-audio service check: explicit authorization, explicitly selected saved profile and verified public PCM required"]
async fn manual_provider_public_audio_probe() {
    let result = run().await;
    if let Err(label) = result {
        println!("{}", json!({"phase":"probe_failure","code":label}));
    }
    assert!(result.is_ok(), "public_audio_probe_failed");
}
