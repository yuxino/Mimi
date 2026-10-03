//! Explicit test-only private ASR evidence; never compiled into the application.
use super::{task_failure, Audio3ASRServerEvent, Failure, Report};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_LINE_BYTES: usize = 512 * 1024;
const MAX_EVENTS_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn validate_root(root: &Path) -> Result<(), Failure> {
    validate_root_under(root, Path::new("/private/tmp/mimi-debug-benchmark"))
}

fn validate_root_under(root: &Path, allowed: &Path) -> Result<(), Failure> {
    if !root.is_absolute()
        || !root.starts_with(allowed)
        || root
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(Failure::EvidenceFailed);
    }
    std::fs::create_dir_all(root).map_err(|_| Failure::EvidenceFailed)?;
    let metadata = std::fs::symlink_metadata(root).map_err(|_| Failure::EvidenceFailed)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !root
            .canonicalize()
            .map_err(|_| Failure::EvidenceFailed)?
            .starts_with(allowed)
    {
        return Err(Failure::EvidenceFailed);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Failure::EvidenceFailed)?;
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<File, Failure> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| Failure::EvidenceFailed)
}
fn write_json(mut file: File, value: &Value) -> Result<(), Failure> {
    serde_json::to_writer(&mut file, value).map_err(|_| Failure::EvidenceFailed)?;
    file.write_all(b"\n").map_err(|_| Failure::EvidenceFailed)?;
    file.sync_all().map_err(|_| Failure::EvidenceFailed)
}

pub(super) struct Evidence {
    directory: PathBuf,
    events: File,
    bytes: usize,
}
impl Evidence {
    pub(super) fn create(root: &Path, label: &str) -> Result<Self, Failure> {
        validate_root(root)?;
        Self::create_in_validated_root(root, label)
    }

    fn create_in_validated_root(root: &Path, label: &str) -> Result<Self, Failure> {
        if !super::valid_label(label) {
            return Err(Failure::EvidenceFailed);
        }
        let directory = root.join(format!("{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).map_err(|_| Failure::EvidenceFailed)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| Failure::EvidenceFailed)?;
        }
        let events = private_file(&directory.join("events.jsonl"))?;
        Ok(Self {
            directory,
            events,
            bytes: 0,
        })
    }
    pub(super) fn request(&mut self, request: &Value) -> Result<(), Failure> {
        // Only the production encoder's task payload is supplied here. HTTP
        // headers and endpoint/credentials never enter this writer.
        write_json(private_file(&self.directory.join("request.json"))?, request)
    }
    pub(super) fn event(
        &mut self,
        raw: &str,
        event: &Audio3ASRServerEvent,
        at_ms: u64,
        after_finish: bool,
    ) -> Result<(), Failure> {
        let mut bytes = serde_json::to_vec(&event_value(raw, event, at_ms, after_finish)?)
            .map_err(|_| Failure::EvidenceFailed)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_LINE_BYTES || self.bytes + bytes.len() > MAX_EVENTS_BYTES {
            return Err(Failure::EvidenceFailed);
        }
        self.events
            .write_all(&bytes)
            .map_err(|_| Failure::EvidenceFailed)?;
        self.bytes += bytes.len();
        Ok(())
    }
    pub(super) fn finish(&mut self, report: &Report) -> Result<(), Failure> {
        self.events
            .sync_all()
            .map_err(|_| Failure::EvidenceFailed)?;
        write_json(
            private_file(&self.directory.join("metrics.json"))?,
            &serde_json::to_value(report).map_err(|_| Failure::EvidenceFailed)?,
        )
    }
}

fn event_value(
    raw: &str,
    event: &Audio3ASRServerEvent,
    at_ms: u64,
    after_finish: bool,
) -> Result<Value, Failure> {
    let mut value = json!({"elapsedMs":at_ms,"afterFinishSent":after_finish});
    value["kind"] = Value::String(
        match event {
            Audio3ASRServerEvent::TaskStarted => "taskStarted",
            Audio3ASRServerEvent::TaskFinished => "taskFinished",
            Audio3ASRServerEvent::Heartbeat => "heartbeat",
            Audio3ASRServerEvent::Ignored { .. } => "ignored",
            Audio3ASRServerEvent::TaskFailed { .. } => "taskFailed",
            Audio3ASRServerEvent::Transcription { .. } => "transcription",
        }
        .into(),
    );
    if let Audio3ASRServerEvent::Transcription {
        text,
        is_final,
        sentence_id,
    } = event
    {
        let raw: Value = serde_json::from_str(raw).map_err(|_| Failure::ProtocolInvalid)?;
        let sentence = raw
            .pointer("/payload/output/sentence")
            .ok_or(Failure::ProtocolInvalid)?;
        let time = |v: &Value, key: &str| {
            v.get(key)
                .and_then(Value::as_u64)
                .filter(|n| *n <= 1_000_000_000)
        };
        value["text"] = json!(text);
        value["isFinal"] = json!(is_final);
        value["sentenceId"] = json!(sentence_id);
        value["beginTimeMs"] = json!(time(sentence, "begin_time"));
        value["endTimeMs"] = json!(time(sentence, "end_time"));
        value["sentenceBegin"] = json!(sentence.get("sentence_begin").and_then(Value::as_bool));
        let words: Vec<Value> = sentence.get("words").and_then(Value::as_array).into_iter().flatten()
            .take(4096).filter_map(|word| {
                let text = word.get("text")?.as_str()?;
                if text.len() > super::MAX_TEXT_BYTES { return None; }
                Some(json!({"text":text,"punctuation":word.get("punctuation").and_then(Value::as_str).filter(|p| p.len()<=256),"beginTimeMs":time(word,"begin_time"),"endTimeMs":time(word,"end_time")}))
            }).collect();
        value["words"] = json!(words);
        value["wordsLimited"] = json!(sentence
            .get("words")
            .and_then(Value::as_array)
            .is_some_and(|words| words.len() > 4096));
    } else if let Some(failure) = task_failure(event) {
        value["failure"] = json!(failure);
    }
    Ok(value)
}

#[test]
fn evidence_keeps_exact_caption_clocks_and_excludes_arbitrary_errors() {
    let raw = r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":3,"sentence_end":true,"begin_time":40,"end_time":140,"text":"synthetic reference","words":[{"text":"synthetic","begin_time":40,"end_time":90,"Authorization":"secret"}]}}}}"#;
    let event = super::Audio3ASRServerEventDecoder::decode(raw).unwrap();
    let value = event_value(raw, &event, 300, true).unwrap();
    assert_eq!(value["text"], "synthetic reference");
    assert_eq!(value["sentenceId"], 3);
    assert_eq!(value["beginTimeMs"], 40);
    assert_eq!(value["words"][0]["endTimeMs"], 90);
    assert!(!value.to_string().contains("secret"));
    let error = Audio3ASRServerEvent::TaskFailed {
        code: "secret".into(),
        message: "secret".into(),
    };
    let value = event_value("secret", &error, 400, false).unwrap();
    assert_eq!(value["failure"], "task_rejected");
    assert!(!value.to_string().contains("secret"));
}

#[test]
fn evidence_is_private_and_rejects_outside_output_roots() {
    assert!(validate_root(Path::new("/private/tmp/outside-benchmark")).is_err());
    assert!(validate_root(Path::new(
        "/private/tmp/mimi-debug-benchmark/../outside-benchmark"
    ))
    .is_err());
    // The real harness stays restricted to the explicit macOS evidence root.
    // Exercise the same filesystem checks/writer in a runner-owned directory:
    // /private/tmp is neither an absolute Windows path nor writable on Linux CI.
    let allowed = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "mimi-private-evidence-unit-{}",
        uuid::Uuid::new_v4()
    ));
    let root = allowed.join("evidence");
    assert!(validate_root(&root).is_err());
    assert!(Evidence::create(&root, "fixture").is_err());
    assert!(validate_root_under(&allowed.with_extension("outside"), &allowed).is_err());
    assert!(validate_root_under(&root.join("../outside"), &allowed).is_err());
    validate_root_under(&root, &allowed).unwrap();
    let evidence = Evidence::create_in_validated_root(&root, "fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&evidence.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(evidence.directory.join("events.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(evidence);
    std::fs::remove_dir_all(allowed).unwrap();
}
