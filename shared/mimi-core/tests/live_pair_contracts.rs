use mimi_core::live_pair_aligner::{LivePairAligner, LivePairEvent, LivePairIdentity};
use mimi_core::{NoopArchive, SubtitleEvent, SubtitleReducer, SubtitleSnapshot};
use serde_json::json;
use serde_json::Value;

#[test]
fn synthetic_identity_contracts_keep_pairs_exact_through_stateless_round_trips() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../live-pair-contracts.json")).unwrap();
    assert_eq!(fixtures["schemaVersion"], 1);
    for case in fixtures["cases"].as_array().unwrap() {
        let mut aligner = LivePairAligner::default();
        let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let output = if step["clearContent"] == true {
                aligner.clear_content();
                reducer.apply(SubtitleEvent::Clear);
                Vec::new()
            } else if step["resetStream"] == true {
                aligner = LivePairAligner::default();
                reducer.reset_transient();
                Vec::new()
            } else {
                let event: LivePairEvent = serde_json::from_value(step["event"].clone()).unwrap();
                let identity: LivePairIdentity =
                    serde_json::from_value(step["identity"].clone()).unwrap();
                aligner.observe_at(&event, &identity, step["atNs"].as_u64().unwrap())
            };
            assert_eq!(
                serde_json::to_value(&output).unwrap(),
                step["expected"],
                "{} step {}",
                case["id"],
                index
            );
            for event in output {
                if let Some(event) = event.subtitle_event() {
                    reducer.apply(event);
                }
            }
            if let Some(expected) = step.get("expectedReducer") {
                assert_eq!(
                    project(&reducer.snapshot),
                    *expected,
                    "{} step {} reducer",
                    case["id"],
                    index
                );
            }
            assert!(aligner.validate_state().is_ok());
            assert!(reducer.validate_state(6).is_ok());
            aligner = serde_json::from_str(&serde_json::to_string(&aligner).unwrap()).unwrap();
            reducer = serde_json::from_str(&serde_json::to_string(&reducer).unwrap()).unwrap();
        }
    }
}

fn project(snapshot: &SubtitleSnapshot) -> Value {
    let pair = |pair: Option<&mimi_core::PreviewSubtitlePair>| {
        pair.map_or(Value::Null, |pair| json!({"utteranceId":pair.utterance_id,"source":pair.source,"translation":pair.translation}))
    };
    json!({
        "source":snapshot.source,
        "translation":snapshot.translation,
        "displayPair":pair(snapshot.display_pair.as_ref()),
        "history":snapshot.history.iter().map(|pair| json!({"source":pair.source,"translation":pair.translation})).collect::<Vec<_>>()
    })
}
