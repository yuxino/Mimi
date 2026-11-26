use super::*;
use crate::core::models::SubtitlePair;

fn history_with_saved_pair() -> (tempfile::TempDir, SessionHistory, String) {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pair(
            &id,
            &SubtitlePair::new("Synthetic source".into(), "译文".into(), 150),
        )
        .unwrap();
    history.finalize(&id, 200, false).unwrap();
    (directory, history, id)
}

#[test]
fn concurrent_private_exports_have_independent_file_ownership() {
    let (directory, history, id) = history_with_saved_pair();
    let exports = directory.path().join("exports");
    let (first, second) = std::thread::scope(|scope| {
        let export = || {
            history
                .export_transcript_to_directory(&id, TranscriptExportOptions::default(), &exports)
                .unwrap()
        };
        let first = scope.spawn(export);
        let second = scope.spawn(export);
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(first, second);
    let bytes = fs::read(&first).unwrap();
    fs::remove_file(first).unwrap();
    assert_eq!(fs::read(&second).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&exports).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&second).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn title_updates_are_private_bounded_and_do_not_rewrite_saved_content() {
    let (_directory, history, id) = history_with_saved_pair();
    let transcript = fs::read(history.path(&id, "json").unwrap()).unwrap();
    assert!(history.list().unwrap()[0].title.is_none());
    history.rename(&id, "  日本語 / 中文 😀  ").unwrap();
    assert_eq!(
        history.list().unwrap()[0].title.as_deref(),
        Some("日本語 / 中文 😀")
    );
    assert_eq!(
        fs::read(history.path(&id, "json").unwrap()).unwrap(),
        transcript
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(history.path(&id, "meta.json").unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
        );
    }
    let title = "😀".repeat(MAX_TITLE_CHARACTERS);
    history.rename(&id, &title).unwrap();
    for invalid in ["😀".repeat(MAX_TITLE_CHARACTERS + 1), "Line\nbreak".into()] {
        assert!(history.rename(&id, &invalid).is_err());
        assert_eq!(
            history.list().unwrap()[0].title.as_deref(),
            Some(title.as_str())
        );
    }
    assert!(history.rename("../escape", "Title").is_err());
    assert!(history
        .rename(&uuid::Uuid::new_v4().to_string(), "Title")
        .is_err());
    assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 2);
    history.rename(&id, " \t ").unwrap();
    assert!(history.list().unwrap()[0].title.is_none());
    assert!(!history.path(&id, "meta.json").unwrap().exists());
    history.rename(&id, "Delete with session").unwrap();
    history.delete(&id).unwrap();
    assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
}

#[test]
fn damaged_or_oversized_titles_do_not_hide_readable_sessions() {
    let (_directory, history, id) = history_with_saved_pair();
    for contents in [
        b"invalid json".to_vec(),
        br#"{"version":2,"title":"Unsupported"}"#.to_vec(),
        vec![b'x'; MAX_TITLE_FILE_BYTES as usize + 1],
    ] {
        fs::write(history.path(&id, "meta.json").unwrap(), contents).unwrap();
        let items = history.list().unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].title.is_none());
        assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
    }
    history.rename(&id, "Recovered title").unwrap();
    assert_eq!(
        history.list().unwrap()[0].title.as_deref(),
        Some("Recovered title")
    );
}

#[test]
fn failed_title_destination_leaves_transcript_and_no_temporary_copy() {
    let (_directory, history, id) = history_with_saved_pair();
    let destination = history.path(&id, "meta.json").unwrap();
    fs::create_dir(&destination).unwrap();
    let before = history.export_transcript(&id).unwrap();
    assert!(history.rename(&id, "Cannot replace a directory").is_err());
    assert!(destination.is_dir());
    assert_eq!(history.export_transcript(&id).unwrap(), before);
    assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 2);
}

#[test]
fn audio_only_and_recovered_sessions_can_have_titles() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pcm_from(&id, AudioSource::System, 16000, &[1, 0])
        .unwrap();
    history.rename(&id, "Audio only").unwrap();
    let items = history.list().unwrap();
    assert_eq!(items[0].count, 0);
    assert!(items[0].has_audio);
    assert_eq!(items[0].title.as_deref(), Some("Audio only"));
    history.finalize(&id, 200, false).unwrap();
    let restored = SessionHistory::new(history.directory.clone(), false);
    assert_eq!(
        restored.list().unwrap()[0].title.as_deref(),
        Some("Audio only")
    );
    restored.delete(&id).unwrap();
    assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
}

#[derive(Default)]
struct TranscriptWriter {
    bytes: Vec<u8>,
    writes: usize,
    fail_after: Option<usize>,
    fail_flush: bool,
}

impl Write for TranscriptWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        let count = self.fail_after.map_or(bytes.len(), |limit| {
            bytes.len().min(limit.saturating_sub(self.bytes.len()))
        });
        if count == 0 && !bytes.is_empty() {
            return Err(io::Error::other("synthetic write failure"));
        }
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::other("synthetic flush failure"))
        } else {
            Ok(())
        }
    }
}

fn synthetic_final_transcript(count: usize) -> SavedTranscript {
    SavedTranscript {
        version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        started_at_ms: 100,
        ended_at_ms: 200,
        limited: false,
        entries: (0..count)
            .map(|index| {
                let mut pair = SubtitlePair::new(
                    "Synthetic \\\"\n日本語".into(),
                    "译文\t😀".into(),
                    index as u64 + 100,
                );
                if index % 2 != 0 {
                    pair.audio_source = AudioSource::Microphone;
                }
                pair
            })
            .collect(),
    }
}

struct ReadFailureAfterRewind {
    input: io::Cursor<Vec<u8>>,
    fail_at: usize,
    rewound: bool,
    failures: usize,
}

impl ReadFailureAfterRewind {
    fn new(bytes: &[u8], fail_at: usize) -> Self {
        Self {
            input: io::Cursor::new(bytes.to_vec()),
            fail_at,
            rewound: false,
            failures: 0,
        }
    }
}

impl Read for ReadFailureAfterRewind {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let input = self.fill_buf()?;
        let count = input.len().min(output.len());
        output[..count].copy_from_slice(&input[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for ReadFailureAfterRewind {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let available = if self.rewound {
            let available = self.fail_at.saturating_sub(self.input.position() as usize);
            if available == 0 {
                self.failures += 1;
                return Err(io::Error::other("synthetic second-pass read failure"));
            }
            available
        } else {
            usize::MAX
        };
        let input = self.input.fill_buf()?;
        Ok(&input[..input.len().min(available)])
    }

    fn consume(&mut self, amount: usize) {
        self.input.consume(amount);
    }
}

impl Seek for ReadFailureAfterRewind {
    fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
        let offset = self.input.seek(position)?;
        if matches!(position, io::SeekFrom::Start(0)) {
            self.rewound = true;
        }
        Ok(offset)
    }
}

#[test]
fn second_pass_read_failures_do_not_publish_or_return_shortened_history() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let mut saved = synthetic_final_transcript(3);
    for pair in &mut saved.entries {
        pair.created_at_ms += 300;
    }
    history.begin(&saved.id, saved.started_at_ms).unwrap();
    let mut header = serde_json::to_value(&saved).unwrap();
    header["entries"] = serde_json::json!([]);
    let mut bytes = serde_json::to_vec(&header).unwrap();
    bytes.push(b'\n');
    let mut boundaries = Vec::new();
    for pair in &saved.entries {
        boundaries.push(bytes.len());
        serde_json::to_writer(&mut bytes, pair).unwrap();
        bytes.push(b'\n');
    }
    let journal = history.path(&saved.id, "jsonl").unwrap();
    fs::write(&journal, &bytes).unwrap();
    let final_path = history.path(&saved.id, "json").unwrap();
    let exports = directory.path().join("exports");
    for fail_at in boundaries {
        let mut input = ReadFailureAfterRewind::new(&bytes, fail_at);
        let original = reader::scan(&mut input, &saved.id, None).unwrap();
        assert_eq!(original.entries, saved.entries.len());
        input.rewind().unwrap();
        let error = persist_final_transcript(
            &history.directory,
            &final_path,
            &original,
            &mut input,
            900,
            true,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(input.failures > 0);
        assert!(!final_path.exists());
        assert_eq!(fs::read(&journal).unwrap(), bytes);
        assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 1);

        let mut input = ReadFailureAfterRewind::new(&bytes, fail_at);
        let options = TranscriptExportOptions::default();
        let error = write_private_export(&exports, options, |writer| {
            write_transcript(&mut input, &saved.id, options, writer)
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(input.failures > 0);
        assert_eq!(fs::read_dir(&exports).unwrap().count(), 0);

        for query in ["", "synthetic"] {
            let mut input = ReadFailureAfterRewind::new(&bytes, fail_at);
            let error = read_page(&mut input, &saved.id, query, 1)
                .err()
                .expect("a changed second pass must reject the page");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(input.failures > 0);
        }
    }
    // A successful finalization still applies the new end/limit values;
    // comparison uses the original header, not those output overrides.
    assert!(history.finalize(&saved.id, 900, true).unwrap());
    let finalized = history.read(&saved.id).unwrap();
    assert_eq!((finalized.ended_at_ms, finalized.limited), (900, true));
    assert_eq!(finalized.entries.len(), saved.entries.len());
}

#[test]
fn streamed_pages_and_exports_preserve_saved_and_recovered_content() {
    use crate::core::lifecycle::session_archive::{
        TranscriptExportContent, TranscriptExportFormat,
    };
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let mut saved = synthetic_final_transcript(137);
    saved.limited = true;
    for (index, pair) in saved.entries.iter_mut().enumerate() {
        pair.source = format!(
            "Synthetic {index} {}",
            if index % 3 == 0 { "Needle" } else { "sample" }
        );
        pair.translation = format!("译文 **ÄPFEL**\n😀 {index}");
        // Confirmation order is independent of timestamps.
        pair.created_at_ms = 100 + (index % 7) as u64;
    }
    history.begin(&saved.id, saved.started_at_ms).unwrap();
    for extension in ["jsonl", "json"] {
        let path = history.path(&saved.id, extension).unwrap();
        let mut file = io::BufWriter::new(fs::File::create(path).unwrap());
        if extension == "json" {
            serde_json::to_writer(&mut file, &saved).unwrap();
        } else {
            let mut header = serde_json::to_value(&saved).unwrap();
            header["entries"].as_array_mut().unwrap().truncate(7);
            serde_json::to_writer(&mut file, &header).unwrap();
            for pair in &saved.entries[7..] {
                file.write_all(b"\n{broken JSON}\n\n").unwrap();
                serde_json::to_writer(&mut file, pair).unwrap();
            }
            file.write_all(b"\n{\"source\":\"unfinished").unwrap();
        }
        file.flush().unwrap();
        for query in ["", "  NEEDLE  ", "äpfel", "missing"] {
            for page in [0, 1, 2, usize::MAX] {
                assert_eq!(
                    serde_json::to_value(history.page(&saved.id, query, page).unwrap()).unwrap(),
                    serde_json::to_value(saved.page(query, page)).unwrap(),
                    "{extension}: query={query}, page={page}"
                );
            }
        }
        for format in [
            TranscriptExportFormat::Txt,
            TranscriptExportFormat::Markdown,
        ] {
            for content in [
                TranscriptExportContent::Bilingual,
                TranscriptExportContent::Source,
                TranscriptExportContent::Translation,
            ] {
                for include_timestamps in [false, true] {
                    let options = TranscriptExportOptions {
                        format,
                        content,
                        include_timestamps,
                    };
                    let expected = saved.export_with_options(options).unwrap().into_bytes();
                    if format == TranscriptExportFormat::Markdown {
                        assert!(expected.starts_with(b"# Mimi transcript"));
                    }
                    assert_eq!(
                        history
                            .export_transcript_with_options(&saved.id, options)
                            .unwrap(),
                        expected
                    );
                    let exported = history
                        .export_transcript_to_directory(
                            &saved.id,
                            options,
                            &directory.path().join("exports"),
                        )
                        .unwrap();
                    assert_eq!(fs::read(&exported).unwrap(), expected);
                    fs::remove_file(exported).unwrap();
                }
            }
        }
    }
}

#[test]
fn transcript_export_propagates_write_errors_and_does_not_publish_invalid_content() {
    let (directory, history, id) = history_with_saved_pair();
    let mut writer = TranscriptWriter {
        fail_after: Some(1),
        ..Default::default()
    };
    let error = history
        .export_transcript_to_writer(&id, TranscriptExportOptions::default(), &mut writer)
        .unwrap_err();
    assert_eq!(error.to_string(), "synthetic write failure");
    fs::write(history.path(&id, "json").unwrap(), b"{\"entries\":[]}").unwrap();
    let exports = directory.path().join("exports");
    assert!(history
        .export_transcript_to_directory(&id, TranscriptExportOptions::default(), &exports)
        .is_err());
    assert_eq!(fs::read_dir(exports).unwrap().count(), 0);
}

#[test]
fn final_transcript_write_batches_small_tokens_and_preserves_bytes() {
    let saved = synthetic_final_transcript(512);
    let mut writer = TranscriptWriter::default();
    write_fixture_final_transcript(&mut writer, &saved).unwrap();
    assert_eq!(writer.bytes, serde_json::to_vec(&saved).unwrap());
    assert!(
        writer.writes < saved.entries.len() / 4,
        "finalization should batch JSON tokens, writes={}",
        writer.writes,
    );
}

#[test]
fn final_transcript_write_propagates_tail_write_and_flush_failures() {
    let saved = synthetic_final_transcript(1);
    let expected = serde_json::to_vec(&saved).unwrap();
    assert!(expected.len() < 8192);
    for accepted in [0, expected.len() - 1] {
        let mut writer = TranscriptWriter {
            fail_after: Some(accepted),
            ..Default::default()
        };
        assert!(write_fixture_final_transcript(&mut writer, &saved).is_err());
        assert_eq!(writer.bytes, expected[..accepted]);
    }
    let mut writer = TranscriptWriter {
        fail_flush: true,
        ..Default::default()
    };
    assert!(write_fixture_final_transcript(&mut writer, &saved).is_err());
    assert_eq!(writer.bytes, expected);
}

fn write_fixture_final_transcript(
    writer: &mut impl Write,
    saved: &SavedTranscript,
) -> io::Result<()> {
    let mut input = io::Cursor::new(serde_json::to_vec(saved)?);
    let metadata = reader::scan(&mut input, &saved.id, None)?;
    input.rewind()?;
    write_final_transcript(
        writer,
        &metadata,
        &mut input,
        saved.ended_at_ms,
        saved.limited,
    )
}

fn assert_metadata_matches_transcript(history: &SessionHistory, id: &str) {
    let saved = history.read(id).unwrap();
    let metadata = history.read_metadata(id).unwrap();
    assert_eq!(metadata.id, saved.id);
    assert_eq!(metadata.started_at_ms, saved.started_at_ms);
    assert_eq!(metadata.ended_at_ms, saved.ended_at_ms);
    assert_eq!(metadata.limited, saved.limited);
    assert_eq!(metadata.entries, saved.entries.len());
}

#[test]
fn list_metadata_preserves_saved_and_interrupted_journal_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    for at in [150, 125, 180] {
        let mut pair = SubtitlePair::new("Synthetic \\\"\n日本語".into(), "译文\t😀".into(), at);
        pair.audio_source = AudioSource::Microphone;
        history.append_pair(&id, &pair).unwrap();
    }
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(history.path(&id, "jsonl").unwrap())
        .unwrap();
    file.write_all(b"\n{invalid json}\n{\"source\":5}\n")
        .unwrap();
    history
        .append_pair(
            &id,
            &SubtitlePair::new("Synthetic tail".into(), "译文".into(), 200),
        )
        .unwrap();
    assert_metadata_matches_transcript(&history, &id);
    assert_eq!(history.list().unwrap()[0].ended_at_ms, 200);

    // Invalid UTF-8 stops recovery at this line, even if valid JSON follows.
    file.write_all(&[b'{', 0xff, b'\n']).unwrap();
    history
        .append_pair(
            &id,
            &SubtitlePair::new("Synthetic ignored".into(), "译文".into(), 300),
        )
        .unwrap();
    assert_metadata_matches_transcript(&history, &id);
    assert_eq!(history.list().unwrap()[0].count, 4);
    assert!(history.finalize(&id, 250, true).unwrap());
    assert_metadata_matches_transcript(&history, &id);
    let listed = history.list().unwrap();
    assert_eq!(
        (listed[0].count, listed[0].ended_at_ms, listed[0].limited),
        (4, 250, true)
    );
}

#[test]
fn list_metadata_preserves_header_and_appended_entry_count_limits() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    let count = crate::core::lifecycle::session_archive::TRANSCRIPT_COUNT_LIMIT;
    let pair = SubtitlePair::new("Synthetic".into(), "译文".into(), 200);
    let saved = SavedTranscript {
        version: 1,
        id: id.clone(),
        started_at_ms: 100,
        ended_at_ms: 150,
        limited: true,
        entries: vec![pair.clone(); count + 1],
    };
    let path = history.path(&id, "jsonl").unwrap();
    let mut file = io::BufWriter::new(fs::File::create(&path).unwrap());
    serde_json::to_writer(&mut file, &saved).unwrap();
    file.write_all(b"\n").unwrap();
    serde_json::to_writer(&mut file, &pair).unwrap();
    file.flush().unwrap();
    assert_metadata_matches_transcript(&history, &id);
    let metadata = history.read_metadata(&id).unwrap();
    assert_eq!((metadata.entries, metadata.ended_at_ms), (count + 1, 150));

    let mut file = io::BufWriter::new(fs::File::create(path).unwrap());
    serde_json::to_writer(
        &mut file,
        &SavedTranscript {
            entries: Vec::new(),
            ..saved
        },
    )
    .unwrap();
    for index in 0..=count {
        file.write_all(b"\n").unwrap();
        let pair = SubtitlePair::new("Synthetic".into(), "译文".into(), index as u64 + 200);
        serde_json::to_writer(&mut file, &pair).unwrap();
    }
    file.flush().unwrap();
    assert_metadata_matches_transcript(&history, &id);
    let metadata = history.read_metadata(&id).unwrap();
    assert_eq!(
        (metadata.entries, metadata.ended_at_ms),
        (count, count as u64 + 199)
    );
}

#[test]
fn list_metadata_rejects_the_same_invalid_saved_fields_as_full_reads() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    let baseline = serde_json::json!({
        "version": 1, "id": id, "startedAtMs": 100, "endedAtMs": 200, "limited": false,
        "entries": [{"source": "Synthetic", "translation": "译文", "createdAt": 150}]
    });
    let mut invalid = Vec::new();
    for (field, replacement) in [
        ("source", serde_json::json!(5)),
        ("translation", serde_json::Value::Null),
        ("createdAt", serde_json::json!(-1)),
        ("audioSource", serde_json::json!("invalid")),
    ] {
        let mut value = baseline.clone();
        value["entries"][0][field] = replacement;
        invalid.push(value);
    }
    for field in ["source", "translation", "createdAt"] {
        let mut value = baseline.clone();
        value["entries"][0].as_object_mut().unwrap().remove(field);
        invalid.push(value);
    }
    for (field, replacement) in [
        ("version", serde_json::json!(2)),
        ("id", serde_json::json!("other")),
        ("entries", serde_json::json!({})),
    ] {
        let mut value = baseline.clone();
        value[field] = replacement;
        invalid.push(value);
    }
    let path = history.path(&id, "json").unwrap();
    for value in invalid {
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(history.read(&id).is_err());
        assert!(history.read_metadata(&id).is_err());
        assert!(history.list().unwrap().is_empty());
    }
    fs::write(&path, serde_json::to_vec(&baseline).unwrap()).unwrap();
    assert_metadata_matches_transcript(&history, &id);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_TRANSCRIPT_FILE_BYTES + 1)
        .unwrap();
    assert!(history.read(&id).is_err());
    assert!(history.read_metadata(&id).is_err());
    assert!(history.list().unwrap().is_empty());
}

#[test]
fn saves_lists_reads_and_deletes_one_session_without_path_escape() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pair(&id, &SubtitlePair::new("source".into(), "译文".into(), 150))
        .unwrap();
    history.append_pcm(&id, 16_000, &[1, 0, 2, 0]).unwrap();
    assert!(history.finalize(&id, 200, false).unwrap());
    assert_eq!(history.list().unwrap()[0].count, 1);
    assert!(history.list().unwrap()[0].has_audio);
    assert_eq!(history.page(&id, "译文", 0).unwrap().total, 1);
    assert_eq!(&history.audio(&id).unwrap()[44..], &[1, 0, 2, 0]);
    assert!(history.path(&id, "json").unwrap().exists());
    assert!(history.path(&id, "wav").unwrap().exists());
    assert!(!history.path(&id, "jsonl").unwrap().exists());
    assert!(!history.path(&id, "pcm").unwrap().exists());
    assert!(history.delete("../escape").is_err());
    history.delete(&id).unwrap();
    assert!(history.list().unwrap().is_empty());
}

#[test]
fn interrupted_session_is_readable_from_incremental_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history");
    let history = SessionHistory::new(path.clone(), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pair(&id, &SubtitlePair::new("spoken".into(), "译文".into(), 150))
        .unwrap();
    history.append_pcm(&id, 16_000, &[1, 0, 2, 0]).unwrap();
    drop(history);

    let restored = SessionHistory::new(path, false);
    let item = &restored.list().unwrap()[0];
    assert_eq!((item.count, item.has_audio), (1, true));
    assert_eq!(restored.page(&id, "spoken", 0).unwrap().total, 1);
    let wav = restored.audio(&id).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[44..], &[1, 0, 2, 0]);
    restored.clear_text(&id).unwrap();
    assert_eq!(restored.list().unwrap()[0].count, 0);
    restored.delete(&id).unwrap();
    assert!(restored.list().unwrap().is_empty());
}

#[test]
fn finalizes_audio_only_from_disk_and_discards_empty_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let empty_id = uuid::Uuid::new_v4().to_string();
    history.begin(&empty_id, 100).unwrap();
    assert!(!history.finalize(&empty_id, 200, false).unwrap());
    assert!(!history.path(&empty_id, "jsonl").unwrap().exists());

    let audio_id = uuid::Uuid::new_v4().to_string();
    history.begin(&audio_id, 300).unwrap();
    history.append_pcm(&audio_id, 24_000, &[3, 0]).unwrap();
    assert!(history.finalize(&audio_id, 400, false).unwrap());
    let item = &history.list().unwrap()[0];
    assert_eq!((item.count, item.has_audio), (0, true));
    let wav = history.audio(&audio_id).unwrap();
    assert_eq!(&wav[24..28], &24_000_u32.to_le_bytes());
    assert_eq!(&wav[44..], &[3, 0]);
    assert!(!history.path(&audio_id, "pcm").unwrap().exists());
}

#[test]
fn recovers_confirmed_pairs_before_a_truncated_final_write() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pair(&id, &SubtitlePair::new("source".into(), "译文".into(), 150))
        .unwrap();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(history.path(&id, "jsonl").unwrap())
        .unwrap();
    file.write_all(&[b'{', 0xff]).unwrap();
    assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
    assert!(history.finalize(&id, 200, false).unwrap());
    assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
}
#[test]
fn dual_inputs_remain_separate_through_journal_finalization_and_export() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    let system = SubtitlePair::new(
        "Synthetic remote".into(),
        "Synthetic remote translation".into(),
        150,
    );
    let mut microphone = SubtitlePair::new(
        "Synthetic local".into(),
        "Synthetic local translation".into(),
        160,
    );
    microphone.audio_source = AudioSource::Microphone;
    history.append_pair(&id, &system).unwrap();
    history.append_pair(&id, &microphone).unwrap();
    history
        .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0, 2, 0])
        .unwrap();
    history
        .append_pcm_from(&id, AudioSource::Microphone, 24_000, &[3, 0, 4, 0])
        .unwrap();
    for finalized in [false, true] {
        if finalized {
            assert!(history.finalize(&id, 200, false).unwrap());
        }
        let items = history.list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].audio_sources,
            [AudioSource::System, AudioSource::Microphone]
        );
        assert_eq!(items[0].count, 2);
        let page = history.page(&id, "", 0).unwrap();
        assert_eq!(page.entries[0].audio_source, AudioSource::Microphone);
        assert_eq!(page.entries[1].audio_source, AudioSource::System);
        let wire = serde_json::to_value(&page).unwrap();
        assert_eq!(wire["entries"][0]["audioSource"], "microphone");
        assert_eq!(
            history.audio_selected(&id, None).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let system_wav = history.audio_from(&id, AudioSource::System).unwrap();
        let mic_wav = history.audio_from(&id, AudioSource::Microphone).unwrap();
        assert_eq!(&system_wav[24..28], &16_000_u32.to_le_bytes());
        assert_eq!(&system_wav[44..], &[1, 0, 2, 0]);
        assert_eq!(&mic_wav[24..28], &24_000_u32.to_le_bytes());
        assert_eq!(&mic_wav[44..], &[3, 0, 4, 0]);
        let transcript = String::from_utf8(history.export_transcript(&id).unwrap()).unwrap();
        assert!(transcript.contains("System audio"));
        assert!(transcript.contains("Microphone"));
    }
    history.delete(&id).unwrap();
    assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
}

#[test]
fn microphone_only_history_is_selected_without_a_system_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pcm_from(&id, AudioSource::Microphone, 24_000, &[3, 0])
        .unwrap();
    assert_eq!(
        history.audio_sources(&id).unwrap(),
        [AudioSource::Microphone]
    );
    assert_eq!(&history.audio_selected(&id, None).unwrap()[44..], &[3, 0]);
    assert!(history
        .audio_selected(&id, Some(AudioSource::System))
        .is_err());
    assert!(history.finalize(&id, 200, false).unwrap());
    assert_eq!(&history.audio_selected(&id, None).unwrap()[44..], &[3, 0]);
    history.delete(&id).unwrap();
}

#[test]
fn disabling_recording_and_discard_remove_every_current_audio_track() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    for discard in [false, true] {
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        for source in [AudioSource::System, AudioSource::Microphone] {
            history
                .append_pcm_from(&id, source, 16_000, &[1, 0])
                .unwrap();
        }
        if discard {
            history.discard(&id).unwrap();
            assert!(!history.path(&id, "jsonl").unwrap().exists());
        } else {
            history.clear_audio(&id).unwrap();
            assert!(!history.finalize(&id, 200, false).unwrap());
        }
        assert!(history.audio_sources(&id).unwrap().is_empty());
        assert!(history.audio_from(&id, AudioSource::System).is_err());
        assert!(history.audio_from(&id, AudioSource::Microphone).is_err());
    }
    assert_eq!(fs::read_dir(&history.directory).unwrap().count(), 0);
}

#[test]
fn incompatible_audio_chunks_cannot_corrupt_either_track() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0])
        .unwrap();
    assert!(history
        .append_pcm_from(&id, AudioSource::System, 24_000, &[2, 0])
        .is_err());
    assert!(history
        .append_pcm_from(&id, AudioSource::Microphone, 16_000, &[3])
        .is_err());
    assert!(history
        .append_pcm_from(&id, AudioSource::Microphone, 48_000, &[4, 0])
        .is_err());
    assert_eq!(history.audio_sources(&id).unwrap(), [AudioSource::System]);
    assert_eq!(&history.audio_selected(&id, None).unwrap()[44..], &[1, 0]);
}

#[test]
fn audio_size_limit_applies_to_both_tracks_together() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0])
        .unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(history.audio_path(&id, AudioSource::System, "pcm").unwrap())
        .unwrap()
        .set_len(AUDIO_BYTE_LIMIT as u64 + 4)
        .unwrap();
    assert!(history
        .append_pcm_from(&id, AudioSource::Microphone, 16_000, &[2, 0])
        .is_err());
    assert_eq!(history.audio_sources(&id).unwrap(), [AudioSource::System]);
}

fn pending_audio(history: &SessionHistory, source: AudioSource, rate: u32) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 100).unwrap();
    history
        .append_pair(
            &id,
            &SubtitlePair::new(
                "Synthetic original".into(),
                "Synthetic translation".into(),
                150,
            ),
        )
        .unwrap();
    history
        .append_pcm_from(&id, source, rate, &[1, 0, 2, 0])
        .unwrap();
    id
}

fn synthetic_wav(rate: u32) -> Vec<u8> {
    let mut wav = wav_header(rate, 4).to_vec();
    wav.extend_from_slice(&[1, 0, 2, 0]);
    wav
}

fn assert_pending_audio(history: &SessionHistory, id: &str, source: AudioSource, rate: u32) {
    let mut expected = rate.to_le_bytes().to_vec();
    expected.extend_from_slice(&[1, 0, 2, 0]);
    assert_eq!(
        fs::read(history.audio_path(id, source, "pcm").unwrap()).unwrap(),
        expected
    );
    assert!(history.path(id, "jsonl").unwrap().is_file());
    assert!(!history.path(id, "json").unwrap().exists());
}

#[test]
fn pending_audio_export_uses_pcm_after_a_failed_wav_finalization() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let mut stale_wav = synthetic_wav(16_000);
    stale_wav[44] ^= 1;
    for source in [AudioSource::System, AudioSource::Microphone] {
        for damaged_wav in [
            Vec::new(),
            b"damaged audio".to_vec(),
            synthetic_wav(24_000),
            stale_wav.clone(),
        ] {
            let id = pending_audio(&history, source, 16_000);
            let wav_path = history.audio_path(&id, source, "wav").unwrap();
            fs::write(&wav_path, &damaged_wav).unwrap();
            assert!(history.finalize(&id, 200, false).is_err());
            assert_pending_audio(&history, &id, source, 16_000);

            // A failed conversion must not make its output shadow the
            // complete original journal in playback or manual export.
            assert_eq!(
                history.audio_selected(&id, Some(source)).unwrap(),
                synthetic_wav(16_000)
            );
            assert_eq!(fs::read(&wav_path).unwrap(), damaged_wav);
            assert_pending_audio(&history, &id, source, 16_000);
            history.clear_audio(&id).unwrap();
            assert!(history.audio_selected(&id, Some(source)).is_err());
            assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
        }
    }
}

#[test]
fn invalid_pending_pcm_cannot_fall_back_to_an_existing_wav() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    for source in [AudioSource::System, AudioSource::Microphone] {
        let id = pending_audio(&history, source, 16_000);
        fs::write(
            history.audio_path(&id, source, "wav").unwrap(),
            synthetic_wav(16_000),
        )
        .unwrap();
        let pcm_path = history.audio_path(&id, source, "pcm").unwrap();
        let mut odd_pcm = 16_000_u32.to_le_bytes().to_vec();
        odd_pcm.push(1);
        for invalid_pcm in [48_000_u32.to_le_bytes().to_vec(), odd_pcm] {
            fs::write(&pcm_path, &invalid_pcm).unwrap();
            assert!(history.audio_from(&id, source).is_err());
            assert_eq!(fs::read(&pcm_path).unwrap(), invalid_pcm);
        }
    }
}

#[test]
fn finalization_preserves_pcm_when_wav_output_is_not_a_file() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    for source in [AudioSource::System, AudioSource::Microphone] {
        let id = pending_audio(&history, source, 16_000);
        let wav = history.audio_path(&id, source, "wav").unwrap();
        fs::create_dir(&wav).unwrap();
        assert!(history.finalize(&id, 200, false).is_err());
        assert_pending_audio(&history, &id, source, 16_000);
        fs::remove_dir(&wav).unwrap();
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(
            history.audio_from(&id, source).unwrap(),
            synthetic_wav(16_000)
        );
    }
}

#[test]
fn finalization_rejects_invalid_existing_wav_without_consuming_pcm() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    for source in [AudioSource::System, AudioSource::Microphone] {
        for damage in [
            "empty",
            "header",
            "format",
            "channels",
            "rate",
            "bits",
            "data_length",
            "truncated",
            "trailing",
            "different_pcm_length",
            "different_pcm",
            "oversized",
        ] {
            let id = pending_audio(&history, source, 16_000);
            let path = history.audio_path(&id, source, "wav").unwrap();
            let mut wav = synthetic_wav(16_000);
            match damage {
                "empty" => wav.clear(),
                "header" => wav[0] = b'X',
                "format" => wav[20..22].copy_from_slice(&3_u16.to_le_bytes()),
                "channels" => wav[22..24].copy_from_slice(&2_u16.to_le_bytes()),
                "rate" => wav = synthetic_wav(24_000),
                "bits" => wav[34..36].copy_from_slice(&8_u16.to_le_bytes()),
                "data_length" => wav[40..44].copy_from_slice(&2_u32.to_le_bytes()),
                "truncated" => wav.truncate(wav.len() - 2),
                "trailing" => wav.extend_from_slice(&[3, 0]),
                "different_pcm_length" => {
                    wav = wav_header(16_000, 2).to_vec();
                    wav.extend_from_slice(&[1, 0]);
                }
                "different_pcm" => wav[44] ^= 1,
                "oversized" => {}
                _ => unreachable!(),
            }
            fs::write(&path, &wav).unwrap();
            if damage == "oversized" {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(AUDIO_BYTE_LIMIT as u64 + 45)
                    .unwrap();
            }
            assert!(
                history.finalize(&id, 200, false).is_err(),
                "source={source:?} damage={damage}"
            );
            assert_pending_audio(&history, &id, source, 16_000);
            fs::remove_file(&path).unwrap();
            assert!(history.finalize(&id, 200, false).unwrap());
            assert_eq!(
                history.audio_from(&id, source).unwrap(),
                synthetic_wav(16_000)
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn finalization_rejects_wav_symlinks_without_touching_saved_audio() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let saved_id = pending_audio(&history, AudioSource::System, 16_000);
    history.finalize(&saved_id, 200, false).unwrap();
    let saved_wav = history
        .audio_path(&saved_id, AudioSource::System, "wav")
        .unwrap();
    for source in [AudioSource::System, AudioSource::Microphone] {
        for target in [&saved_wav, &directory.path().join("missing.wav")] {
            let id = pending_audio(&history, source, 16_000);
            let path = history.audio_path(&id, source, "wav").unwrap();
            std::os::unix::fs::symlink(target, &path).unwrap();
            assert!(history.finalize(&id, 200, false).is_err());
            assert_pending_audio(&history, &id, source, 16_000);
            assert!(fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(history.audio(&saved_id).unwrap(), synthetic_wav(16_000));
            fs::remove_file(&path).unwrap();
            assert!(history.finalize(&id, 200, false).unwrap());
        }
    }
}

#[test]
fn finalization_reuses_valid_wav_from_an_interrupted_conversion() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    for (source, rate) in [
        (AudioSource::System, 16_000),
        (AudioSource::Microphone, 24_000),
    ] {
        let id = pending_audio(&history, source, rate);
        let path = history.audio_path(&id, source, "wav").unwrap();
        let wav = synthetic_wav(rate);
        fs::write(&path, &wav).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(history.audio_from(&id, source).unwrap(), wav);
        assert!(!history.audio_path(&id, source, "pcm").unwrap().exists());
        assert!(!history.path(&id, "jsonl").unwrap().exists());
        assert_eq!(history.page(&id, "", 0).unwrap().total, 1);
    }
}

#[test]
fn finalization_compares_existing_wav_payload_through_the_last_chunk() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let pcm = [1, 0].repeat(8193);
    for (source, rate) in [
        (AudioSource::System, 16_000),
        (AudioSource::Microphone, 24_000),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 100).unwrap();
        history.append_pcm_from(&id, source, rate, &pcm).unwrap();
        let path = history.audio_path(&id, source, "wav").unwrap();
        let mut wav = wav_header(rate, pcm.len() as u32).to_vec();
        wav.extend_from_slice(&pcm);
        *wav.last_mut().unwrap() ^= 1;
        fs::write(&path, &wav).unwrap();
        assert!(history.finalize(&id, 200, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), wav);
        let mut expected_pcm = rate.to_le_bytes().to_vec();
        expected_pcm.extend_from_slice(&pcm);
        assert_eq!(
            fs::read(history.audio_path(&id, source, "pcm").unwrap()).unwrap(),
            expected_pcm
        );
        assert!(history.path(&id, "jsonl").unwrap().is_file());
        assert!(!history.path(&id, "json").unwrap().exists());
        *wav.last_mut().unwrap() ^= 1;
        fs::write(&path, &wav).unwrap();
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(history.audio_from(&id, source).unwrap(), wav);
    }
}

#[test]
fn finalization_retries_dual_input_after_one_wav_output_is_blocked() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    for existing_system_wav in [false, true] {
        let id = pending_audio(&history, AudioSource::System, 16_000);
        history
            .append_pcm_from(&id, AudioSource::Microphone, 24_000, &[1, 0, 2, 0])
            .unwrap();
        let system = history.audio_path(&id, AudioSource::System, "wav").unwrap();
        let microphone = history
            .audio_path(&id, AudioSource::Microphone, "wav")
            .unwrap();
        if existing_system_wav {
            fs::write(&system, synthetic_wav(16_000)).unwrap();
        }
        fs::create_dir(&microphone).unwrap();
        assert!(history.finalize(&id, 200, false).is_err());
        assert_pending_audio(&history, &id, AudioSource::System, 16_000);
        assert_pending_audio(&history, &id, AudioSource::Microphone, 24_000);
        assert_eq!(system.is_file(), existing_system_wav);
        fs::remove_dir(&microphone).unwrap();
        assert!(history.finalize(&id, 200, false).unwrap());
        assert_eq!(
            history.audio_from(&id, AudioSource::System).unwrap(),
            synthetic_wav(16_000)
        );
        assert_eq!(
            history.audio_from(&id, AudioSource::Microphone).unwrap(),
            synthetic_wav(24_000)
        );
        assert!(history.audio_selected(&id, None).is_err());
    }
}

#[test]
fn finalization_never_replaces_an_existing_saved_record() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let id = pending_audio(&history, AudioSource::System, 16_000);
    history.finalize(&id, 200, false).unwrap();
    let saved = fs::read(history.path(&id, "json").unwrap()).unwrap();
    let wav = history.audio(&id).unwrap();
    history.begin(&id, 300).unwrap();
    history
        .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0, 2, 0])
        .unwrap();
    assert!(history.finalize(&id, 400, false).is_err());
    assert_eq!(fs::read(history.path(&id, "json").unwrap()).unwrap(), saved);
    assert_eq!(history.audio(&id).unwrap(), wav);
    assert!(history.path(&id, "jsonl").unwrap().is_file());
    assert!(history
        .audio_path(&id, AudioSource::System, "pcm")
        .unwrap()
        .is_file());
}

#[test]
fn batch_delete_removes_requested_sessions_and_preserves_unselected_and_protected_records() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), false);
    let save = |started: u64, title: Option<&str>| {
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, started).unwrap();
        history
            .append_pair(
                &id,
                &SubtitlePair::new(
                    "Synthetic source".into(),
                    "Synthetic result".into(),
                    started + 50,
                ),
            )
            .unwrap();
        history
            .append_pcm_from(&id, AudioSource::System, 16_000, &[1, 0, 2, 0])
            .unwrap();
        history.finalize(&id, started + 100, false).unwrap();
        if let Some(title) = title {
            history.rename(&id, title).unwrap();
        }
        id
    };
    let first = save(100, Some("First"));
    let second = save(200, Some("Second"));
    let keep = save(300, Some("Keep"));
    let active = uuid::Uuid::new_v4().to_string();
    history.begin(&active, 400).unwrap();
    history
        .append_pair(
            &active,
            &SubtitlePair::new("Synthetic active".into(), "Synthetic active".into(), 450),
        )
        .unwrap();
    let missing = uuid::Uuid::new_v4().to_string();

    let report = history
        .delete_many(
            &[
                first.as_str(),
                second.as_str(),
                first.as_str(),
                active.as_str(),
                missing.as_str(),
                "not-a-uuid",
            ],
            &[active.as_str()],
        )
        .unwrap();

    assert_eq!(report.deleted, vec![first.clone(), second.clone()]);
    assert_eq!(
        report.failed,
        vec![active.clone(), missing, "not-a-uuid".to_owned()]
    );
    assert_eq!(
        report.failure_reason,
        Some(HistoryDeleteFailureReason::ActiveSession)
    );
    for removed in [&first, &second] {
        assert!(!history.path(removed, "json").unwrap().exists());
        assert!(!history.path(removed, "meta.json").unwrap().exists());
        assert!(history.audio_sources(removed).unwrap().is_empty());
    }
    assert!(history.path(&keep, "json").unwrap().is_file());
    assert!(history.path(&active, "jsonl").unwrap().is_file());
    let listed_ids: Vec<String> = history
        .list()
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(listed_ids, vec![active, keep]);
}

#[test]
fn batch_delete_fails_when_history_is_disabled() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().join("history"), true);
    let id = uuid::Uuid::new_v4().to_string();
    let error = history
        .delete_many(&[id.as_str()], &[] as &[&str])
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}
