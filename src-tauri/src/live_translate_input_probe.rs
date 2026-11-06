//! Explicit, bounded real-provider experiment. No native capture or persistence.
//! Reuses the production client, two send queues, DSP and subtitle controller.

use crate::audio::echo_pipeline::EchoPipeline;
use crate::audio::send_pipeline::{AudioIngress, AudioSendPipeline};
use crate::audio::CaptureFailureSender;
use crate::clients::provider_events::provider_event_channel;
use crate::clients::translation_client::TranslationClient;
use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use crate::core::protocols::live_translate::LiveTranslateServerEvent as Event;
use crate::core::provider::{AlibabaRealtimeMode, ProviderKind};
use crate::core::session::TranslationSessionController;
use crate::core::support_diagnostics::SafeFailure;
use crate::settings_store::{SettingsStore, DEVELOPMENT_APPLICATION_IDENTIFIER};
use mimi_runtime::core::health::{probe_connection_health, HEALTH_CHECK_INTERVAL};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod soak;

#[derive(Default, serde::Serialize)]
struct Counts {
    events: u64,
    content: u64,
    pairs: u64,
    last_content_ms: Option<u64>,
    failure: Option<String>,
    source_events: u64,
    translation_events: u64,
    cyrillic_source_events: u64,
    han_translation_events: u64,
    first_source_ms: Option<u64>,
    first_translation_ms: Option<u64>,
    last_translation_ms: Option<u64>,
    follow_latency_ms: Vec<u64>,
    session_finished: bool,
    health_checks: u64,
    maximum_health_ms: u64,
}

impl Counts {
    fn source_text(&mut self, text: &str, at: u64) {
        if text.is_empty() {
            return;
        }
        self.source_events += 1;
        self.first_source_ms.get_or_insert(at);
        if text.chars().any(|c| ('\u{0400}'..='\u{04ff}').contains(&c)) {
            self.cyrillic_source_events += 1;
        }
    }

    fn translation_text(&mut self, text: &str, at: u64) {
        if text.is_empty() {
            return;
        }
        self.translation_events += 1;
        self.first_translation_ms.get_or_insert(at);
        self.last_translation_ms = Some(at);
        if text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
            self.han_translation_events += 1;
        }
    }

    fn observe_text(&mut self, event: &Event, at: u64) {
        match event {
            Event::SourceDraft { text, .. } | Event::SourceFinal { text, .. } => {
                self.source_text(text, at)
            }
            Event::TranslationDraft(text) | Event::TranslationFinal(text) => {
                self.translation_text(text, at)
            }
            Event::UtteranceText {
                text,
                role: UtteranceRole::Source,
                ..
            } => self.source_text(text, at),
            Event::UtteranceText {
                text,
                role: UtteranceRole::Translation,
                ..
            } => self.translation_text(text, at),
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
            }
            | Event::SubtitlePreviewPair {
                source,
                translation,
                ..
            } => {
                self.source_text(source, at);
                self.translation_text(translation, at);
            }
            _ => {}
        }
    }
}

struct Lane {
    source: AudioSource,
    client: TranslationClient,
    pipeline: AudioSendPipeline,
    ingress: AudioIngress,
    counts: Arc<Mutex<Counts>>,
    bytes: Arc<AtomicU64>,
    closing: Arc<AtomicBool>,
    events: tokio::task::JoinHandle<()>,
}

struct CaseOutcome {
    offset: usize,
    failure: Option<String>,
}

async fn run_case(
    configuration: &LiveTranslationConfiguration,
    pcm: &[u8],
    input: AudioInput,
    seconds: u64,
    name: &str,
    repeat: bool,
) -> bool {
    use mimi_runtime::core::recovery::{
        provider_error_is_retryable, provider_recovery_minimum_delay, RecoverySchedule,
    };
    let recover = std::env::var("MIMI_INPUT_PROBE_RECOVER").as_deref() == Ok("1");
    assert!(
        !recover || !repeat,
        "recovery comparison requires a finite fixed sample"
    );
    let started = Instant::now();
    let mut offset = 0;
    let mut recovery = None;
    loop {
        let outcome = observe_case(
            configuration,
            &pcm[offset..],
            input,
            seconds,
            name,
            repeat,
            None,
        )
        .await;
        let Some(code) = outcome.failure else {
            return true;
        };
        if !recover || !provider_error_is_retryable(&code) {
            return false;
        }
        let schedule = recovery.get_or_insert_with(|| {
            RecoverySchedule::new(1, 2, provider_recovery_minimum_delay(&code))
        });
        let Some(attempt) = schedule.next_attempt() else {
            return false;
        };
        offset = (offset + outcome.offset).min(pcm.len());
        if offset == pcm.len() {
            return false;
        }
        println!(
            "{}",
            json!({"case":name,"phase":"probe_recovery","attempt":attempt.index+1,"code":code,"at_ms":started.elapsed().as_millis(),"delay_ms":attempt.delay.as_millis(),"remaining_bytes":pcm.len()-offset})
        );
        // Reuse the production classification/backoff and client. This is a
        // PCM experiment, not proof of OS-capture or desktop-manager recovery.
        tokio::time::sleep(attempt.delay).await;
    }
}

async fn observe_case(
    configuration: &LiveTranslationConfiguration,
    pcm: &[u8],
    input: AudioInput,
    seconds: u64,
    name: &str,
    repeat: bool,
    playback: Option<soak::Playback<'_>>,
) -> CaseOutcome {
    let started = Instant::now();
    let observe_healthy_lane_to_end =
        std::env::var("MIMI_INPUT_PROBE_OBSERVE_HEALTHY_TO_END").as_deref() == Ok("1");
    let recover =
        playback.is_some() || std::env::var("MIMI_INPUT_PROBE_RECOVER").as_deref() == Ok("1");
    let realistic = std::env::var("MIMI_INPUT_PROBE_REALISTIC").as_deref() == Ok("1");
    assert!(
        !(realistic && recover),
        "keep pattern timing comparable across attempts"
    );
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    controller.lock().unwrap().set_audio_input(input);
    let (capture_errors, mut failures) = CaptureFailureSender::channel();
    let echo =
        (input == AudioInput::Both).then(|| EchoPipeline::spawn(16_000, capture_errors).unwrap());
    let mut lanes = Vec::new();
    for &source in input.sources() {
        let (tx, mut rx) = provider_event_channel();
        tx.set_debug_context(source, 1);
        let client =
            TranslationClient::new(configuration, tx).expect("validated probe configuration");
        let counts = Arc::new(Mutex::new(Counts::default()));
        let observed = Arc::clone(&counts);
        let closing = Arc::new(AtomicBool::new(false));
        let event_closing = Arc::clone(&closing);
        let event_controller = Arc::clone(&controller);
        let event_client = client.clone();
        let events = tokio::spawn(async move {
            while let Some(envelope) = rx.recv_with_revision().await {
                let now = Instant::now();
                let mut observed = observed.lock().unwrap();
                observed.events += 1;
                if envelope.is_content() {
                    observed.content += 1;
                    observed.last_content_ms = Some(started.elapsed().as_millis() as u64);
                }
                observed.observe_text(&envelope.event, started.elapsed().as_millis() as u64);
                match &envelope.event {
                    Event::Error { code, .. } => {
                        // Decoded production codes only; no messages or text.
                        println!(
                            "{}",
                            json!({"phase":"provider_failure","source":source,"at_ms":started.elapsed().as_millis(),"code":mimi_runtime::core::recovery::normalize_provider_error_code(code),"retryable":mimi_runtime::core::recovery::provider_error_is_retryable(code)})
                        );
                        observed.failure.get_or_insert_with(|| code.clone());
                    }
                    Event::SessionFinished => {
                        observed.session_finished = true;
                        if !event_closing.load(Ordering::SeqCst) {
                            observed
                                .failure
                                .get_or_insert_with(|| "unexpected_finish".into());
                        }
                    }
                    Event::SubtitleConfirmedPair { .. }
                    | Event::SubtitleIdentifiedFinalPair { .. }
                    | Event::SubtitleFinalPair { .. } => {
                        observed.pairs += 1;
                        if let Some(latency) = event_client.translation_latency() {
                            if observed.follow_latency_ms.len() < 512 {
                                observed.follow_latency_ms.push(latency.milliseconds);
                            }
                        }
                    }
                    _ => {}
                }
                drop(observed);
                envelope.apply_to_at(&mut event_controller.lock().unwrap(), source, now);
            }
        });
        if let Err(error) = client.connect().await {
            // Never print provider objects or credential-bearing configuration.
            println!(
                "{}",
                json!({"case":name,"phase":"connect","source":source,"error":error.diagnostic_label()})
            );
            client.disconnect().await;
            events.abort();
            for lane in &lanes {
                let lane: &Lane = lane;
                lane.pipeline.stop();
                lane.client.disconnect().await;
                lane.events.abort();
            }
            return CaseOutcome {
                offset: 0,
                failure: Some(error.safe_code().into()),
            };
        }
        let sender = client.clone();
        let bytes = Arc::new(AtomicU64::new(0));
        let sent_bytes = Arc::clone(&bytes);
        let failed = Arc::clone(&counts);
        let pipeline = AudioSendPipeline::spawn_with_error_handler(
            move |data| {
                let sender = sender.clone();
                let sent_bytes = Arc::clone(&sent_bytes);
                async move {
                    sender.send_audio(&data).await?;
                    sent_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                    Ok::<_, crate::clients::translation_client::ConnectError>(())
                }
            },
            move |error: crate::clients::translation_client::ConnectError| {
                println!(
                    "{}",
                    json!({"phase":"send_failure","source":source,"classification":SafeFailure::from_error(&error.to_string())})
                );
                failed
                    .lock()
                    .unwrap()
                    .failure
                    .get_or_insert_with(|| "send_failed".into());
            },
        );
        let ingress = pipeline.ingress().unwrap();
        let ingress = match &echo {
            Some(echo) => echo.ingress(source, ingress),
            None => ingress,
        };
        lanes.push(Lane {
            source,
            client,
            pipeline,
            ingress,
            counts,
            bytes,
            closing,
            events,
        });
    }
    controller.lock().unwrap().did_connect();
    println!(
        "{}",
        json!({"case":name,"phase":"ready","lanes":lanes.len(),"seconds":seconds,"sample_rate":16_000})
    );
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let health_lanes: Vec<_> = lanes
        .iter()
        .map(|lane| (lane.client.clone(), Arc::clone(&lane.counts)))
        .collect();
    let health = tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEALTH_CHECK_INTERVAL);
        loop {
            interval.tick().await;
            // Independent of native callback pacing, as in the real manager.
            for (client, counts) in &health_lanes {
                if counts.lock().unwrap().failure.is_some() {
                    continue;
                }
                match probe_connection_health(client).await {
                    Ok(elapsed) => {
                        let mut counts = counts.lock().unwrap();
                        counts.health_checks += 1;
                        counts.maximum_health_ms =
                            counts.maximum_health_ms.max(elapsed.as_millis() as u64);
                    }
                    Err(error) => {
                        counts
                            .lock()
                            .unwrap()
                            .failure
                            .get_or_insert_with(|| error.safe_code().into());
                    }
                }
            }
        }
    });
    let mut report = tokio::time::interval(Duration::from_secs(30));
    let streaming = Instant::now();
    let mut offset = 0;
    let mut failed_at = None;
    let mut last_tick = Instant::now();
    let mut maximum_tick_gap_ms = 0;
    let mut input_queued_ms = None;
    let mut capture_failure = None;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                maximum_tick_gap_ms = maximum_tick_gap_ms.max(last_tick.elapsed().as_millis());
                last_tick = Instant::now();
                if playback.is_some_and(|p|p.complete()) { break; }
                if playback.is_none() && repeat && streaming.elapsed().as_secs() >= seconds { break; }
                if playback.is_none() && !repeat && offset >= pcm.len() + 320_000 { break; }
                if playback.is_none() && !repeat && streaming.elapsed().as_secs() >= seconds + 30 {
                    println!("{}",json!({"case":name,"phase":"pacing_deadline","offset":offset}));
                    capture_failure = Some("probe_pacing_deadline".into());
                    break;
                }
                if let Some(at) = failed_at {
                    // Observe whether the unaffected lane still produces text.
                    if !observe_healthy_lane_to_end
                        && Instant::now().duration_since(at) >= Duration::from_secs(10) { break; }
                }
                for lane in &lanes {
                    if lane.counts.lock().unwrap().failure.is_some() { continue; }
                    let t = streaming.elapsed().as_secs();
                    let silent = if realistic {
                        match t {
                            0..120 | 360..480 | 780.. => lane.source == AudioSource::System,
                            120..240 => true,
                            240..360 | 720..780 => continue,
                            480..600 => lane.source == AudioSource::Microphone,
                            _ => false,
                        }
                    } else { input == AudioInput::Both && lane.source == AudioSource::System };
                    // After eight minutes of digital silence, model a quiet
                    // loopback device that delivers no callback at all.
                    if playback.is_none() && repeat && !realistic && silent && t >= 480 { continue; }
                    let data = if let Some(playback) = playback {
                        let Some(data) = playback.packet(lane.source) else {continue;};
                        data
                    } else if silent { vec![0;640] } else {
                        let source_offset = if realistic && lane.source == AudioSource::System { 32_000 * 30 } else { 0 };
                        (0..640).map(|i| if repeat { pcm[(offset+i+source_offset)%pcm.len()] } else { pcm.get(offset+i+source_offset).copied().unwrap_or(0) }).collect()
                    };
                    if lane.ingress.try_send(data).is_err() {
                        lane.counts.lock().unwrap().failure.get_or_insert_with(|| "ingress_failed".into());
                    }
                }
                offset = if repeat { (offset + 640) % pcm.len() } else { offset + 640 };
                if !repeat && input_queued_ms.is_none() && offset >= pcm.len() {
                    input_queued_ms = Some(started.elapsed().as_millis() as u64);
                    println!("{}",json!({"case":name,"phase":"input_queued","at_ms":input_queued_ms,"pcm_bytes":pcm.len()}));
                }
            }
            _ = report.tick() => {
                for lane in &lanes {
                    println!("{}",json!({"case":name,"phase":"progress","elapsed_ms":streaming.elapsed().as_millis(),"soak_elapsed_ms":playback.map(|p|p.elapsed_ms()),"input_phase":playback.map(|p|p.phase_name()),"source":lane.source,"sent_bytes":lane.bytes.load(Ordering::Relaxed),"counts":*lane.counts.lock().unwrap()}));
                }
            }
            Some(error) = failures.recv() => {
                println!("{}",json!({"case":name,"phase":"capture_processing_failure","elapsed_ms":streaming.elapsed().as_millis(),"error":error.diagnostic_label(),"max_tick_gap_ms":maximum_tick_gap_ms}));
                capture_failure = Some(if error.is_recoverable() { "transport_error" } else { "invalid_configuration" }.to_string());
                break;
            }
        }
        if failed_at.is_none()
            && lanes
                .iter()
                .any(|lane| lane.counts.lock().unwrap().failure.is_some())
        {
            failed_at = Some(Instant::now());
            for lane in &lanes {
                println!(
                    "{}",
                    json!({"case":name,"phase":"lane_at_failure","source":lane.source,"counts":*lane.counts.lock().unwrap(),"max_tick_gap_ms":maximum_tick_gap_ms})
                );
            }
            println!(
                "{}",
                json!({"case":name,"phase":"first_failure","elapsed_ms":streaming.elapsed().as_millis()})
            );
            if recover {
                break;
            }
        }
    }
    health.abort();
    let failed_recovery = recover && (failed_at.is_some() || capture_failure.is_some());
    if let Some(echo) = &echo {
        let drained = if failed_recovery {
            echo.stop();
            false
        } else {
            echo.finish(Duration::from_secs(1)).await
        };
        println!(
            "{}",
            json!({"case":name,"phase":"echo_drain","drained":drained})
        );
    }
    for lane in &lanes {
        lane.closing.store(true, Ordering::SeqCst);
        if failed_recovery {
            lane.pipeline.stop();
        } else {
            lane.pipeline.finish(Duration::from_secs(1)).await;
        }
        let finish_returned = if failed_recovery {
            false
        } else {
            tokio::time::timeout(
                lane.client.finish_timeout() + Duration::from_secs(1),
                lane.client.finish(),
            )
            .await
            .is_ok()
        };
        println!(
            "{}",
            json!({"case":name,"phase":"provider_finish","source":lane.source,"returned":finish_returned})
        );
        lane.client.disconnect().await;
        lane.events.abort();
        println!(
            "{}",
            json!({"case":name,"phase":"finished","elapsed_ms":streaming.elapsed().as_millis(),"input_queued_ms":input_queued_ms,"max_tick_gap_ms":maximum_tick_gap_ms,"source":lane.source,"sent_bytes":lane.bytes.load(Ordering::Relaxed),"counts":*lane.counts.lock().unwrap()})
        );
    }
    let mut codes: Vec<_> = lanes
        .iter()
        .filter_map(|lane| lane.counts.lock().unwrap().failure.clone())
        .chain(capture_failure)
        .map(|code| {
            if matches!(
                code.as_str(),
                "ingress_failed" | "send_failed" | "provider.live_translate"
            ) {
                "transport_error".into()
            } else {
                code
            }
        })
        .collect();
    codes.sort_by_key(|code| mimi_runtime::core::recovery::provider_error_is_retryable(code));
    CaseOutcome {
        offset,
        failure: codes.into_iter().next(),
    }
}

fn saved_configuration() -> LiveTranslationConfiguration {
    let directory = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support")
        .join(DEVELOPMENT_APPLICATION_IDENTIFIER);
    let settings = SettingsStore::load_for_read_only_manual_probe(directory).unwrap();
    let profile_id =
        std::env::var("MIMI_INPUT_PROBE_PROFILE").expect("explicit Dev profile required");
    let profile = settings
        .profile_catalog()
        .unwrap()
        .1
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .expect("existing Dev profile required");
    assert_eq!(profile.provider, ProviderKind::AlibabaCloud);
    assert_eq!(
        profile.alibaba_realtime_mode,
        AlibabaRealtimeMode::LiveTranslate
    );
    let mut configuration = settings
        .configuration_for_profile_probe(&profile)
        .expect("saved Dev credentials required");
    configuration.source_language = SourceLanguage::Automatic;
    configuration.target_language = TargetLanguage::SimplifiedChinese;
    let configuration = configuration.validated().unwrap();
    assert_eq!(configuration.capabilities().input_sample_rate_hz, 16_000);
    configuration
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "paid live-provider input experiment: explicit authorization, saved Dev profile and public PCM required"]
async fn manual_live_translate_input_matrix() {
    let configuration = saved_configuration();
    use mimi_runtime::core::development_debug::{self, DebugEvent, TraceSink};
    development_debug::initialize(true);
    development_debug::set_enabled(true).unwrap();
    development_debug::set_sink(Some(TraceSink::new(|entry| {
        if let DebugEvent::Pipeline { label } = entry.event {
            if ["audio queue full", "audio send blockedMs=", "audio send failed", "live translate send", "live translate receive", "live translate service error"].iter().any(|prefix|label.starts_with(prefix)) {
                println!("{}",json!({"phase":"pipeline_diagnostic","elapsed_ms":entry.elapsed_ms,"label":label}));
            }
        }
    }))).unwrap();
    if let Some(manifest) = std::env::var_os("MIMI_INPUT_PROBE_SOAK_MANIFEST") {
        let completed = soak::run(&configuration, PathBuf::from(manifest)).await;
        development_debug::set_sink(None).unwrap();
        development_debug::set_enabled(false).unwrap();
        assert!(
            completed,
            "long scenario stopped; inspect content-free failure evidence"
        );
        return;
    }
    if let Some(manifest) = std::env::var_os("MIMI_INPUT_PROBE_SPEED_MANIFEST") {
        #[derive(serde::Deserialize)]
        struct Clip {
            label: String,
            pcm_path: PathBuf,
            bytes: usize,
        }
        #[derive(serde::Deserialize)]
        struct Manifest {
            clips: Vec<Clip>,
        }
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
        assert_eq!(manifest.clips.len(), 3);
        let mut total_bytes = 0;
        for (clip, expected) in manifest.clips.iter().zip(["ru-1x", "ru-1-25x", "ru-1-5x"]) {
            assert_eq!(clip.label, expected);
            assert!((32000 * 60..=32000 * 1441).contains(&clip.bytes));
            assert!(clip.bytes.is_multiple_of(2));
            total_bytes += clip.bytes;
        }
        assert!(total_bytes <= 32000 * 3600);
        for clip in manifest.clips {
            if std::env::var("MIMI_INPUT_PROBE_SPEED_CASE")
                .is_ok_and(|selected| selected != clip.label)
            {
                continue;
            }
            let pcm = std::fs::read(clip.pcm_path).unwrap();
            assert_eq!(pcm.len(), clip.bytes);
            let seconds = (pcm.len() as u64).div_ceil(32000) + 10;
            let completed = run_case(
                &configuration,
                &pcm,
                AudioInput::Both,
                seconds,
                &clip.label,
                false,
            )
            .await;
            println!(
                "{}",
                json!({"case":clip.label,"phase":"case_outcome","completed":completed})
            );
            if !completed {
                break;
            }
        }
        development_debug::set_sink(None).unwrap();
        development_debug::set_enabled(false).unwrap();
        return;
    }
    let pcm = std::fs::read(std::env::var_os("MIMI_INPUT_PROBE_PCM").expect("public PCM required"))
        .unwrap();
    assert!(!pcm.is_empty() && pcm.len() <= 16_000 * 2 * 60 && pcm.len().is_multiple_of(2));
    let both_seconds: u64 = std::env::var("MIMI_INPUT_PROBE_BOTH_SECONDS")
        .ok()
        .map(|value| value.parse().expect("bounded duration"))
        .unwrap_or(960);
    assert!((480..=960).contains(&both_seconds));
    if let Ok(seconds) = std::env::var("MIMI_INPUT_PROBE_SHORT_BOTH_SECONDS") {
        let seconds: u64 = seconds.parse().expect("bounded short duration");
        assert!((20..=60).contains(&seconds));
        run_case(
            &configuration,
            &pcm,
            AudioInput::Both,
            seconds,
            "short_both_mic_speech_system_quiet",
            true,
        )
        .await;
        return;
    }
    // 22 minutes maximum, less if any scenario fails. No automatic repeats.
    for (input, seconds, name) in [
        (
            AudioInput::Both,
            both_seconds,
            "both_mic_speech_system_quiet",
        ),
        (AudioInput::Microphone, 180, "microphone_only"),
        (AudioInput::System, 180, "system_only"),
    ] {
        if !run_case(&configuration, &pcm, input, seconds, name, true).await {
            break;
        }
    }
}

/// Independent zero-PCM control: no DSP, provider decoder, subtitles or native capture.
/// Only protocol error identifiers are inspected; error/close prose is discarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "paid silent wire control: explicit authorization and saved Dev profile required"]
async fn manual_live_translate_silent_wire_control() {
    use futures_util::{SinkExt, StreamExt};
    use mimi_runtime::clients::provider_network::{self, ProviderNetwork};
    use mimi_runtime::core::protocols::live_translate::{
        LiveTranslateEndpoint, LiveTranslateRequestEncoder,
    };
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};
    let config = saved_configuration();
    let key = config.credentials.alibaba_key().unwrap();
    let endpoint =
        LiveTranslateEndpoint::for_mode(config.alibaba_realtime_mode, &config.alibaba_workspace_id)
            .unwrap();
    let mut request = endpoint.url.into_client_request().unwrap();
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {key}")).unwrap(),
    );
    let network = ProviderNetwork::resolve(&config.network_proxy).unwrap();
    let (mut socket, _) = provider_network::websocket_with_message_limit(request, &network, 65_536)
        .await
        .unwrap_or_else(|_| panic!("wire control connection failed"));
    let update = LiveTranslateRequestEncoder::session_update_for_mode(
        config.alibaba_realtime_mode,
        config.source_language,
        config.target_language,
        &Default::default(),
        None,
    )
    .unwrap();
    assert!(socket
        .send(Message::Text(update.to_string().into()))
        .await
        .is_ok());
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut health = tokio::time::interval(Duration::from_secs(10));
    let mut report = tokio::time::interval(Duration::from_secs(30));
    let mut ready = false;
    let mut sent = 0u64;
    let mut pongs = 0u64;
    let deadline = tokio::time::sleep(Duration::from_secs(960));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => { println!("{}",json!({"phase":"wire_deadline","at_ms":started.elapsed().as_millis()})); break; }
            _ = tick.tick(), if ready => {
                let audio = LiveTranslateRequestEncoder::audio_append(&[0;640],None).unwrap().to_string();
                if !matches!(tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(audio.into()))).await,Ok(Ok(()))) {
                    println!("{}",json!({"phase":"wire_send_failure","at_ms":started.elapsed().as_millis()})); break;
                }
                sent += 640;
            }
            _ = health.tick(), if ready => {
                if !matches!(tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Ping(Default::default()))).await,Ok(Ok(()))) { break; }
            }
            _ = report.tick() => println!("{}",json!({"phase":"wire_progress","at_ms":started.elapsed().as_millis(),"sent_bytes":sent,"pongs":pongs})),
            frame = socket.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { break; };
                    if value["type"] == "session.updated" { ready = true; }
                    if value["type"] == "error" {
                        // This zero-input control can only expose a short protocol
                        // identifier, never a body, sentence, URL or credential.
                        let identifier = |field: &str| value.pointer(field).and_then(serde_json::Value::as_str)
                            .filter(|v| !v.is_empty() && v.len() < 32 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                                && !v.contains(key) && !v.contains(&config.alibaba_workspace_id) && !v.starts_with("sk-") && !v.starts_with("llm-"))
                            .unwrap_or("redacted");
                        let message = value.pointer("/error/message").and_then(serde_json::Value::as_str).unwrap_or("").to_ascii_lowercase();
                        let terms: Vec<_> = ["timeout","idle","silence","session","duration","limit","audio","expired","response"].into_iter().filter(|term|message.contains(term)).collect();
                        println!("{}",json!({"phase":"wire_service_error","at_ms":started.elapsed().as_millis(),"code":identifier("/error/code"),"type":identifier("/error/type"),"reason_categories":terms}));
                    }
                }
                Some(Ok(Message::Pong(_))) => pongs += 1,
                Some(Ok(Message::Close(frame))) => {
                    println!("{}",json!({"phase":"wire_close","at_ms":started.elapsed().as_millis(),"code":frame.map(|f|u16::from(f.code))})); break;
                }
                Some(Ok(_)) => {},
                Some(Err(error)) => {
                    let kind = match error {
                        tokio_tungstenite::tungstenite::Error::Io(error) => format!("io_{:?}",error.kind()),
                        tokio_tungstenite::tungstenite::Error::Protocol(_) => "protocol".into(),
                        _ => "other".into(),
                    };
                    println!("{}",json!({"phase":"wire_transport_end","at_ms":started.elapsed().as_millis(),"kind":kind})); break;
                }
                None => { println!("{}",json!({"phase":"wire_transport_end","at_ms":started.elapsed().as_millis(),"kind":"eof"})); break; }
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_millis(250), socket.close(None)).await;
}
