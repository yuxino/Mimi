use mimi_core::{NoopArchive, SubtitleEvent, SubtitleReducer, SubtitleSnapshot};
use serde_json::{json, Value};

fn display(snapshot: &SubtitleSnapshot) -> Value {
    let (source, translation) = if let Some(preview) = &snapshot.realtime_preview {
        (&preview.source.text, &preview.translation.text)
    } else if let Some(pair) = &snapshot.display_pair {
        (&pair.source, &pair.translation)
    } else {
        (&snapshot.source.text, &snapshot.translation.text)
    };
    json!({"source":source,"translation":translation,
        "confirmed":snapshot.realtime_preview.is_none() && snapshot.display_pair_final,
        "historyCount":snapshot.history.len()})
}

#[test]
fn realtime_lanes_follow_shared_contracts_directly_and_through_the_bridge() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../realtime-display-contracts.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let limit = case["historyLimit"].as_u64().unwrap() as usize;
        let mut reducer = SubtitleReducer::<NoopArchive>::new(limit);
        let mut response: Value = serde_json::from_str(
            &mimi_core::bridge::exchange(
                &json!({"operation":{"type":"create","history_limit":limit}}).to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let op = &step["operation"];
            if op["type"] == "reset" {
                reducer.reset_transient();
            } else {
                let event: SubtitleEvent = serde_json::from_value(op["event"].clone()).unwrap();
                reducer.apply(event);
            }
            assert_eq!(
                display(&reducer.snapshot),
                step["expected"],
                "{} step {index}",
                case["id"]
            );
            assert!(reducer.validate_state(6).is_ok());
            response = serde_json::from_str(
                &mimi_core::bridge::exchange(
                    &json!({"state":response["state"],"operation":op}).to_string(),
                )
                .unwrap(),
            )
            .unwrap();
            let bridged: SubtitleSnapshot =
                serde_json::from_value(response["snapshot"].clone()).unwrap();
            assert_eq!(
                display(&bridged),
                step["expected"],
                "JNI bridge {} step {index}",
                case["id"]
            );
            reducer = serde_json::from_str(&serde_json::to_string(&reducer).unwrap()).unwrap();
        }
    }
}
