use mimi_core::{AudioSource, NoopArchive, SubtitleEvent, SubtitleReducer, SubtitleSnapshot};
use serde_json::{json, Value};

fn projection(snapshot: &SubtitleSnapshot) -> Value {
    let pair = |pair: Option<&mimi_core::PreviewSubtitlePair>| {
        pair.map_or(
            Value::Null,
            |p| json!({"source":p.source,"translation":p.translation}),
        )
    };
    json!({
        "source":{"text":snapshot.source.text,"isFinal":snapshot.source.is_final},
        "translation":{"text":snapshot.translation.text,"isFinal":snapshot.translation.is_final},
        "previewPair":pair(snapshot.preview_pair.as_ref()),
        "displayPair":pair(snapshot.display_pair.as_ref()),
        "history":snapshot.history.iter().map(|p| json!({"audioSource":p.audio_source,"source":p.source,"translation":p.translation})).collect::<Vec<_>>()
    })
}

#[test]
fn shared_synthetic_contracts_preserve_every_complete_field_and_identity_boundary() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../subtitle-contracts.json")).unwrap();
    assert_eq!(fixtures["schemaVersion"], 1);
    let cases = fixtures["cases"].as_array().unwrap();
    assert!(cases.len() >= 10);
    for case in cases {
        let limit = case["maxHistoryCount"].as_u64().unwrap() as usize;
        let source: AudioSource = serde_json::from_value(case["audioSource"].clone()).unwrap();
        let mut reducer = SubtitleReducer::<NoopArchive>::new(limit);
        reducer.audio_source = source;
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            if let Some(event) = step.get("event") {
                let event: SubtitleEvent = serde_json::from_value(event.clone()).unwrap();
                reducer.apply(event);
            } else if step["resetTransient"] == true {
                reducer.reset_transient();
            } else if step["beginSession"] == true {
                reducer.begin_new_session();
            } else if let Some(limit) = step["historyLimit"].as_u64() {
                reducer.set_history_limit(limit as usize);
            } else {
                panic!("unknown contract operation");
            }
            assert_eq!(
                projection(&reducer.snapshot),
                step["expected"],
                "{} step {}",
                case["id"],
                index
            );
            if let Some(expected) = step.get("displayPairFinal") {
                assert_eq!(
                    reducer.snapshot.display_pair_final,
                    expected.as_bool().unwrap()
                );
            }
            assert!(reducer.validate_state(6).is_ok());
            // Opaque state round-trip must keep watermarks and ownership, not
            // just the visible strings. Every subsequent operation uses it.
            reducer = serde_json::from_str(&serde_json::to_string(&reducer).unwrap()).unwrap();
        }
    }
}

#[test]
fn stateless_bridge_executes_the_same_contract_cases_without_a_second_reducer() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../subtitle-contracts.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        // Android currently captures system playback only; lane isolation is
        // exercised directly by the microphone contract above.
        if case["audioSource"] != "system" {
            continue;
        }
        let mut response: Value = serde_json::from_str(
            &mimi_core::bridge::exchange(
                &json!({"operation":{"type":"create","history_limit":case["maxHistoryCount"]}})
                    .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let operation = if let Some(event) = step.get("event") {
                json!({"type":"apply","event":event})
            } else if step["resetTransient"] == true {
                json!({"type":"reset"})
            } else if step["beginSession"] == true {
                json!({"type":"begin_session"})
            } else {
                json!({"type":"history_limit","limit":step["historyLimit"]})
            };
            response = serde_json::from_str(
                &mimi_core::bridge::exchange(
                    &json!({"state":response["state"],"operation":operation}).to_string(),
                )
                .unwrap(),
            )
            .unwrap();
            if let Some(expected) = step.get("displayPairFinal") {
                assert_eq!(response["snapshot"]["displayPairFinal"], *expected);
            }
            let snapshot: SubtitleSnapshot =
                serde_json::from_value(response["snapshot"].clone()).unwrap();
            assert_eq!(
                projection(&snapshot),
                step["expected"],
                "bridge {} step {}",
                case["id"],
                index
            );
        }
    }
}
