//! Explicit native permission/capture acceptance under the canonical dev identity.
//! No product settings, credentials, providers, microphone, or recordings are used.

use super::macos::{MacSystemAudioCapture, MainThreadDispatcher};
use super::send_pipeline::AudioSendPipeline;
use super::{AudioCaptureFormat, CaptureFailureSender};
use crate::core::system_audio_target::SystemAudioTarget;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) struct Options {
    output: PathBuf,
    backend: Option<bool>,
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
    Some(Options { output, backend })
}

pub(crate) fn run(mut context: tauri::Context<tauri::Wry>, options: Options) {
    assert_eq!(context.config().identifier, "app.yuxino.mimi.dev");
    assert_ne!(std::env::var("MIMI_UI_TEST").as_deref(), Ok("1"));
    // No product WebViews or settings store are constructed in this mode.
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .setup(move |app| {
            let app = app.handle().clone();
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
            Ok(())
        })
        .run(context)
        .expect("native audio smoke runtime");
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
        &SystemAudioTarget::System,
        tap_available,
        screen_authorized,
    ));
    writeln!(
        output,
        "{}",
        serde_json::json!({"phase":"starting", "screenAuthorized":screen_authorized,
        "tapAvailable":tap_available, "backend":if selected_tap { "tap" } else { "legacy" }})
    )
    .map_err(|error| error.to_string())?;
    let capture = MacSystemAudioCapture::for_smoke(dispatcher, options.backend);
    // Atomic metrics only. PCM is dropped after each bounded send operation.
    let buffers = Arc::new(AtomicU64::new(0));
    let bytes = Arc::new(AtomicU64::new(0));
    let nonzero = Arc::new(AtomicU64::new(0));
    let peak = Arc::new(AtomicU64::new(0));
    let failure_label = Arc::new(Mutex::new(None::<&'static str>));
    for rate in [16_000, 24_000] {
        let counters = (
            buffers.clone(),
            bytes.clone(),
            nonzero.clone(),
            peak.clone(),
        );
        let pipeline_failure = failure_label.clone();
        let pipeline = AudioSendPipeline::spawn(
            move |pcm| {
                counters.0.fetch_add(1, Ordering::SeqCst);
                counters.1.fetch_add(pcm.len() as u64, Ordering::SeqCst);
                for sample in pcm.chunks_exact(2) {
                    let value =
                        i32::from(i16::from_le_bytes([sample[0], sample[1]])).unsigned_abs() as u64;
                    if value > 32 {
                        counters.2.fetch_add(1, Ordering::SeqCst);
                    }
                    counters.3.fetch_max(value, Ordering::SeqCst);
                }
                async { Ok::<(), ()>(()) }
            },
            move |failure| {
                *pipeline_failure.lock().unwrap() = Some(failure.diagnostic_label());
            },
        );
        let (failure, mut failure_rx) = CaptureFailureSender::channel();
        let before_buffers = buffers.load(Ordering::SeqCst);
        let before_nonzero = nonzero.load(Ordering::SeqCst);
        let result = capture
            .start(
                pipeline.ingress().unwrap(),
                failure,
                AudioCaptureFormat::pcm16_mono(rate).unwrap(),
                SystemAudioTarget::System,
            )
            .await;
        if let Err(error) = result {
            writeln!(
                output,
                "{}",
                serde_json::json!({"phase":"start_failed", "rate":rate, "error":error.to_string()})
            )
            .map_err(|error| error.to_string())?;
            pipeline.stop();
            return Err(error.to_string());
        }
        writeln!(
            output,
            "{}",
            serde_json::json!({"phase":"capturing", "rate":rate})
        )
        .map_err(|error| error.to_string())?;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(6)) => {},
            failure = failure_rx.recv() => {
                *failure_label.lock().unwrap() = failure.map(|failure| failure.diagnostic_label());
            }
        }
        capture.stop().await;
        let idle = capture.wait_until_idle().await.is_ok();
        let drained = pipeline.finish(Duration::from_secs(2)).await;
        let captured_buffers = buffers.load(Ordering::SeqCst) - before_buffers;
        let audible_samples = nonzero.load(Ordering::SeqCst) - before_nonzero;
        writeln!(output, "{}", serde_json::json!({"phase":"stopped", "rate":rate,
            "buffers":captured_buffers,"audibleSamples":audible_samples,"peak":peak.load(Ordering::SeqCst),
            "idle":idle,"drained":drained,"failure":*failure_label.lock().unwrap()})).map_err(|error| error.to_string())?;
        if !idle
            || !drained
            || captured_buffers == 0
            || audible_samples == 0
            || failure_label.lock().unwrap().is_some()
        {
            return Err("native capture/stop acceptance failed".into());
        }
    }
    writeln!(
        output,
        "{}",
        serde_json::json!({"phase":"passed", "totalBytes":bytes.load(Ordering::SeqCst)})
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
