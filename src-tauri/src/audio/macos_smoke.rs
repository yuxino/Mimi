//! Explicit native permission/capture acceptance under the canonical dev identity.
//! No product settings, credentials, providers, microphone, or recordings are used.

use super::macos::{MacSystemAudioCapture, MainThreadDispatcher};
use super::send_pipeline::AudioSendPipeline;
use super::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::system_audio_target::SystemAudioTarget;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Suite {
    Smoke,
    Stress,
    Silence,
    Cancel,
    MissingApplication,
    OwnPlayback,
    FailedStart,
    Application,
    ApplicationExit,
}

pub(crate) struct Options {
    output: PathBuf,
    backend: Option<bool>,
    suite: Suite,
}

pub(crate) fn options_from_args() -> Option<Options> {
    let args: Vec<_> = std::env::args().collect();
    let index = args.iter().position(|arg| arg == "--native-audio-smoke")?;
    let output = PathBuf::from(
        args.get(index + 1)
            .expect("native smoke output path required"),
    );
    assert!(
        output.is_absolute(),
        "native smoke requires an absolute output path"
    );
    let backend = match args.get(index + 2).map(String::as_str) {
        Some("tap") => Some(true),
        Some("legacy") => Some(false),
        Some("auto") | None => None,
        _ => panic!("native smoke backend must be auto, tap, or legacy"),
    };
    let suite = match args.get(index + 3).map(String::as_str) {
        Some("smoke") | None => Suite::Smoke,
        Some("stress") => Suite::Stress,
        Some("silence") => Suite::Silence,
        Some("cancel") => Suite::Cancel,
        Some("missing-application") => Suite::MissingApplication,
        Some("own-playback") => Suite::OwnPlayback,
        Some("failed-start") => Suite::FailedStart,
        Some("application") => Suite::Application,
        Some("application-exit") => Suite::ApplicationExit,
        _ => panic!("unknown native smoke suite"),
    };
    assert!(
        !matches!(
            suite,
            Suite::MissingApplication | Suite::Application | Suite::ApplicationExit
        ) || backend.is_none()
    );
    Some(Options {
        output,
        backend,
        suite,
    })
}

fn target(suite: Suite, cycle: usize) -> SystemAudioTarget {
    if matches!(suite, Suite::Application | Suite::ApplicationExit) && cycle == 0 {
        SystemAudioTarget::Application {
            id: "com.apple.QuickTimePlayerX".into(),
            name: "Synthetic playback source".into(),
        }
    } else {
        SystemAudioTarget::System
    }
}

pub(crate) fn run(mut context: tauri::Context<tauri::Wry>, options: Options) {
    assert_eq!(context.config().identifier, "app.yuxino.mimi.dev");
    assert_ne!(std::env::var("MIMI_UI_TEST").as_deref(), Ok("1"));
    // No product WebViews or settings store are constructed in this mode.
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .build(context)
        .expect("native audio smoke runtime");
    let mut options = Some(options);
    app.run(move |app, event| {
        if !matches!(event, tauri::RunEvent::Ready) {
            return;
        }
        let Some(options) = options.take() else {
            return;
        };
        let app = app.clone();
        let dispatcher_app = app.clone();
        let dispatcher: MainThreadDispatcher = Arc::new(move |task| {
            dispatcher_app
                .run_on_main_thread(task)
                .expect("native smoke main thread");
        });
        tauri::async_runtime::spawn(async move {
            let result = capture(dispatcher, &options).await;
            let code = if result.is_ok() { 0 } else { 1 };
            if let Err(error) = result {
                eprintln!("native smoke failed: {error}");
            }
            app.exit(code);
        });
    });
}

#[derive(Default)]
struct Metrics {
    buffers: AtomicU64,
    bytes: AtomicU64,
    audible: AtomicU64,
    peak: AtomicU64,
    first_buffer_us: AtomicU64,
    failure: Mutex<Option<&'static str>>,
}

impl Metrics {
    fn pipeline(self: &Arc<Self>) -> AudioSendPipeline {
        let counters = self.clone();
        let failure = self.clone();
        let started = Instant::now();
        AudioSendPipeline::spawn(
            move |pcm| {
                let _ = counters.first_buffer_us.compare_exchange(
                    0,
                    started.elapsed().as_micros() as u64 + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
                counters.buffers.fetch_add(1, Ordering::SeqCst);
                counters.bytes.fetch_add(pcm.len() as u64, Ordering::SeqCst);
                for sample in pcm.chunks_exact(2) {
                    let value =
                        i32::from(i16::from_le_bytes([sample[0], sample[1]])).unsigned_abs() as u64;
                    if value > 32 {
                        counters.audible.fetch_add(1, Ordering::SeqCst);
                    }
                    counters.peak.fetch_max(value, Ordering::SeqCst);
                }
                async { Ok::<(), ()>(()) }
            },
            move |value| {
                *failure.failure.lock().unwrap() = Some(value.diagnostic_label());
            },
        )
    }
}

fn write(output: &mut std::fs::File, value: serde_json::Value) -> Result<(), String> {
    writeln!(output, "{value}").map_err(|error| error.to_string())
}

async fn capture(dispatcher: MainThreadDispatcher, options: &Options) -> Result<(), String> {
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&options.output)
        .map_err(|error| error.to_string())?;
    let screen_authorized = core_graphics2::window::preflight_screen_capture_access();
    let tap_available = super::macos_tap::is_available();
    let selected_tap = options.backend.unwrap_or(super::macos_tap::use_audio_tap(
        &target(options.suite, 0),
        tap_available,
        screen_authorized,
    ));
    write(
        &mut output,
        serde_json::json!({"phase":"starting", "screenAuthorized":screen_authorized,
        "tapAvailable":tap_available, "backend":if selected_tap { "tap" } else { "legacy" }}),
    )?;
    let capture = MacSystemAudioCapture::for_smoke(dispatcher, options.backend);
    let metrics = Arc::new(Metrics::default());
    if options.suite == Suite::Cancel {
        return cancellation(&capture, &metrics, &mut output).await;
    }
    if options.suite == Suite::FailedStart {
        return failed_start(&capture, &metrics, &mut output).await;
    }
    if options.suite == Suite::MissingApplication {
        let pipeline = metrics.pipeline();
        let (failure, _) = CaptureFailureSender::channel();
        let error = capture
            .start(
                pipeline.ingress().unwrap(),
                failure,
                AudioCaptureFormat::pcm16_mono(16_000).unwrap(),
                SystemAudioTarget::Application {
                    id: "app.yuxino.mimi.smoke.missing-source".into(),
                    name: "Missing smoke source".into(),
                },
            )
            .await
            .err();
        let idle = capture.wait_until_idle().await.is_ok();
        let drained = pipeline.finish(Duration::from_secs(2)).await;
        write(
            &mut output,
            serde_json::json!({"phase":"missing_application", "error":error.as_ref().map(ToString::to_string),
            "idle":idle,"drained":drained,"buffers":metrics.buffers.load(Ordering::SeqCst)}),
        )?;
        if error != Some(SystemAudioCaptureError::ApplicationUnavailable)
            || !idle
            || !drained
            || metrics.buffers.load(Ordering::SeqCst) != 0
        {
            return Err("missing application fell back or failed to clean up".into());
        }
    }
    let own_playback = if options.suite == Suite::OwnPlayback {
        Some(own_playback()?)
    } else {
        None
    };
    let cycles = if options.suite == Suite::ApplicationExit {
        1
    } else if options.suite == Suite::Stress {
        12
    } else {
        2
    };
    let duration = if options.suite == Suite::Stress {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(6)
    };
    let expect_silence = matches!(options.suite, Suite::Silence | Suite::OwnPlayback);
    for cycle in 0..cycles {
        let rate = if cycle % 2 == 0 { 16_000 } else { 24_000 };
        metrics.first_buffer_us.store(0, Ordering::SeqCst);
        let pipeline = metrics.pipeline();
        let (failure, mut failure_rx) = CaptureFailureSender::channel();
        let before_buffers = metrics.buffers.load(Ordering::SeqCst);
        let before_audible = metrics.audible.load(Ordering::SeqCst);
        if let Err(error) = capture
            .start(
                pipeline.ingress().unwrap(),
                failure,
                AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                target(options.suite, cycle),
            )
            .await
        {
            write(
                &mut output,
                serde_json::json!({"phase":"start_failed", "rate":rate, "error":error.to_string()}),
            )?;
            pipeline.stop();
            return Err(error.to_string());
        }
        if options.suite == Suite::Stress {
            let (failure, _) = CaptureFailureSender::channel();
            let duplicate = capture
                .clone()
                .start(
                    pipeline.ingress().unwrap(),
                    failure,
                    AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                    SystemAudioTarget::System,
                )
                .await;
            if duplicate != Err(SystemAudioCaptureError::AlreadyRunning) {
                return Err("duplicate native capture accepted".into());
            }
        }
        write(
            &mut output,
            serde_json::json!({"phase":"capturing", "rate":rate,"cycle":cycle}),
        )?;
        let application_exited = if options.suite == Suite::ApplicationExit {
            let failure = tokio::time::timeout(Duration::from_secs(20), failure_rx.recv()).await;
            if !matches!(
                failure,
                Ok(Some(SystemAudioCaptureFailure::ApplicationUnavailable))
            ) {
                capture.stop().await;
                return Err("application exit did not report source unavailable".into());
            }
            true
        } else {
            false
        };
        if !application_exited {
            tokio::select! {
                _ = tokio::time::sleep(duration) => {},
                failure = failure_rx.recv() => { *metrics.failure.lock().unwrap() = failure.map(|value| value.diagnostic_label()); }
            }
        }
        capture.clone().stop().await;
        capture.stop().await;
        let idle = capture.wait_until_idle().await.is_ok();
        // Leave ingress open: stopping only the send pipeline could conceal ghost capture.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let stopped_buffers = metrics.buffers.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let post_stop_buffers = metrics.buffers.load(Ordering::SeqCst) - stopped_buffers;
        let drained = pipeline.finish(Duration::from_secs(2)).await;
        let buffers = metrics.buffers.load(Ordering::SeqCst) - before_buffers;
        let audible = metrics.audible.load(Ordering::SeqCst) - before_audible;
        write(
            &mut output,
            serde_json::json!({"phase":"stopped", "rate":rate,"cycle":cycle,
            "buffers":buffers,"audibleSamples":audible,"peak":metrics.peak.load(Ordering::SeqCst),
            "firstBufferMs":metrics.first_buffer_us.load(Ordering::SeqCst).checked_sub(1).map(|value| value / 1_000),
            "postStopBuffers":post_stop_buffers,"idle":idle,"drained":drained,"applicationExited":application_exited,
            "failure":*metrics.failure.lock().unwrap()}),
        )?;
        if !idle
            || !drained
            || buffers == 0
            || post_stop_buffers != 0
            || (expect_silence && audible != 0)
            || (!expect_silence && audible == 0)
            || metrics.failure.lock().unwrap().is_some()
        {
            return Err("native capture/stop acceptance failed".into());
        }
    }
    if let Some(playback) = own_playback {
        let frames = playback.frames.load(Ordering::SeqCst);
        let failed = playback.failed.load(Ordering::SeqCst);
        write(
            &mut output,
            serde_json::json!({"phase":"own_playback", "outputFrames":frames,"failed":failed}),
        )?;
        if frames == 0 || failed {
            return Err("own playback control failed".into());
        }
    }
    write(
        &mut output,
        serde_json::json!({"phase":"passed", "cycles":cycles,"totalBytes":metrics.bytes.load(Ordering::SeqCst)}),
    )
}

async fn cancellation(
    capture: &MacSystemAudioCapture,
    metrics: &Arc<Metrics>,
    output: &mut std::fs::File,
) -> Result<(), String> {
    for delay_ms in [0, 1, 5, 20, 100, 250] {
        let pipeline = metrics.pipeline();
        let (failure, _) = CaptureFailureSender::channel();
        let ingress = pipeline.ingress().unwrap();
        let starting = capture.clone();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = entered.send(());
            starting
                .start(
                    ingress,
                    failure,
                    AudioCaptureFormat::pcm16_mono(16_000).unwrap(),
                    SystemAudioTarget::System,
                )
                .await
        });
        ready.await.map_err(|_| "cancellation task lost")?;
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        capture.stop().await;
        let result = tokio::time::timeout(Duration::from_secs(12), task)
            .await
            .map_err(|_| "canceled start did not return")?
            .map_err(|_| "cancellation task failed")?;
        let idle = capture.wait_until_idle().await.is_ok();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let before = metrics.buffers.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let ghost = metrics.buffers.load(Ordering::SeqCst) - before;
        let drained = pipeline.finish(Duration::from_secs(2)).await;
        write(
            output,
            serde_json::json!({"phase":"cancelled", "delayMs":delay_ms,"result":result.as_ref().err().map(ToString::to_string),
            "idle":idle,"drained":drained,"postStopBuffers":ghost}),
        )?;
        if !matches!(
            result,
            Ok(()) | Err(SystemAudioCaptureError::StartCancelled)
        ) || !idle
            || !drained
            || ghost != 0
        {
            return Err("startup cancellation acceptance failed".into());
        }
    }
    write(
        output,
        serde_json::json!({"phase":"passed", "cancellations":6}),
    )
}

async fn failed_start(
    capture: &MacSystemAudioCapture,
    metrics: &Arc<Metrics>,
    output: &mut std::fs::File,
) -> Result<(), String> {
    let pipeline = metrics.pipeline();
    let (failure, _) = CaptureFailureSender::channel();
    let first = capture
        .start(
            pipeline.ingress().unwrap(),
            failure,
            AudioCaptureFormat::pcm16_mono(16_000).unwrap(),
            SystemAudioTarget::System,
        )
        .await;
    if first.is_ok() {
        capture.stop().await;
        return Err("failed-start case unexpectedly captured audio".into());
    }
    let idle = capture.wait_until_idle().await.is_ok();
    let (failure, _) = CaptureFailureSender::channel();
    let retry = capture
        .start(
            pipeline.ingress().unwrap(),
            failure,
            AudioCaptureFormat::pcm16_mono(24_000).unwrap(),
            SystemAudioTarget::System,
        )
        .await;
    capture.stop().await;
    let drained = pipeline.finish(Duration::from_secs(2)).await;
    let buffers = metrics.buffers.load(Ordering::SeqCst);
    write(
        output,
        serde_json::json!({"phase":"failed_start", "first":first.err().map(|error| error.to_string()),
        "idle":idle,"retry":retry.as_ref().err().map(ToString::to_string),"drained":drained,"buffers":buffers}),
    )?;
    if buffers != 0
        || !drained
        || retry.is_ok()
        || (!idle && retry != Err(SystemAudioCaptureError::PreviousCaptureStopping))
    {
        return Err("failed start allowed overlapping or fallback capture".into());
    }
    write(
        output,
        serde_json::json!({"phase":"passed", "failedStartStayedClosed":true}),
    )
}

struct OwnPlayback {
    _stream: cpal::Stream,
    frames: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
}

fn own_playback() -> Result<OwnPlayback, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output device")?;
    let supported = device
        .default_output_config()
        .map_err(|_| "output config unavailable")?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err("smoke playback requires float output".into());
    }
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let sample_rate = f64::from(config.sample_rate);
    let mut phase = 0.0_f64;
    let frames = Arc::new(AtomicU64::new(0));
    let output_frames = frames.clone();
    let failed = Arc::new(AtomicBool::new(false));
    let output_failed = failed.clone();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| {
                output_frames.fetch_add((data.len() / channels) as u64, Ordering::SeqCst);
                for frame in data.chunks_mut(channels) {
                    let sample = phase.sin() as f32 * 0.025;
                    phase = (phase + std::f64::consts::TAU * 440.0 / sample_rate)
                        % std::f64::consts::TAU;
                    frame.fill(sample);
                }
            },
            move |_| {
                output_failed.store(true, Ordering::SeqCst);
            },
            None,
        )
        .map_err(|_| "smoke playback creation failed")?;
    stream.play().map_err(|_| "smoke playback start failed")?;
    Ok(OwnPlayback {
        _stream: stream,
        frames,
        failed,
    })
}
