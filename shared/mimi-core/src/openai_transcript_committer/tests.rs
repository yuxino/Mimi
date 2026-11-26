use super::*;

#[test]
fn omitted_originals_keep_bounded_translation_and_cannot_contaminate_next_pair() {
    let mut stream = OpenAITranscriptPairCommitter::new(8, None);
    stream.append_source_delta("Old original", Some(0));
    stream.append_translation_delta("先前译文", Some(30_000));
    assert!(matches!(stream.isolate_source().as_slice(),
        [TranscriptEvent::SourceDraft { text, .. }] if text.is_empty()));
    for _ in 0..1_000 {
        let events = stream.append_translation_delta("😀新译文", Some(40_000));
        assert!(
            matches!(events.as_slice(), [TranscriptEvent::TranslationDraft(text)] if text.chars().count() <= 8)
        );
        stream.validate_state(8).unwrap();
    }
    for whitespace in [" \t\r\n", "\u{2003}\u{3000}"] {
        assert!(stream
            .append_source_delta(whitespace, Some(45_000))
            .is_empty());
    }
    // This is still a preview, not a fabricated bilingual confirmation.
    let state = serde_json::to_string(&stream).unwrap();
    let mut restored: OpenAITranscriptPairCommitter = serde_json::from_str(&state).unwrap();
    assert!(restored.finish().is_empty());
    let events = stream.append_source_delta("New.", Some(50_000));
    assert!(
        matches!(events.as_slice(), [TranscriptEvent::TranslationDraft(text), TranscriptEvent::SourceDraft { .. }] if text.is_empty())
    );
    assert!(
        matches!(stream.append_translation_delta("新的。", Some(50_000)).last(),
        Some(TranscriptEvent::SubtitleFinalPair { source, translation, .. }) if source == "New." && translation == "新的。")
    );
}

#[test]
fn gemini_keeps_split_translation_with_its_original_block() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("First sentence.", Some(0));
    let events = stream.append_translation_delta("第一部分。", Some(100));
    assert!(!events
        .iter()
        .any(|e| matches!(e, TranscriptEvent::SubtitleFinalPair { .. })));
    assert!(stream.settle(1_900).is_empty());
    stream.append_translation_delta("后半部分。", Some(1_000));
    stream.append_source_delta(" Next sentence.", Some(1_100));
    stream.append_translation_delta("下一句。", Some(1_900));
    assert!(stream.settle(3_899).is_empty());
    assert!(
        matches!(stream.settle(3_900).as_slice(), [TranscriptEvent::SubtitleFinalPair { source, translation, .. }] if source == "First sentence. Next sentence." && translation == "第一部分。后半部分。下一句。")
    );
    assert!(!stream.has_pending());
    // A genuine repetition belongs to a new block, not a deduplication filter.
    stream.append_source_delta("First sentence.", Some(4_000));
    stream.append_translation_delta("第一句。", Some(4_100));
    assert_eq!(stream.settle(6_100).len(), 1);
}

#[test]
fn gemini_explicit_boundary_absorbs_late_unpunctuated_tail() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("Source ", Some(0));
    stream.append_translation_delta("翻訳", Some(50));
    stream.note_turn_complete(100);
    assert!(stream.settle(599).is_empty());
    stream.append_source_delta("tail", Some(300));
    stream.append_translation_delta("末尾", Some(350));
    assert!(stream.settle(849).is_empty());
    assert!(
        matches!(stream.settle(850).as_slice(), [TranscriptEvent::SubtitleFinalPair { source, translation, .. }] if source == "Source tail" && translation == "翻訳末尾")
    );
}

#[test]
fn gemini_quiet_closes_paired_unpunctuated_text_but_retains_unmatched_text() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("Complete.", Some(0));
    assert!(stream.settle(5_000).is_empty());
    stream.append_translation_delta("尚未完成", Some(5_100));
    assert!(stream.settle(7_099).is_empty());
    assert!(matches!(stream.settle(7_100).as_slice(),
        [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
        if source == "Complete." && translation == "尚未完成"));
    assert!(!stream.has_pending());
    stream.append_translation_delta("Translation only", Some(8_000));
    assert!(stream.settle(10_000).is_empty());
    assert!(stream.has_pending());
}

#[test]
fn gemini_merges_fragments_and_cumulative_extensions_in_independent_lanes() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    for (index, chunk) in ["我", "今天", "不想", "出去。"].iter().enumerate() {
        stream.append_source_delta(chunk, Some(index as u64));
    }
    for (index, (chunk, expected)) in [
        ("私", "私"),
        ("私は今日", "私は今日"),
        ("私は今日外に", "私は今日外に"),
        ("出たくない。", "私は今日外に出たくない。"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            stream.append_translation_delta(chunk, Some(index as u64)),
            vec![TranscriptEvent::TranslationDraft((*expected).into())]
        );
    }
    assert!(matches!(stream.settle(2_003).as_slice(),
        [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
        if source == "我今天不想出去。" && translation == "私は今日外に出たくない。"));
}

#[test]
fn gemini_preserves_repetition_subwords_and_provider_whitespace() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    for chunk in ["trans", "lation", " is very", " very", " good", "."] {
        stream.append_source_delta(chunk, Some(0));
    }
    for chunk in ["哈", "哈", "！"] {
        stream.append_translation_delta(chunk, Some(0));
    }
    assert!(matches!(stream.finish().as_slice(),
        [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
        if source == "translation is very very good." && translation == "哈哈！"));
}

#[test]
fn gemini_equal_cumulative_snapshots_do_not_postpone_quiet_or_cross_a_reset() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("Hello", Some(0));
    stream.append_source_delta("Hello world", Some(100));
    stream.append_translation_delta("你好", Some(0));
    stream.append_translation_delta("你好世界", Some(100));
    assert!(stream
        .append_source_delta("Hello world", Some(1_900))
        .is_empty());
    assert!(stream
        .append_translation_delta("你好世界", Some(2_000))
        .is_empty());
    assert!(stream.settle(2_099).is_empty());
    assert_eq!(stream.settle(2_100).len(), 1);
    // Local silence is not a provider boundary; a resend stays ignored.
    assert!(stream
        .append_translation_delta("你好世界", Some(3_000))
        .is_empty());
    stream.reset();
    assert_eq!(
        stream.append_translation_delta("你好世界", Some(4_000)),
        vec![TranscriptEvent::TranslationDraft("你好世界".into())]
    );
}

#[test]
fn gemini_cumulative_extensions_after_quiet_checkpoint_only_confirm_new_text() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("Hello", Some(0));
    stream.append_source_delta("Hello world", Some(100));
    stream.append_translation_delta("你好", Some(0));
    stream.append_translation_delta("你好世界", Some(100));
    assert_eq!(stream.settle(2_100).len(), 1);
    let state = serde_json::to_string(&stream).unwrap();
    let mut stream: OpenAITranscriptPairCommitter = serde_json::from_str(&state).unwrap();
    stream.validate_state(320).unwrap();
    assert_eq!(
        stream.append_source_delta("Hello world again", Some(3_000)),
        vec![TranscriptEvent::SourceDraft {
            text: " again".into(),
            language: None
        }]
    );
    assert!(stream
        .append_translation_delta("你好世界", Some(3_100))
        .is_empty());
    assert_eq!(
        stream.append_translation_delta("你好世界再见", Some(3_200)),
        vec![TranscriptEvent::TranslationDraft("再见".into())]
    );
    assert!(matches!(stream.settle(5_200).as_slice(),
        [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
        if source == "again" && translation == "再见"));
    assert!(stream
        .append_source_delta("Hello world again", Some(5_300))
        .is_empty());
    assert!(stream
        .append_translation_delta("你好世界再见", Some(5_400))
        .is_empty());
    assert!(!stream.has_pending());
}

#[test]
fn gemini_explicit_turn_keeps_late_cumulative_tail_then_clears_checkpoint() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("A", Some(0));
    stream.append_source_delta("AB", Some(100));
    stream.append_translation_delta("甲", Some(0));
    stream.append_translation_delta("甲乙", Some(100));
    assert_eq!(stream.settle(2_100).len(), 1);
    stream.note_turn_complete(2_200);
    stream.append_source_delta("ABC", Some(2_400));
    stream.append_translation_delta("甲乙丙", Some(2_400));
    assert!(stream.settle(2_899).is_empty());
    assert!(matches!(stream.settle(2_900).as_slice(),
        [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
        if source == "C" && translation == "丙"));
    assert_eq!(
        stream.append_translation_delta("甲乙丙", Some(3_000)),
        vec![TranscriptEvent::TranslationDraft("甲乙丙".into())]
    );
}

#[test]
fn gemini_empty_explicit_turn_clears_checkpoint_and_preserves_new_repetition() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("A", Some(0));
    stream.append_source_delta("AB", Some(100));
    stream.append_translation_delta("甲", Some(0));
    stream.append_translation_delta("甲乙", Some(100));
    assert_eq!(stream.settle(2_100).len(), 1);
    stream.note_turn_complete(2_200);
    assert!(stream.settle(2_700).is_empty());
    assert_eq!(
        stream.append_source_delta("AB", Some(3_000)),
        vec![TranscriptEvent::SourceDraft {
            text: "AB".into(),
            language: None
        }]
    );
    assert_eq!(
        stream.append_translation_delta("甲乙", Some(3_000)),
        vec![TranscriptEvent::TranslationDraft("甲乙".into())]
    );
    assert_eq!(stream.finish().len(), 1);
}

#[test]
fn gemini_checkpoint_preserves_independent_delta_and_cumulative_lanes() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("Hello", Some(0));
    stream.append_translation_delta("你", Some(0));
    stream.append_translation_delta("你好", Some(100));
    assert_eq!(stream.settle(2_100).len(), 1);
    stream.append_source_delta("Hello again", Some(3_000));
    stream.append_translation_delta("你好世界", Some(3_000));
    assert!(matches!(stream.finish().as_slice(),
        [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
        if source == "Hello again" && translation == "世界"));
}

#[test]
fn gemini_checkpoint_and_pending_text_share_the_existing_safety_bound() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_source_delta("a", Some(0));
    stream.append_source_delta(&"a".repeat(5_118), Some(100));
    stream.append_translation_delta("translation", Some(100));
    assert_eq!(stream.settle(2_100).len(), 1);
    stream.validate_state(320).unwrap();
    assert!(matches!(
        stream.append_source_delta("xyz", Some(3_000)).as_slice(),
        [TranscriptEvent::Error { .. }]
    ));
    assert!(!stream.has_pending());
    stream.validate_state(320).unwrap();
}

#[test]
fn gemini_cumulative_state_roundtrips_without_changing_openai_delta_semantics() {
    let mut stream = OpenAITranscriptPairCommitter::new_gemini();
    stream.append_translation_delta("合成", Some(0));
    stream.append_translation_delta("合成文本", Some(1));
    let state = serde_json::to_string(&stream).unwrap();
    let mut restored: OpenAITranscriptPairCommitter = serde_json::from_str(&state).unwrap();
    restored.validate_state(320).unwrap();
    assert!(restored
        .append_translation_delta("合成文本", Some(2))
        .is_empty());
    assert_eq!(
        restored.append_translation_delta("后续", Some(3)),
        vec![TranscriptEvent::TranslationDraft("合成文本后续".into())]
    );
    let mut openai = OpenAITranscriptPairCommitter::default();
    openai.append_translation_delta("合成", None);
    assert_eq!(
        openai.append_translation_delta("合成文本", None),
        vec![TranscriptEvent::TranslationDraft("合成合成文本".into())]
    );
}

#[test]
fn pending_status_tracks_unmatched_content_without_counting_whitespace() {
    let mut stream = OpenAITranscriptPairCommitter::default();
    assert!(!stream.has_pending());
    stream.append_source_delta("Hello.", None);
    assert!(stream.has_pending());
    stream.append_translation_delta("こんにちは。 ", None);
    assert!(!stream.has_pending());
    stream.append_source_delta("Unmatched", None);
    assert!(stream.has_pending());
    stream.reset();
    assert!(!stream.has_pending());
}

#[test]
fn accumulates_append_only_drafts() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    assert_eq!(
        committer.append_source_delta("Hello", None),
        vec![TranscriptEvent::SourceDraft {
            text: "Hello".into(),
            language: Some("en".into())
        }]
    );
    assert_eq!(
        committer.append_source_delta(" world", None),
        vec![TranscriptEvent::SourceDraft {
            text: "Hello world".into(),
            language: Some("en".into())
        }]
    );
}

#[test]
fn finalizes_a_pair_as_one_atomic_event() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    let _ = committer.append_source_delta("Hello world.", None);
    let events = committer.append_translation_delta("你好世界。", None);
    assert_eq!(
        events,
        vec![
            TranscriptEvent::TranslationDraft("你好世界。".into()),
            TranscriptEvent::SubtitleFinalPair {
                source: "Hello world.".into(),
                language: Some("en".into()),
                translation: "你好世界。".into()
            }
        ]
    );
}

#[test]
fn untimed_partial_tails_stay_together_until_the_complete_block() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    let _ = committer.append_source_delta("First. Tail", None);
    let events = committer.append_translation_delta("第一句。尾巴", None);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &committer.finish()[0],
        TranscriptEvent::SubtitleFinalPair { source, translation, .. }
            if source == "First. Tail" && translation == "第一句。尾巴"
    ));
    assert!(!committer.has_pending());
}

#[test]
fn timing_checkpoint_can_align_unequal_punctuation() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    let _ = committer.append_source_delta("A. B.", Some(1_200));
    let events = committer.append_translation_delta("合并译文。", Some(1_200));
    assert!(events.iter().any(|event| matches!(
        event,
        TranscriptEvent::SubtitleFinalPair { source, translation, .. }
            if source == "A. B." && translation == "合并译文。"
    )));
}

#[test]
fn timed_pair_does_not_cut_different_sentence_prefixes_from_joint_coverage() {
    let mut committer = OpenAITranscriptPairCommitter::default();
    committer.append_source_delta("First. Second.", Some(1_000));
    let events = committer.append_translation_delta("第一句。第二句", Some(1_000));
    assert!(!events
        .iter()
        .any(|event| matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));
    let events = committer.append_translation_delta("。", Some(1_000));
    assert!(events.iter().any(|event| matches!(event,
        TranscriptEvent::SubtitleFinalPair { source, translation, .. }
        if source == "First. Second." && translation == "第一句。第二句。")));
    assert!(!committer.has_pending());
}

#[test]
fn untimed_pair_does_not_discard_an_unfinished_half_of_the_interval() {
    let mut committer = OpenAITranscriptPairCommitter::default();
    committer.append_source_delta("First. Second", None);
    let events = committer.append_translation_delta("合并的第一和第二句。", None);
    assert!(!events
        .iter()
        .any(|event| matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));
    committer.append_source_delta(".", None);
    let events = committer.finish();
    assert!(events.iter().any(|event| matches!(event,
        TranscriptEvent::SubtitleFinalPair { source, translation, .. }
        if source == "First. Second." && translation == "合并的第一和第二句。")));
}

#[test]
fn timed_source_deltas_commit_once_and_keep_the_unicode_tail_aligned() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    let _ = committer.append_source_delta("Hello", Some(500));
    let _ = committer.append_source_delta(" world.", Some(1_000));
    let _ = committer.append_source_delta(" 次の文", Some(2_000));

    assert_eq!(
        committer.append_translation_delta("你好世界。", Some(1_000)),
        vec![
            TranscriptEvent::TranslationDraft("你好世界。".into()),
            TranscriptEvent::SubtitleFinalPair {
                source: "Hello world.".into(),
                language: Some("en".into()),
                translation: "你好世界。".into(),
            },
            TranscriptEvent::SourceDraft {
                text: " 次の文".into(),
                language: Some("en".into()),
            },
        ]
    );

    let _ = committer.append_source_delta("。", Some(2_000));
    assert_eq!(
        committer.append_translation_delta("下一句。", Some(2_000)),
        vec![
            TranscriptEvent::TranslationDraft("下一句。".into()),
            TranscriptEvent::SubtitleFinalPair {
                source: "次の文。".into(),
                language: Some("en".into()),
                translation: "下一句。".into(),
            },
        ]
    );
    assert!(committer.finish().is_empty());
}

#[test]
fn timed_translation_deltas_commit_once_and_keep_the_unicode_tail_aligned() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    let _ = committer.append_translation_delta("你好", Some(500));
    let _ = committer.append_translation_delta("世界。", Some(1_000));
    let _ = committer.append_translation_delta(" 后文", Some(2_000));

    assert_eq!(
        committer.append_source_delta("Hello world.", Some(1_000)),
        vec![
            TranscriptEvent::SourceDraft {
                text: "Hello world.".into(),
                language: Some("en".into()),
            },
            TranscriptEvent::SubtitleFinalPair {
                source: "Hello world.".into(),
                language: Some("en".into()),
                translation: "你好世界。".into(),
            },
            TranscriptEvent::TranslationDraft(" 后文".into()),
        ]
    );

    let _ = committer.append_translation_delta("。", Some(2_000));
    assert_eq!(
        committer.append_source_delta("Next.", Some(2_000)),
        vec![
            TranscriptEvent::SourceDraft {
                text: "Next.".into(),
                language: Some("en".into()),
            },
            TranscriptEvent::SubtitleFinalPair {
                source: "Next.".into(),
                language: Some("en".into()),
                translation: "后文。".into(),
            },
        ]
    );
    assert!(committer.finish().is_empty());
}

#[test]
fn one_sided_growth_is_retained_until_the_counterpart_arrives() {
    let mut committer = OpenAITranscriptPairCommitter::new(8, None);
    let _ = committer.append_source_delta("abcdefgh", None);
    let events = committer.append_source_delta("i", None);
    assert!(events
        .iter()
        .all(|event| !matches!(event, TranscriptEvent::Error { .. })));
    assert_eq!(committer.pending_source_character_count(), 9);

    let paired = committer.append_translation_delta("译文。", None);
    assert!(paired
        .iter()
        .all(|event| !matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));
    assert_eq!(
        committer.finish(),
        vec![TranscriptEvent::SubtitleFinalPair {
            source: "abcdefghi".into(),
            language: None,
            translation: "译文。".into(),
        }]
    );
}

#[test]
fn paired_single_deltas_larger_than_the_alignment_checkpoint_remain_complete() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
    let source = "a".repeat(384);
    let translation = "译".repeat(384);

    let _ = committer.append_source_delta(&source, None);
    let events = committer.append_translation_delta(&translation, None);

    assert!(events
        .iter()
        .all(|event| !matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));
    assert_eq!(
        committer.finish(),
        vec![TranscriptEvent::SubtitleFinalPair {
            source,
            language: Some("en".into()),
            translation,
        }]
    );
}

#[test]
fn unpaired_growth_fails_closed_only_at_the_absolute_safety_limit() {
    let mut committer = OpenAITranscriptPairCommitter::new(8, None);
    let source = "a".repeat(committer.maximum_buffer_characters + 1);

    assert_eq!(
        committer.append_source_delta(&source, None),
        vec![TranscriptEvent::Error {
            code: "openai_transcript_safety_limit".into(),
            message: "OpenAI Realtime Translation transcript alignment exceeded its safety limit."
                .into(),
        }]
    );
    assert_eq!(committer.pending_source_character_count(), 0);
    assert_eq!(committer.pending_translation_character_count(), 0);
}

#[test]
fn missing_timing_and_unequal_sentence_segmentation_commit_at_the_bound() {
    let mut committer = OpenAITranscriptPairCommitter::new(32, Some("en".into()));
    let _ = committer.append_source_delta(&"A. B. ".repeat(8), None);
    let events = committer.append_translation_delta(&"合并译文。".repeat(7), None);

    assert!(events
        .iter()
        .all(|event| !matches!(event, TranscriptEvent::Error { .. })));
    assert!(events.iter().any(|event| matches!(
        event,
        TranscriptEvent::SubtitleFinalPair { source, translation, .. }
            if source.contains("A. B.") && translation.contains("合并译文。")
    )));
    assert!(committer.pending_source_character_count() <= 32);
    assert!(committer.pending_translation_character_count() <= 32);
}

#[test]
fn a_short_lagging_translation_cannot_finalize_a_long_source() {
    let mut committer = OpenAITranscriptPairCommitter::new(32, Some("en".into()));
    let source = format!("{}.", "a".repeat(40));
    let translation = format!("你{}。", "译".repeat(39));
    let _ = committer.append_source_delta(&source, None);

    let short = committer.append_translation_delta("你", None);
    assert!(short
        .iter()
        .all(|event| !matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));

    let completed = committer.append_translation_delta(&format!("{}。", "译".repeat(39)), None);
    assert!(completed.iter().any(|event| matches!(
        event,
        TranscriptEvent::SubtitleFinalPair {
            source: committed_source,
            translation: committed_translation,
            ..
        } if committed_source == &source && committed_translation == &translation
    )));
}

#[test]
fn graceful_close_flushes_only_a_meaningful_pair() {
    let mut committer = OpenAITranscriptPairCommitter::new(320, Some("ja".into()));
    let _ = committer.append_source_delta("こんにちは", None);
    let _ = committer.append_translation_delta("你好", None);
    assert_eq!(
        committer.finish(),
        vec![TranscriptEvent::SubtitleFinalPair {
            source: "こんにちは".into(),
            language: Some("ja".into()),
            translation: "你好".into()
        }]
    );

    let mut one_sided = OpenAITranscriptPairCommitter::default();
    let _ = one_sided.append_source_delta("private source tail", None);
    assert!(one_sided.finish().is_empty());
}
