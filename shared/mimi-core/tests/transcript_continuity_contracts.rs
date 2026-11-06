use mimi_core::openai_transcript_committer::{OpenAITranscriptPairCommitter, TranscriptEvent};
use serde_json::{json, Value};

#[test]
fn independent_transcript_lanes_keep_joint_intervals_in_direct_and_bridge_replays() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../transcript-continuity-contracts.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let mode = case["mode"].as_str().unwrap_or("openai");
        let mut stream = if mode == "gemini" {
            OpenAITranscriptPairCommitter::new_gemini()
        } else {
            OpenAITranscriptPairCommitter::default()
        };
        let mut state = Value::Null;
        let mut direct_pairs = Vec::new();
        let mut bridged_pairs = Vec::new();
        for step in case["steps"].as_array().unwrap() {
            let delta = step["delta"].as_str().unwrap_or("");
            let time = step["elapsed_ms"].as_u64();
            let events = match step["action"].as_str().unwrap() {
                "source" => stream.append_source_delta(delta, time),
                "translation" => stream.append_translation_delta(delta, time),
                "finish" => stream.finish(),
                "settle" => stream.settle(time.unwrap()),
                "reset" => {
                    stream.reset();
                    Vec::new()
                }
                _ => panic!("unknown synthetic action"),
            };
            if step["clear_previews"] == true {
                assert!(events.iter().any(|event| matches!(event, TranscriptEvent::SourceDraft { text, .. } if text.is_empty())));
                assert!(events.iter().any(|event| matches!(event, TranscriptEvent::TranslationDraft(text) if text.is_empty())));
            }
            for event in events {
                if let TranscriptEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } = event
                {
                    direct_pairs.push(json!([source, translation]));
                }
            }
            let response: Value = serde_json::from_str(&mimi_core::bridge::exchange(&json!({
                "state": state,
                "operation": {"type":mode, "action": step["action"], "delta":delta, "elapsed_ms":time}
            }).to_string()).unwrap()).unwrap();
            state = response["state"].clone();
            if step["clear_previews"] == true {
                let events = response["events"].as_array().unwrap();
                assert!(
                    events
                        .iter()
                        .any(|event| event["type"] == "source_draft"
                            && event["payload"]["text"] == "")
                );
                assert!(events
                    .iter()
                    .any(|event| event["type"] == "translation_draft" && event["payload"] == ""));
            }
            for event in response["events"].as_array().unwrap() {
                if event["type"] == "subtitle_final_pair" {
                    bridged_pairs.push(json!([
                        event["payload"]["source"],
                        event["payload"]["translation"]
                    ]));
                }
            }
            assert_eq!(json!(direct_pairs), step["pairs"], "direct {}", case["id"]);
            assert_eq!(json!(bridged_pairs), step["pairs"], "bridge {}", case["id"]);
        }
    }
}
