//! Native wire adapter for the shared source-state projection.
use mimi_runtime::core::{
    audio::audio_input::AudioSource, lifecycle::session::merge_independent_states,
    lifecycle::session::TranslationSessionState, models::DetectedLanguage, models::SessionStatus,
    models::SubtitleSnapshot,
};
use serde::Deserialize;
use serde_json::{json, Value};

const INVALID: &str = "native_runtime_invalid_request";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Lane {
    audio_source: AudioSource,
    state: NativeState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeState {
    snapshot: SubtitleSnapshot,
    #[serde(default)]
    atomic_preview: Option<bool>,
    #[serde(default)]
    detected_language: Option<String>,
    #[serde(default)]
    connection_status: String,
    #[serde(default)]
    is_translation_pending: bool,
    #[serde(default)]
    is_translation_preview_pending: bool,
    #[serde(default)]
    is_translation_timed_out: bool,
    #[serde(default)]
    translation_recovery: Option<mimi_runtime::core::diagnostics::latency::TranslationRecovery>,
}

pub fn merge(value: &Value) -> Result<Value, &'static str> {
    let rows = value["snapshots"]
        .as_array()
        .filter(|rows| rows.len() <= 2)
        .ok_or(INVALID)?;
    let limit = value["historyLimit"]
        .as_u64()
        .filter(|limit| *limit <= 6)
        .ok_or(INVALID)?;
    let stopped = value["stopped"].as_bool().ok_or(INVALID)?;
    let mut lanes = Vec::with_capacity(rows.len());
    for row in rows {
        let lane: Lane = serde_json::from_value(row.clone()).map_err(|_| INVALID)?;
        if lanes.iter().any(|(source, _)| *source == lane.audio_source) {
            return Err(INVALID);
        }
        let input = lane.state;
        if input.snapshot.history.len() > 20
            || !bounded_text(&row["state"]["snapshot"])
            || input
                .detected_language
                .as_ref()
                .is_some_and(|code| code.len() > 64)
        {
            return Err(INVALID);
        }
        let status = match input.connection_status.as_str() {
            "" | "idle" => SessionStatus::Idle,
            "connecting" => SessionStatus::Connecting,
            "listening" => SessionStatus::Listening,
            "stopping" => SessionStatus::Stopping,
            "error" => SessionStatus::Error("provider_error".into()),
            _ => return Err(INVALID),
        };
        lanes.push((
            lane.audio_source,
            TranslationSessionState {
                status,
                subtitles: input.snapshot,
                atomic_preview: input.atomic_preview,
                detected_language: input
                    .detected_language
                    .map(|code| DetectedLanguage { code }),
                is_translation_pending: input.is_translation_pending,
                is_translation_preview_pending: input.is_translation_preview_pending,
                is_translation_timed_out: input.is_translation_timed_out,
                translation_recovery: input.translation_recovery,
            },
        ));
    }
    let state = merge_independent_states(lanes, limit as usize, stopped);
    let status = match state.status {
        SessionStatus::Idle => "idle",
        SessionStatus::Connecting => "connecting",
        SessionStatus::Listening => "listening",
        SessionStatus::Stopping => "stopping",
        SessionStatus::Error(_) => "error",
    };
    Ok(
        json!({"snapshot":state.subtitles,"connectionStatus":status,"atomicPreview":state.atomic_preview,
        "detectedLanguage":state.detected_language.map(|language|language.code),
        "isTranslationPending":state.is_translation_pending,"isTranslationPreviewPending":state.is_translation_preview_pending,
        "isTranslationTimedOut":state.is_translation_timed_out,"translationRecovery":state.translation_recovery}),
    )
}

fn bounded_text(value: &Value) -> bool {
    match value {
        Value::String(text) => mimi_runtime::core::models::subtitle_text_within_limit(text),
        Value::Array(items) => items.len() <= 20 && items.iter().all(bounded_text),
        Value::Object(fields) => fields.values().all(bounded_text),
        _ => true,
    }
}
