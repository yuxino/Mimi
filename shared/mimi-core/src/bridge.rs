//! Bounded stateless JNI interchange. Errors deliberately contain no input.
use crate::subtitle_reducer::{NoopArchive, SubtitleReducer};
use crate::translation_policy;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};

const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
// State is JSON inside a JSON string on the next request. Reserve room for
// escaping and one complete input event instead of returning unrestorable state.
const MAX_STATE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ANDROID_HISTORY: usize = 6;

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    state: Option<String>,
    operation: Value,
}

/// The platform retains bounded state; native code retains no session content.
pub fn exchange(input: &str) -> Result<String, &'static str> {
    if input.len() > MAX_INPUT_BYTES {
        return Err("core_request_limit");
    }
    let request: Request = serde_json::from_str(input).map_err(|_| "core_invalid_request")?;
    if request
        .state
        .as_ref()
        .is_some_and(|state| state.len() > MAX_STATE_BYTES)
    {
        return Err("core_state_limit");
    }
    let op = &request.operation;
    let kind = op["type"].as_str().ok_or("core_invalid_operation")?;
    if kind == "policy" {
        return encode(json!({"policy": translation_policy::policy()}));
    }
    if kind == "openai" || kind == "gemini" {
        use crate::openai_transcript_committer::OpenAITranscriptPairCommitter;
        let mut stream: OpenAITranscriptPairCommitter = match request.state {
            Some(state) => serde_json::from_str(&state).map_err(|_| "core_invalid_stream")?,
            None if kind == "gemini" => OpenAITranscriptPairCommitter::new_gemini(),
            None => OpenAITranscriptPairCommitter::default(),
        };
        stream
            .validate_state(320)
            .map_err(|_| "core_invalid_stream")?;
        if stream.is_stable_block_mode() != (kind == "gemini") {
            return Err("core_invalid_stream");
        }
        let delta = op["delta"].as_str().unwrap_or("");
        if !crate::subtitle_text_within_limit(delta) {
            return Err("core_stream_limit");
        }
        let time = op["elapsed_ms"].as_u64();
        let events = match op["action"].as_str() {
            Some("source") => stream.append_source_delta(delta, time),
            Some("translation") => stream.append_translation_delta(delta, time),
            Some("finish") => stream.finish(),
            Some("turn_complete") if kind == "gemini" => {
                stream.note_turn_complete(time.ok_or("core_invalid_stream_operation")?);
                Vec::new()
            }
            Some("settle") if kind == "gemini" => {
                stream.settle(time.ok_or("core_invalid_stream_operation")?)
            }
            Some("reset") => {
                stream.reset();
                Vec::new()
            }
            _ => return Err("core_invalid_stream_operation"),
        };
        return encode(json!({"state":encode_state(&stream)?,"events":events}));
    }
    if kind == "dashscope" {
        use crate::live_pair_aligner::{LivePairAligner, LivePairEvent, LivePairIdentity};
        let mut stream: LivePairAligner = match request.state {
            Some(state) => serde_json::from_str(&state).map_err(|_| "core_invalid_stream")?,
            None => LivePairAligner::default(),
        };
        stream.validate_state().map_err(|_| "core_invalid_stream")?;
        let events = match op["action"].as_str() {
            Some("clear") => {
                stream.clear_content();
                Vec::new()
            }
            Some("observe") => {
                let event: LivePairEvent = serde_json::from_value(op["event"].clone())
                    .map_err(|_| "core_invalid_event")?;
                let identity: LivePairIdentity = serde_json::from_value(op["identity"].clone())
                    .map_err(|_| "core_invalid_identity")?;
                let valid = |text: Option<&str>| text.is_none_or(crate::subtitle_text_within_limit);
                if !valid(identity.item_id.as_deref())
                    || !valid(identity.previous_item_id.as_deref())
                {
                    return Err("core_stream_limit");
                }
                let valid_event = match &event {
                    LivePairEvent::SourceDraft { text, language }
                    | LivePairEvent::SourceFinal { text, language } => {
                        valid(Some(text)) && valid(language.as_deref())
                    }
                    LivePairEvent::TranslationDraft { text }
                    | LivePairEvent::TranslationFinal { text } => valid(Some(text)),
                    LivePairEvent::UtteranceText { .. } | LivePairEvent::FinalPair { .. } => {
                        return Err("core_invalid_event")
                    }
                    _ => true,
                };
                if !valid_event {
                    return Err("core_stream_limit");
                }
                stream.observe_at(
                    &event,
                    &identity,
                    op["received_at_ns"].as_u64().ok_or("core_invalid_time")?,
                )
            }
            _ => return Err("core_invalid_stream_operation"),
        };
        return encode(json!({"state":encode_state(&stream)?, "events": events}));
    }
    if kind == "decision" {
        let elapsed = op["elapsed_ms"].as_u64().unwrap_or(0);
        let decision = match op["kind"].as_str() {
            Some("retry") => serde_json::to_value(translation_policy::retry_decision(
                op["code"].as_str().unwrap_or("invalid_failure"),
                usize::try_from(op["attempt"].as_u64().unwrap_or(1))
                    .map_err(|_| "core_invalid_attempt")?,
                elapsed,
            ))
            .map_err(|_| "core_encode_failed")?,
            Some("remaining") => {
                json!({"remaining_ms": translation_policy::remaining_budget_ms(elapsed)})
            }
            Some("start") => json!({"start": translation_policy::can_start(elapsed)}),
            Some("admit") => json!({"admit": translation_policy::can_enqueue(
                usize::try_from(op["waiting_depth"].as_u64().ok_or("core_invalid_depth")?).map_err(|_| "core_invalid_depth")?,
                op["accepting"].as_bool().unwrap_or(false))}),
            _ => return Err("core_invalid_decision"),
        };
        return encode(json!({"decision": decision}));
    }
    let mut reducer: SubtitleReducer<NoopArchive> = if kind == "create" {
        SubtitleReducer::new(history_limit(op["history_limit"].as_u64().unwrap_or(0))?)
    } else {
        let state = request.state.ok_or("core_missing_state")?;
        let state: SubtitleReducer<NoopArchive> =
            serde_json::from_str(&state).map_err(|_| "core_invalid_state")?;
        state
            .validate_state(MAX_ANDROID_HISTORY)
            .map_err(|_| "core_invalid_state")?;
        state
    };
    match kind {
        "create" => {}
        "apply" => reducer
            .apply(serde_json::from_value(op["event"].clone()).map_err(|_| "core_invalid_event")?),
        "history_limit" => reducer.set_history_limit(history_limit(
            op["limit"].as_u64().ok_or("core_invalid_history_limit")?,
        )?),
        "reset" => reducer.reset_transient(),
        _ => return Err("core_invalid_operation"),
    }
    reducer
        .validate_state(MAX_ANDROID_HISTORY)
        .map_err(|_| "core_invalid_state")?;
    let state = encode_state(&reducer)?;
    encode(json!({"state": state, "snapshot": reducer.snapshot}))
}

fn history_limit(value: u64) -> Result<usize, &'static str> {
    if value > MAX_ANDROID_HISTORY as u64 {
        return Err("core_invalid_history_limit");
    }
    Ok(value as usize)
}
fn encode(value: Value) -> Result<String, &'static str> {
    serde_json::to_string(&value).map_err(|_| "core_encode_failed")
}
fn encode_state(state: &impl Serialize) -> Result<String, &'static str> {
    let encoded = serde_json::to_string(state).map_err(|_| "core_encode_failed")?;
    if encoded.len() > MAX_STATE_BYTES {
        return Err("core_state_limit");
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(state: Option<&str>, operation: Value) -> Value {
        serde_json::from_str(
            &exchange(&json!({"state":state,"operation":operation}).to_string()).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn stateless_roundtrip_preserves_complete_multilingual_text() {
        let created = call(None, json!({"type":"create"}));
        let text = "今日は雨です。明日また出発します，아직 끝나지 않았어요。";
        let applied = call(
            created["state"].as_str(),
            json!({"type":"apply","event":{"type":"source_draft","text":text}}),
        );
        assert_eq!(applied["snapshot"]["source"]["text"], text);
        let reset = call(applied["state"].as_str(), json!({"type":"reset"}));
        assert_eq!(reset["snapshot"]["source"]["text"], "");
    }
    #[test]
    fn invalid_state_and_unbounded_history_fail_without_echoing_content() {
        assert_eq!(
            exchange("private malformed input"),
            Err("core_invalid_request")
        );
        assert_eq!(
            exchange(&json!({"operation":{"type":"create","history_limit":7}}).to_string()),
            Err("core_invalid_history_limit")
        );
        assert_eq!(
            exchange(
                &json!({"state":"private malformed state","operation":{"type":"reset"}})
                    .to_string()
            ),
            Err("core_invalid_state")
        );
    }
    #[test]
    fn oversized_identity_never_produces_unrestorable_subtitle_state() {
        let created = call(None, json!({"type":"create"}));
        let operation = json!({"type":"apply", "event":{"type":"utterance_text", "utterance_id":"x".repeat(65_537),"role":"source","text":"ok","is_final":false}});
        // A core reducer may ignore invalid events; either response must be
        // restorable, or the bridge must fail with a content-free label.
        match exchange(&json!({"state":created["state"],"operation":operation}).to_string()) {
            Ok(response) => {
                let response: Value = serde_json::from_str(&response).unwrap();
                call(response["state"].as_str(), json!({"type":"reset"}));
            }
            Err(code) => assert_eq!(code, "core_invalid_state"),
        }
    }
    #[test]
    fn escaped_stream_state_budget_fails_before_returning_unrestorable_state() {
        let text = "\\".repeat(65_536);
        let mut state: Option<String> = None;
        let mut bounded_failure = false;
        for index in 0..64 {
            let operation = json!({"type":"dashscope","action":"observe", "event":{"type":"source_draft","text":text,"language":null},"identity":{"item_id":format!("source-{index}"),"previous_item_id":null},"received_at_ns":index});
            match exchange(&json!({"state":state,"operation":operation}).to_string()) {
                Ok(response) => {
                    let response: Value = serde_json::from_str(&response).unwrap();
                    state = Some(response["state"].as_str().unwrap().to_owned());
                    let follow = json!({"state":state,"operation":{"type":"dashscope","action":"observe", "event":{"type":"passthrough","is_content":false},"identity":{"item_id":null,"previous_item_id":null},"received_at_ns":index}}).to_string();
                    assert!(follow.len() <= MAX_INPUT_BYTES);
                    assert!(exchange(&follow).is_ok());
                }
                Err(code) => {
                    assert_eq!(code, "core_state_limit");
                    bounded_failure = true;
                    break;
                }
            }
        }
        assert!(bounded_failure);
    }
}
