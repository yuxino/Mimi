use super::*;

#[test]
fn reliable_identity_registration_is_bounded_and_does_not_change_subtitles() {
    let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
    let before = reducer.snapshot.clone();
    for index in 0..MAX_IDENTIFIED_SOURCES * 3 {
        reducer.register_source_identity(&format!("sentence-{index}"));
    }
    assert_eq!(reducer.snapshot, before);
    assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
    assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
    assert!(reducer
        .identified_sources
        .iter()
        .all(|item| !item.confirmed));
    assert!(reducer.validate_state(6).is_ok());
    reducer.register_source_identity("");
    reducer.register_source_identity(&"x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1));
    assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
    let last_id = format!("sentence-{}", MAX_IDENTIFIED_SOURCES * 3 - 1);
    reducer.apply(SubtitleEvent::Clear);
    reducer.register_source_identity(&last_id);
    assert!(reducer.identified_sources.is_empty());
    assert_eq!(reducer.snapshot, before);
}

#[test]
fn identified_owner_indexes_and_clear_tombstones_are_bounded_and_validated() {
    let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
    for index in 0..MAX_IDENTIFIED_SOURCES * 3 {
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: format!("source-{index}"),
            role: UtteranceRole::Source,
            text: "Synthetic source".into(),
            is_final: false,
        });
    }
    assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
    assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
    assert!(reducer.validate_state(6).is_ok());
    reducer.apply(SubtitleEvent::Clear);
    assert!(reducer.identified_sources.is_empty());
    assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
    assert!(reducer.validate_state(6).is_ok());
    let mut state = serde_json::to_value(&reducer).unwrap();
    state["discarded_source_ids"] = serde_json::json!(["duplicate", "duplicate"]);
    let restored: SubtitleReducer<NoopArchive> = serde_json::from_value(state).unwrap();
    assert_eq!(restored.validate_state(6), Err("subtitle_state_invalid"));
    reducer.reset_transient();
    assert!(reducer.discarded_source_ids.is_empty());
    assert!(reducer.validate_state(6).is_ok());
}

#[derive(Default)]
struct CountingArchive(usize);
impl ArchiveSink for CountingArchive {
    fn append(&mut self, _pair: &SubtitlePair) {
        self.0 = self.0.saturating_add(1);
    }
    fn clear(&mut self) {
        self.0 = 0;
    }
}

fn confirmed(id: u64, source_id: u64, text: &str) -> SubtitleEvent {
    SubtitleEvent::ConfirmedPair {
        utterance_id: id,
        source_utterance_id: Some(source_id),
        source: text.into(),
        translation: format!("完整译文 {text}"),
    }
}

#[test]
fn history_limit_zero_keeps_one_current_pair_and_does_not_disable_platform_archive() {
    let mut reducer = SubtitleReducer::<CountingArchive>::new(2);
    for id in 1..=3 {
        reducer.apply(confirmed(id, id, &format!("Complete sentence {id}")));
    }
    assert_eq!(reducer.archive.0, 3);
    assert_eq!(reducer.snapshot.history.len(), 2);
    reducer.set_history_limit(0);
    assert!(reducer.snapshot.history.is_empty());
    assert_eq!(reducer.archive.0, 3);
    assert_eq!(
        reducer.snapshot.display_pair.as_ref().unwrap().source,
        "Complete sentence 3"
    );
    reducer.apply(SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 4,
        text: "Unfinished next sentence".into(),
    });
    assert_eq!(
        reducer.snapshot.display_pair.as_ref().unwrap().source,
        "Complete sentence 3"
    );
    reducer.apply(confirmed(4, 4, "Next complete sentence"));
    assert!(reducer.snapshot.history.is_empty());
    assert_eq!(reducer.archive.0, 4);
    assert_eq!(
        reducer.snapshot.display_pair.as_ref().unwrap().source,
        "Next complete sentence"
    );
}

#[test]
fn archive_contents_are_never_serialized_in_shared_opaque_state() {
    let mut reducer = SubtitleReducer::<CountingArchive>::new(1);
    reducer.apply(confirmed(1, 1, "Synthetic complete sentence"));
    assert_eq!(reducer.archive.0, 1);
    let state = serde_json::to_value(&reducer).unwrap();
    assert!(state.get("archive").is_none());
    let restored: SubtitleReducer<CountingArchive> = serde_json::from_value(state).unwrap();
    assert_eq!(restored.archive.0, 0);
    assert_eq!(restored.snapshot, reducer.snapshot);
    assert!(!restored.is_new_confirmation_id(1));
}

#[test]
fn validation_rejects_unbounded_history_and_complete_display_without_echoing_text() {
    let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
    assert!(reducer.validate_state(6).is_ok());
    reducer.set_history_limit(7);
    assert_eq!(reducer.validate_state(6), Err("subtitle_state_invalid"));
    reducer.set_history_limit(0);
    reducer.snapshot.display_pair = Some(PreviewSubtitlePair {
        utterance_id: None,
        source: "x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1),
        translation: "Allowed synthetic translation".into(),
    });
    assert_eq!(reducer.validate_state(6), Err("subtitle_state_invalid"));
    reducer.snapshot.display_pair = None;
    reducer.snapshot.history.push(SubtitlePair::new(
        "Unretained synthetic source".into(),
        "Unretained synthetic translation".into(),
        1,
    ));
    assert_eq!(reducer.validate_state(6), Err("subtitle_state_invalid"));
}

#[test]
fn reconnect_retains_final_display_but_releases_owner_ids_for_the_next_generation() {
    let mut reducer = SubtitleReducer::<NoopArchive>::new(0);
    reducer.apply(confirmed(1, 99, "Previous generation final"));
    let old_display = reducer.snapshot.display_pair.clone();
    reducer.reset_transient();
    assert_eq!(reducer.snapshot.display_pair, old_display);
    reducer.apply(SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 0,
        text: "New generation raw".into(),
    });
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: Some(0),
        source: "New generation complete preview".into(),
        translation: "新一代完整预览".into(),
    });
    assert_eq!(
        reducer.snapshot.display_pair.as_ref().unwrap().source,
        "New generation complete preview"
    );
    reducer.reset_transient();
    assert!(reducer.snapshot.display_pair.is_none());
    assert!(reducer.snapshot.history.is_empty());
}
