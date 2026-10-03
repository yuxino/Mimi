//! Manually opted-in, content-free ASR comparison. Never compiled into the app.
//! This deliberately bypasses HQ/MT and the replaceable draft watch lane: it
//! measures recognizer events, not capture-to-screen latency or UI correctness.

use crate::clients::provider_network::{self, ProviderNetwork};
use crate::core::models::SourceLanguage;
use crate::core::protocols::audio3::{
    Audio3ASRContext, Audio3ASREndpoint, Audio3ASRRequestEncoder, Audio3ASRServerEvent,
    Audio3ASRServerEventDecoder, MAX_AUDIO3_MESSAGE_BYTES,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

mod denoising;
mod realtime;

const FRAME_BYTES: usize = 640; // 20 ms, mono signed little-endian PCM16, 16 kHz.
const BYTES_PER_SECOND: usize = 32_000;
const TAIL_FRAMES: usize = 100; // Exactly 2 seconds; identical in every arm.
const MAX_PCM_BYTES: usize = 60 * BYTES_PER_SECOND;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_REFERENCE_UNITS: usize = 4096;
const MAX_HYPOTHESIS_UNITS: usize = 8192;
const MAX_EVENTS: u64 = 20_000;
const MAX_FINAL_SAMPLES: usize = 512;
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Only fixed classifications cross the output boundary, never underlying
/// serde/filesystem/WebSocket/provider errors (which may contain private data).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Failure {
    ManifestInvalid,
    InputInvalid,
    CredentialsUnavailable,
    NetworkFailed,
    AuthenticationRejected,
    TaskRejected,
    ReadyTimeout,
    SendTimeout,
    FinishTimeout,
    ProtocolInvalid,
    EventLimit,
    EvaluationLimit,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Language {
    #[serde(alias = "en")]
    English,
    #[serde(alias = "ja")]
    Japanese,
    #[serde(alias = "zh")]
    Chinese,
    #[serde(rename = "auto")]
    Automatic,
}

impl Language {
    fn source(self) -> SourceLanguage {
        match self {
            Self::English => SourceLanguage::English,
            Self::Japanese => SourceLanguage::Japanese,
            Self::Chinese => SourceLanguage::Chinese,
            Self::Automatic => SourceLanguage::Automatic,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Unit {
    Word,
    Char,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    clips: Vec<Clip>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Clip {
    label: String,
    pcm_path: PathBuf,
    reference_path: Option<PathBuf>,
    language: Language,
    unit: Unit,
    #[serde(default)]
    preparation: FixturePreparation,
}

/// Fixed labels for independently prepared public PCM; never arbitrary paths
/// or user-provided descriptions in the diagnostic report.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum FixturePreparation {
    #[default]
    Raw,
    ResampleRoundtrip,
    Deepfilter12,
}

struct PreparedClip {
    label: String,
    pcm: Vec<u8>,
    reference: Option<Vec<String>>,
    language: Language,
    unit: Unit,
    preparation: FixturePreparation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Arm {
    Baseline,
    NoContext,
    Model31,
    RealtimeAsr,
}

impl Arm {
    fn parse(value: &str) -> Result<Self, Failure> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "no-context" => Ok(Self::NoContext),
            "model31" => Ok(Self::Model31),
            "realtime-asr" => Ok(Self::RealtimeAsr),
            _ => Err(Failure::ManifestInvalid),
        }
    }

    fn request(self, source: SourceLanguage, task_id: &str) -> Result<serde_json::Value, Failure> {
        if matches!(self, Self::RealtimeAsr) {
            return Err(Failure::ProtocolInvalid);
        }
        let context =
            matches!(self, Self::Baseline).then(|| Audio3ASRContext::audiovisual_dialogue(source));
        let mut request = Audio3ASRRequestEncoder::run_task(task_id, source, context)
            .map_err(|_| Failure::ProtocolInvalid)?;
        if matches!(self, Self::Model31) {
            // Test-only allowlisted override; the production encoder remains 3.0.
            request["payload"]["model"] = "qwen-audio-3.1-asr-flash-streaming".into();
        }
        Ok(request)
    }
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 48
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_manifest(bytes: &[u8]) -> Result<Manifest, Failure> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(Failure::ManifestInvalid);
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| Failure::ManifestInvalid)?;
    if manifest.clips.is_empty() || manifest.clips.len() > 10 {
        return Err(Failure::ManifestInvalid);
    }
    let mut labels = std::collections::HashSet::new();
    for clip in &manifest.clips {
        if !valid_label(&clip.label)
            || !labels.insert(&clip.label)
            || !clip.pcm_path.is_absolute()
            || !matches!(
                clip.pcm_path.extension().and_then(|part| part.to_str()),
                Some("pcm" | "s16le")
            )
            || clip.reference_path.as_ref().is_some_and(|path| {
                !path.is_absolute()
                    || path.extension().and_then(|part| part.to_str()) != Some("txt")
            })
        {
            return Err(Failure::ManifestInvalid);
        }
    }
    Ok(manifest)
}

/// Public fixture files must stay within the manifest directory. Refuse links,
/// special files and oversized reads; do not let a manifest reference `.env`.
fn bounded_file(path: &Path, maximum: usize) -> Result<Vec<u8>, Failure> {
    let before = std::fs::symlink_metadata(path).map_err(|_| Failure::InputInvalid)?;
    if !before.is_file() || before.len() > maximum as u64 {
        return Err(Failure::InputInvalid);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        options.custom_flags(0x0100 | 0x0004); // O_NOFOLLOW | O_NONBLOCK.
        #[cfg(not(target_os = "macos"))]
        options.custom_flags(0x20000 | 0x0800);
    }
    let file = options.open(path).map_err(|_| Failure::InputInvalid)?;
    let after = file.metadata().map_err(|_| Failure::InputInvalid)?;
    if !after.is_file() || after.len() > maximum as u64 {
        return Err(Failure::InputInvalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ino() != after.ino() || before.dev() != after.dev() {
            return Err(Failure::InputInvalid);
        }
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::InputInvalid)?;
    if bytes.len() > maximum {
        return Err(Failure::InputInvalid);
    }
    Ok(bytes)
}

fn prepare_manifest(path: &Path) -> Result<Vec<PreparedClip>, Failure> {
    if !path.is_absolute() || path.extension().and_then(|part| part.to_str()) != Some("json") {
        return Err(Failure::ManifestInvalid);
    }
    let manifest = parse_manifest(&bounded_file(path, MAX_MANIFEST_BYTES)?)?;
    let directory = path
        .parent()
        .ok_or(Failure::ManifestInvalid)?
        .canonicalize()
        .map_err(|_| Failure::InputInvalid)?;
    let mut prepared = Vec::new();
    for clip in manifest.clips {
        let read = |path: &Path, maximum| {
            let canonical = path.canonicalize().map_err(|_| Failure::InputInvalid)?;
            if !canonical.starts_with(&directory) {
                return Err(Failure::InputInvalid);
            }
            bounded_file(path, maximum)
        };
        let pcm = read(&clip.pcm_path, MAX_PCM_BYTES)?;
        if pcm.is_empty() || !pcm.len().is_multiple_of(2) {
            return Err(Failure::InputInvalid);
        }
        let reference = clip
            .reference_path
            .as_ref()
            .map(|path| {
                let text = String::from_utf8(read(path, MAX_TEXT_BYTES)?)
                    .map_err(|_| Failure::InputInvalid)?;
                let units = normalize(&text, clip.unit);
                if units.is_empty() || units.len() > MAX_REFERENCE_UNITS {
                    return Err(Failure::InputInvalid);
                }
                Ok(units)
            })
            .transpose()?;
        prepared.push(PreparedClip {
            label: clip.label,
            pcm,
            reference,
            language: clip.language,
            unit: clip.unit,
            preparation: clip.preparation,
        });
    }
    Ok(prepared)
}

fn normalize(text: &str, unit: Unit) -> Vec<String> {
    let cleaned = |word: &str| {
        word.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    match unit {
        Unit::Word => text
            .split_whitespace()
            .map(cleaned)
            .filter(|word| !word.is_empty())
            .collect(),
        Unit::Char => text
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .map(|character| character.to_string())
            .collect(),
    }
}

/// Online Levenshtein distance consumes only authoritative final units. It
/// stores the public reference and two numeric rows, never a full hypothesis.
struct EditDistance {
    reference: Vec<String>,
    previous: Vec<EditCounts>,
    next: Vec<EditCounts>,
    hypothesis_units: u64,
}

#[derive(Clone, Copy, Default)]
struct EditCounts {
    substitutions: u64,
    insertions: u64,
    deletions: u64,
}

impl EditCounts {
    fn distance(self) -> u64 {
        self.substitutions + self.insertions + self.deletions
    }
}

impl EditDistance {
    fn new(reference: Vec<String>) -> Self {
        Self {
            previous: (0..=reference.len() as u64)
                .map(|deletions| EditCounts {
                    deletions,
                    ..EditCounts::default()
                })
                .collect(),
            next: vec![EditCounts::default(); reference.len() + 1],
            reference,
            hypothesis_units: 0,
        }
    }

    fn push(&mut self, units: &[String]) -> Result<(), Failure> {
        if self.hypothesis_units + units.len() as u64 > MAX_HYPOTHESIS_UNITS as u64 {
            return Err(Failure::EvaluationLimit);
        }
        for unit in units {
            self.hypothesis_units += 1;
            self.next[0] = EditCounts {
                insertions: self.hypothesis_units,
                ..EditCounts::default()
            };
            for (index, reference) in self.reference.iter().enumerate() {
                let substitution = EditCounts {
                    substitutions: self.previous[index].substitutions
                        + u64::from(unit != reference),
                    ..self.previous[index]
                };
                let deletion = EditCounts {
                    deletions: self.next[index].deletions + 1,
                    ..self.next[index]
                };
                let insertion = EditCounts {
                    insertions: self.previous[index + 1].insertions + 1,
                    ..self.previous[index + 1]
                };
                // Multiple minimal alignments may exist. Choose deterministically
                // by fewer deletions, then insertions; these are reference edits,
                // not a semantic classification of unheard speech.
                self.next[index + 1] = [substitution, deletion, insertion]
                    .into_iter()
                    .min_by_key(|counts| (counts.distance(), counts.deletions, counts.insertions))
                    .unwrap();
            }
            std::mem::swap(&mut self.previous, &mut self.next);
        }
        Ok(())
    }

    fn result(&self) -> Evaluation {
        let counts = *self.previous.last().unwrap();
        let distance = counts.distance();
        Evaluation {
            edit_distance: distance,
            substitutions: counts.substitutions,
            insertions: counts.insertions,
            deletions: counts.deletions,
            reference_units: self.reference.len() as u64,
            hypothesis_units: self.hypothesis_units,
            error_rate: distance as f64 / self.reference.len() as f64,
        }
    }
}

#[derive(Serialize)]
struct Evaluation {
    edit_distance: u64,
    substitutions: u64,
    insertions: u64,
    deletions: u64,
    reference_units: u64,
    hypothesis_units: u64,
    error_rate: f64,
}

#[derive(Default, Serialize)]
struct Metrics {
    skipped: u64,
    realtime_stage: Option<realtime::Stage>,
    ready_ack_mismatch_mask: u64,
    audio_frames: u64,
    tail_frames: u64,
    sent_bytes: u64,
    maximum_send_lateness_ms: u64,
    task_started: u64,
    task_finished: u64,
    ready_ms: u64,
    elapsed_ms: u64,
    first_nonempty_ms: Option<u64>,
    first_final_ms: Option<u64>,
    first_stable_prefix_ms: Option<u64>,
    stable_prefix_events: u64,
    stable_prefix_changes: u64,
    stable_prefix_retractions: u64,
    stable_prefix_retracted_characters: u64,
    stable_prefix_retracted_han_characters: u64,
    stable_prefix_final_retractions: u64,
    stable_prefix_final_retracted_han_characters: u64,
    stable_prefix_latin_characters: u64,
    stable_prefix_han_characters: u64,
    stable_prefix_kana_characters: u64,
    stash_revisions: u64,
    stash_retractions: u64,
    stash_retracted_characters: u64,
    stash_latin_characters: u64,
    stash_han_characters: u64,
    stash_kana_characters: u64,
    draft_events: u64,
    draft_revisions: u64,
    draft_retracted_characters: u64,
    empty_sentence_begins: u64,
    final_events: u64,
    duplicate_final_ids: u64,
    identified_events: u64,
    missing_identity_events: u64,
    maximum_draft_characters: u64,
    maximum_final_characters: u64,
    draft_latin_characters: u64,
    draft_han_characters: u64,
    draft_kana_characters: u64,
    final_latin_characters: u64,
    final_han_characters: u64,
    final_kana_characters: u64,
    maximum_nonempty_gap_ms: u64,
    final_after_finish: u64,
    sentence_end_delay_p50_ms: Option<u64>,
    sentence_end_delay_p95_ms: Option<u64>,
    sentence_end_delay_max_ms: Option<u64>,
    speech_stop_to_final_p50_ms: Option<u64>,
    speech_stop_to_final_p95_ms: Option<u64>,
    speech_stop_to_final_max_ms: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    billed_duration_seconds: Option<u64>,
    finals: Vec<FinalSample>,
}

#[derive(Serialize)]
struct FinalSample {
    sentence_id: Option<u64>,
    begin_time_ms: Option<u64>,
    end_time_ms: Option<u64>,
    speech_stop_to_final_ms: Option<u64>,
    received_ms: u64,
    source_characters: u64,
    after_finish: u64,
    duplicate_id: u64,
    language_hint: Language,
    character_classes: CharacterClasses,
}

#[derive(Default, Serialize)]
struct CharacterClasses {
    latin: u64,
    han: u64,
    kana: u64,
}

fn character_classes(text: &str) -> CharacterClasses {
    let mut counts = CharacterClasses::default();
    for character in text.chars() {
        let number = character as u32;
        counts.latin += u64::from(
            character.is_alphabetic()
                && matches!(number,
            0x0041..=0x007a | 0x00c0..=0x024f | 0x1d00..=0x1dbf | 0x1e00..=0x1eff
            | 0xab30..=0xab6f | 0xff21..=0xff5a),
        );
        counts.han +=
            u64::from(matches!(number, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0x20000..=0x3134f));
        counts.kana += u64::from(
            character.is_alphabetic() && matches!(number, 0x3040..=0x30ff | 0xff66..=0xff9d),
        );
    }
    counts
}

#[derive(Serialize)]
struct Report {
    label: String,
    arm: Arm,
    input_processing: denoising::Kind,
    fixture_preparation: FixturePreparation,
    language: Language,
    unit: Unit,
    metrics: Metrics,
    evaluation: Option<Evaluation>,
    failure: Option<Failure>,
}

struct Observer {
    metrics: Metrics,
    evaluator: Option<EditDistance>,
    // Exactly one replaceable draft; no transcript/history accumulation.
    last_draft: Option<(Option<u64>, String)>,
    last_final_id: Option<u64>,
    last_nonempty_ms: Option<u64>,
    final_delays: Vec<u64>,
    events: u64,
    unit: Unit,
    language: Language,
}

/// Each protocol supplies its real identity/replay policy and optional audio
/// clocks. In particular, realtime opaque item IDs are not Audio3 watermarks.
struct CaptionObservation<'a> {
    caption: &'a str,
    is_final: bool,
    sentence_id: Option<u64>,
    duplicate_final: bool,
    begin_time_ms: Option<u64>,
    end_time_ms: Option<u64>,
}

impl Observer {
    fn new(clip: &PreparedClip) -> Self {
        Self {
            metrics: Metrics::default(),
            evaluator: clip.reference.clone().map(EditDistance::new),
            last_draft: None,
            last_final_id: None,
            last_nonempty_ms: None,
            final_delays: Vec::new(),
            events: 0,
            unit: clip.unit,
            language: clip.language,
        }
    }

    fn record(
        &mut self,
        text: &str,
        at_ms: u64,
        finished_sending: bool,
    ) -> Result<Audio3ASRServerEvent, Failure> {
        self.events += 1;
        if self.events > MAX_EVENTS {
            return Err(Failure::EventLimit);
        }
        let event =
            Audio3ASRServerEventDecoder::decode(text).map_err(|_| Failure::ProtocolInvalid)?;
        // Only bounded numeric upstream fields are observed; free-text error
        // values never cross the report boundary.
        let json: serde_json::Value =
            serde_json::from_str(text).map_err(|_| Failure::ProtocolInvalid)?;
        if let Audio3ASRServerEvent::Transcription {
            text: caption,
            is_final,
            sentence_id,
        } = &event
        {
            let duplicate_final = *is_final
                && sentence_id.is_some_and(|id| self.last_final_id.is_some_and(|last| id <= last));
            let numeric_time = |name| {
                json.pointer("/payload/output/sentence")
                    .and_then(|sentence| sentence.get(name))
                    .and_then(serde_json::Value::as_u64)
                    .filter(|value| *value <= 1_000_000_000)
            };
            self.observe_caption(
                CaptionObservation {
                    caption,
                    is_final: *is_final,
                    sentence_id: *sentence_id,
                    duplicate_final,
                    begin_time_ms: numeric_time("begin_time"),
                    end_time_ms: numeric_time("end_time"),
                },
                at_ms,
                finished_sending,
            )?;
            if *is_final && !duplicate_final {
                self.last_final_id = sentence_id.or(self.last_final_id);
            }
        }
        for (name, destination) in [
            ("input_tokens", &mut self.metrics.input_tokens),
            ("output_tokens", &mut self.metrics.output_tokens),
            ("duration", &mut self.metrics.billed_duration_seconds),
        ] {
            if let Some(value) = json
                .pointer("/payload/usage")
                .or_else(|| json.pointer("/payload/output/usage"))
                .and_then(|usage| usage.get(name))
                .and_then(serde_json::Value::as_u64)
                .filter(|value| *value <= 1_000_000_000_000)
            {
                *destination = Some(value);
            }
        }
        Ok(event)
    }

    fn observe_caption(
        &mut self,
        observation: CaptionObservation<'_>,
        at_ms: u64,
        finished_sending: bool,
    ) -> Result<(), Failure> {
        let CaptionObservation {
            caption,
            is_final,
            sentence_id,
            duplicate_final,
            begin_time_ms,
            end_time_ms,
        } = observation;
        if caption.len() > MAX_TEXT_BYTES {
            return Err(Failure::ProtocolInvalid);
        }
        let length = caption.chars().count() as u64;
        self.metrics.identified_events += u64::from(sentence_id.is_some());
        self.metrics.missing_identity_events += u64::from(sentence_id.is_none());
        if caption.is_empty() {
            self.metrics.empty_sentence_begins += 1;
        } else {
            self.metrics.first_nonempty_ms.get_or_insert(at_ms);
            if let Some(previous) = self.last_nonempty_ms {
                self.metrics.maximum_nonempty_gap_ms = self
                    .metrics
                    .maximum_nonempty_gap_ms
                    .max(at_ms.saturating_sub(previous));
            }
            self.last_nonempty_ms = Some(at_ms);
        }
        let classes = character_classes(caption);
        if is_final {
            if self.metrics.finals.len() == MAX_FINAL_SAMPLES {
                return Err(Failure::EventLimit);
            }
            self.metrics.final_events += 1;
            self.metrics.first_final_ms.get_or_insert(at_ms);
            self.metrics.maximum_final_characters =
                self.metrics.maximum_final_characters.max(length);
            self.metrics.final_after_finish += u64::from(finished_sending);
            self.metrics.duplicate_final_ids += u64::from(duplicate_final);
            if !duplicate_final {
                if let Some(evaluator) = &mut self.evaluator {
                    evaluator.push(&normalize(caption, self.unit))?;
                }
                self.metrics.final_latin_characters += classes.latin;
                self.metrics.final_han_characters += classes.han;
                self.metrics.final_kana_characters += classes.kana;
                if let Some(end) = end_time_ms.filter(|end| *end <= at_ms) {
                    self.final_delays.push(at_ms - end);
                }
            }
            if self
                .last_draft
                .as_ref()
                .is_some_and(|(owner, _)| *owner == sentence_id)
            {
                self.last_draft = None;
            }
            self.metrics.finals.push(FinalSample {
                sentence_id,
                begin_time_ms,
                end_time_ms,
                speech_stop_to_final_ms: None,
                received_ms: at_ms,
                source_characters: length,
                after_finish: u64::from(finished_sending),
                duplicate_id: u64::from(duplicate_final),
                language_hint: self.language,
                character_classes: classes,
            });
        } else {
            self.metrics.draft_events += 1;
            self.metrics.draft_latin_characters += classes.latin;
            self.metrics.draft_han_characters += classes.han;
            self.metrics.draft_kana_characters += classes.kana;
            self.metrics.maximum_draft_characters =
                self.metrics.maximum_draft_characters.max(length);
            if let Some((owner, previous)) = &self.last_draft {
                if *owner == sentence_id && previous != caption {
                    self.metrics.draft_revisions += 1;
                    let common = previous
                        .chars()
                        .zip(caption.chars())
                        .take_while(|(a, b)| a == b)
                        .count();
                    self.metrics.draft_retracted_characters +=
                        previous.chars().count().saturating_sub(common) as u64;
                }
            }
            self.last_draft = Some((sentence_id, caption.to_owned()));
        }
        Ok(())
    }

    fn report(mut self, clip: &PreparedClip, arm: Arm, failure: Option<Failure>) -> Report {
        self.final_delays.sort_unstable();
        let percentile = |percent: usize| {
            (!self.final_delays.is_empty()).then(|| {
                self.final_delays[(self.final_delays.len() * percent)
                    .div_ceil(100)
                    .saturating_sub(1)]
            })
        };
        self.metrics.sentence_end_delay_p50_ms = percentile(50);
        self.metrics.sentence_end_delay_p95_ms = percentile(95);
        self.metrics.sentence_end_delay_max_ms = self.final_delays.last().copied();
        let mut stop_delays = self
            .metrics
            .finals
            .iter()
            .filter(|sample| sample.duplicate_id == 0)
            .filter_map(|sample| sample.speech_stop_to_final_ms)
            .collect::<Vec<_>>();
        stop_delays.sort_unstable();
        let percentile = |percent: usize| {
            (!stop_delays.is_empty()).then(|| {
                stop_delays[(stop_delays.len() * percent)
                    .div_ceil(100)
                    .saturating_sub(1)]
            })
        };
        self.metrics.speech_stop_to_final_p50_ms = percentile(50);
        self.metrics.speech_stop_to_final_p95_ms = percentile(95);
        self.metrics.speech_stop_to_final_max_ms = stop_delays.last().copied();
        Report {
            label: clip.label.clone(),
            arm,
            input_processing: denoising::Kind::Bypass,
            fixture_preparation: clip.preparation,
            language: clip.language,
            unit: clip.unit,
            evaluation: if failure.is_none() {
                self.evaluator.as_ref().map(EditDistance::result)
            } else {
                None
            },
            metrics: self.metrics,
            failure,
        }
    }
}

fn websocket_failure(error: tokio_tungstenite::tungstenite::Error) -> Failure {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response)
            if matches!(response.status().as_u16(), 401 | 403) =>
        {
            Failure::AuthenticationRejected
        }
        _ => Failure::NetworkFailed,
    }
}

fn task_failure(event: &Audio3ASRServerEvent) -> Option<Failure> {
    match event {
        Audio3ASRServerEvent::TaskFailed { message, .. } if message == "authentication" => {
            Some(Failure::AuthenticationRejected)
        }
        Audio3ASRServerEvent::TaskFailed { .. } => Some(Failure::TaskRejected),
        _ => None,
    }
}

fn text_frame(message: Message) -> Result<Option<String>, Failure> {
    match message {
        Message::Text(text) => Ok(Some(text.to_string())),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec())
            .map(Some)
            .map_err(|_| Failure::ProtocolInvalid),
        Message::Ping(_) | Message::Pong(_) => Ok(None),
        Message::Close(_) => Err(Failure::NetworkFailed),
        Message::Frame(_) => Err(Failure::ProtocolInvalid),
    }
}

async fn compare(
    clip: &PreparedClip,
    arm: Arm,
    api_key: &str,
    network: &ProviderNetwork,
    endpoint: &str,
) -> Report {
    let mut observer = Observer::new(clip);
    let mut sent = Metrics::default();
    let operation = async {
        let task_id = uuid::Uuid::new_v4().to_string();
        let run_task = arm.request(clip.language.source(), &task_id)?;
        let mut request = endpoint.into_client_request().map_err(|_| Failure::NetworkFailed)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| Failure::CredentialsUnavailable)?;
        authorization.set_sensitive(true);
        request.headers_mut().insert("Authorization", authorization);
        let connecting = Instant::now();
        let (mut socket, _) = provider_network::websocket_with_message_limit(request, network, MAX_AUDIO3_MESSAGE_BYTES)
            .await.map_err(websocket_failure)?;
        tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(run_task.to_string().into())))
            .await.map_err(|_| Failure::SendTimeout)?.map_err(websocket_failure)?;
        let ready_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let message = tokio::time::timeout_at(ready_deadline, socket.next()).await.map_err(|_| Failure::ReadyTimeout)?
                .ok_or(Failure::NetworkFailed)?.map_err(websocket_failure)?;
            let Some(text) = text_frame(message)? else { continue; };
            let event = Audio3ASRServerEventDecoder::decode(&text).map_err(|_| Failure::ProtocolInvalid)?;
            if let Some(failure) = task_failure(&event) { return Err(failure); }
            if matches!(event, Audio3ASRServerEvent::TaskStarted) { break; }
            if !matches!(event, Audio3ASRServerEvent::Heartbeat | Audio3ASRServerEvent::Ignored {..}) {
                return Err(Failure::ProtocolInvalid);
            }
        }
        observer.metrics.task_started = 1;
        observer.metrics.ready_ms = connecting.elapsed().as_millis() as u64;
        let (mut sink, mut stream) = socket.split();
        let started = Instant::now();
        let overall_deadline = started + Duration::from_millis(clip.pcm.len() as u64 / 32 + 2000 + 30_000);
        let (finish_tx, mut finish_rx) = watch::channel(None::<Instant>);
        let sender = async {
            let zeros = [0u8; FRAME_BYTES];
            let mut schedule = started;
            for (is_tail, bytes) in clip.pcm.chunks(FRAME_BYTES).map(|bytes| (false, bytes))
                .chain((0..TAIL_FRAMES).map(|_| (true, zeros.as_slice())))
            {
                tokio::time::sleep_until(schedule).await;
                sent.maximum_send_lateness_ms = sent.maximum_send_lateness_ms.max(Instant::now().saturating_duration_since(schedule).as_millis() as u64);
                tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Binary(bytes.to_vec().into())))
                    .await.map_err(|_| Failure::SendTimeout)?.map_err(websocket_failure)?;
                sent.audio_frames += u64::from(!is_tail);
                sent.tail_frames += u64::from(is_tail);
                sent.sent_bytes += bytes.len() as u64;
                schedule += Duration::from_micros(bytes.len() as u64 * 1_000_000 / BYTES_PER_SECOND as u64);
            }
            tokio::time::sleep_until(schedule).await;
            let finish = Audio3ASRRequestEncoder::finish_task(&task_id).map_err(|_| Failure::ProtocolInvalid)?;
            // Publish the local finish clock before writing: a fast real ack
            // must never look like a task-finished that preceded our finish.
            finish_tx.send_replace(Some(Instant::now()));
            tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Text(finish.to_string().into())))
                .await.map_err(|_| Failure::SendTimeout)?.map_err(websocket_failure)?;
            Ok::<_, Failure>(())
        };
        let receiver = async {
            loop {
                let finish_at = *finish_rx.borrow();
                let deadline = finish_at.map_or(overall_deadline, |at| at + FINISH_TIMEOUT).min(overall_deadline);
                let message = tokio::select! {
                    biased;
                    changed = finish_rx.changed(), if finish_at.is_none() => {
                        changed.map_err(|_| Failure::NetworkFailed)?;
                        continue;
                    }
                    message = tokio::time::timeout_at(deadline, stream.next()) => message
                        .map_err(|_| Failure::FinishTimeout)?.ok_or(Failure::NetworkFailed)?.map_err(websocket_failure)?,
                };
                let Some(text) = text_frame(message)? else { continue; };
                let event = observer.record(text.as_str(), started.elapsed().as_millis() as u64, finish_rx.borrow().is_some())?;
                if let Some(failure) = task_failure(&event) { return Err(failure); }
                if matches!(event, Audio3ASRServerEvent::TaskFinished) {
                    if finish_rx.borrow().is_none() { return Err(Failure::ProtocolInvalid); }
                    observer.metrics.task_finished = 1;
                    observer.metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                    return Ok::<_, Failure>(());
                }
            }
        };
        tokio::time::timeout_at(overall_deadline, async { tokio::try_join!(sender, receiver) })
            .await.map_err(|_| Failure::FinishTimeout)??;
        Ok::<_, Failure>(())
    }.await;
    observer.metrics.audio_frames = sent.audio_frames;
    observer.metrics.tail_frames = sent.tail_frames;
    observer.metrics.sent_bytes = sent.sent_bytes;
    observer.metrics.maximum_send_lateness_ms = sent.maximum_send_lateness_ms;
    observer.report(clip, arm, operation.err())
}

/// This entry exists only in the explicitly enabled macOS test executable. It
/// never falls back to Keychain and never accepts credentials from environment.
#[cfg(all(feature = "local-dev-credentials", target_os = "macos"))]
#[tokio::test]
#[ignore = "manual paid provider benchmark; requires explicit private development credentials and public fixture manifest"]
async fn manual_same_pcm_asr_comparison() {
    use crate::core::protocols::audio3::DASHSCOPE_INFERENCE_WS;
    use crate::core::provider::{ProviderKind, TextTranslation};
    use crate::settings_store::{SettingsStore, DEVELOPMENT_APPLICATION_IDENTIFIER};

    let run = async {
        let path = std::env::var_os("MIMI_ASR_BENCH_MANIFEST")
            .map(PathBuf::from)
            .ok_or(Failure::ManifestInvalid)?;
        let mut clips = prepare_manifest(&path)?;
        let processing = match std::env::var("MIMI_ASR_BENCH_DENOISE") {
            Ok(value) => denoising::Kind::parse(&value)?,
            Err(std::env::VarError::NotPresent) => denoising::Kind::Bypass,
            Err(_) => return Err(Failure::ManifestInvalid),
        };
        for clip in &mut clips {
            if processing != denoising::Kind::Bypass && clip.preparation != FixturePreparation::Raw
            {
                return Err(Failure::ManifestInvalid);
            }
            clip.pcm = denoising::apply(&clip.pcm, processing)?;
        }
        let arms = match std::env::var("MIMI_ASR_BENCH_ARM") {
            Ok(value) => vec![Arm::parse(&value)?],
            Err(std::env::VarError::NotPresent) => {
                vec![Arm::Baseline, Arm::NoContext, Arm::Model31]
            }
            Err(_) => return Err(Failure::ManifestInvalid),
        };
        let directory = match std::env::var_os("MIMI_ASR_BENCH_CONFIG_DIR") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(std::env::var_os("HOME").ok_or(Failure::CredentialsUnavailable)?)
                .join("Library/Application Support")
                .join(DEVELOPMENT_APPLICATION_IDENTIFIER),
        };
        if !directory.is_absolute() {
            return Err(Failure::CredentialsUnavailable);
        }
        let settings = SettingsStore::load(directory, false, DEVELOPMENT_APPLICATION_IDENTIFIER);
        // `load` may construct the ordinary store for an absent file, but this
        // check runs before any credential read. No Keychain lookup is allowed.
        if settings.credential_storage() != "localDevFile" {
            return Err(Failure::CredentialsUnavailable);
        }
        let (_, profiles) = settings
            .profile_catalog()
            .map_err(|_| Failure::CredentialsUnavailable)?;
        let profile = profiles
            .iter()
            .find(|profile| {
                profile.provider == ProviderKind::AlibabaCloud
                    && profile.text_translation() == TextTranslation::FollowService
            })
            .ok_or(Failure::CredentialsUnavailable)?;
        let configuration = settings
            .configuration_for_profile_probe(profile)
            .map_err(|_| Failure::CredentialsUnavailable)?;
        let key = configuration
            .credentials
            .alibaba_key()
            .filter(|key| !key.is_empty())
            .ok_or(Failure::CredentialsUnavailable)?;
        let network = ProviderNetwork::resolve(&configuration.network_proxy)
            .map_err(|_| Failure::NetworkFailed)?;
        let mut failed = false;
        let mut rejected = Vec::<(Arm, Failure)>::new();
        let mut authentication_failed = false;
        for clip in &clips {
            for arm in &arms {
                let blocked = if authentication_failed {
                    Some(Failure::AuthenticationRejected)
                } else {
                    rejected
                        .iter()
                        .find(|(blocked_arm, _)| blocked_arm == arm)
                        .map(|(_, failure)| *failure)
                };
                let mut report = if let Some(failure) = blocked {
                    let mut observer = Observer::new(clip);
                    observer.metrics.skipped = 1;
                    observer.report(clip, *arm, Some(failure))
                } else {
                    if matches!(arm, Arm::RealtimeAsr) {
                        realtime::compare(clip, key, &network, &realtime::endpoint()).await
                    } else {
                        compare(clip, *arm, key, &network, DASHSCOPE_INFERENCE_WS).await
                    }
                };
                report.input_processing = processing;
                if matches!(report.failure, Some(Failure::AuthenticationRejected)) {
                    authentication_failed = true;
                } else if matches!(report.failure, Some(Failure::TaskRejected))
                    && !rejected.iter().any(|(blocked_arm, _)| blocked_arm == arm)
                {
                    rejected.push((*arm, Failure::TaskRejected));
                }
                failed |= report.failure.is_some();
                println!(
                    "ASR_BENCH_JSON {}",
                    serde_json::to_string(&report).map_err(|_| Failure::ProtocolInvalid)?
                );
            }
        }
        if failed {
            Err(Failure::TaskRejected)
        } else {
            Ok(())
        }
    }
    .await;
    if let Err(failure) = run {
        println!("ASR_BENCH_JSON {}", serde_json::json!({"failure": failure}));
        panic!("ASR benchmark failed; see the fixed diagnostic classification above");
    }
}

#[test]
fn streaming_word_distance_crosses_final_boundaries_without_a_transcript() {
    let mut distance =
        EditDistance::new(normalize("a small synthetic reading example", Unit::Word));
    distance.push(&normalize("A small", Unit::Word)).unwrap();
    distance
        .push(&normalize("reading examples", Unit::Word))
        .unwrap();
    let result = distance.result();
    assert_eq!(result.edit_distance, 2); // Missing synthetic, examples != example.
    assert_eq!(result.reference_units, 5);
    assert_eq!(result.hypothesis_units, 4);
    assert_eq!(result.error_rate, 0.4);
}

#[test]
fn character_distance_handles_japanese_chinese_and_punctuation() {
    for (reference, hypothesis) in [
        ("こんにちは。", "こんにちは！"),
        ("安全的示例。", "安全的例。"),
    ] {
        let mut distance = EditDistance::new(normalize(reference, Unit::Char));
        distance.push(&normalize(hypothesis, Unit::Char)).unwrap();
        assert_eq!(
            distance.result().edit_distance,
            u64::from(reference.starts_with('安'))
        );
    }
    let mut distance = EditDistance::new(normalize("one two", Unit::Word));
    distance
        .push(&normalize("one two three", Unit::Word))
        .unwrap();
    assert_eq!(distance.result().edit_distance, 1);
    assert!(distance
        .push(&vec!["synthetic".into(); MAX_HYPOTHESIS_UNITS])
        .is_err());
}

#[test]
fn reference_edit_counts_distinguish_deletions_insertions_and_substitutions() {
    for (reference, hypothesis, expected) in [
        ("one two three", "one three", (0, 0, 1)),
        ("one three", "one two three", (0, 1, 0)),
        ("one two three", "one other three", (1, 0, 0)),
    ] {
        let mut distance = EditDistance::new(normalize(reference, Unit::Word));
        for unit in normalize(hypothesis, Unit::Word) {
            distance.push(&[unit]).unwrap();
        }
        let score = distance.result();
        assert_eq!(
            (score.substitutions, score.insertions, score.deletions),
            expected
        );
        assert_eq!(score.edit_distance, 1);
    }
}

fn synthetic_clip() -> PreparedClip {
    PreparedClip {
        label: "synthetic".into(),
        pcm: vec![0; FRAME_BYTES],
        reference: Some(normalize("private synthetic token", Unit::Word)),
        language: Language::English,
        unit: Unit::Word,
        preparation: FixturePreparation::Raw,
    }
}

fn caption(id: u64, text: &str, final_result: bool) -> String {
    serde_json::json!({"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":id,"sentence_begin":!final_result,"sentence_end":final_result,"text":text,"end_time":50}}}}).to_string()
}

#[test]
fn observer_ignores_same_id_replays_but_keeps_real_repeated_utterances() {
    let clip = synthetic_clip();
    let mut observer = Observer::new(&clip);
    observer
        .record(&caption(1, "private synthetic", false), 100, false)
        .unwrap();
    observer
        .record(&caption(1, "private synthet", false), 120, false)
        .unwrap();
    observer
        .record(&caption(1, "private synthetic token", true), 200, false)
        .unwrap();
    observer
        .record(&caption(1, "private synthetic token", true), 220, false)
        .unwrap();
    observer
        .record(&caption(2, "private synthetic token", true), 300, true)
        .unwrap();
    let report = observer.report(&clip, Arm::Baseline, None);
    assert_eq!(report.metrics.draft_revisions, 1);
    assert_eq!(report.metrics.draft_retracted_characters, 2);
    assert_eq!(report.metrics.duplicate_final_ids, 1);
    assert_eq!(report.evaluation.as_ref().unwrap().hypothesis_units, 6);
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains("private"));
    assert!(!output.contains("synthetic token"));
    assert!(!output.contains("pcm_path"));
    assert!(!output.contains("reference_path"));
}

#[test]
fn reports_distinguish_prepared_fixtures_from_benchmark_processing() {
    let mut clip = synthetic_clip();
    clip.preparation = FixturePreparation::Deepfilter12;
    let report = Observer::new(&clip).report(&clip, Arm::Baseline, None);
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["fixture_preparation"], "deepfilter12");
    assert_eq!(value["input_processing"], "bypass");
}

#[test]
fn error_bodies_and_unknown_fields_never_enter_report() {
    let clip = synthetic_clip();
    let mut observer = Observer::new(&clip);
    let event = observer.record(r#"{"header":{"event":"task-failed","error_code":"private credential","error_message":"private transcript and credential"}}"#, 50, false).unwrap();
    let report = observer.report(&clip, Arm::NoContext, task_failure(&event));
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains("private"));
    assert!(output.contains("task_rejected"));
    assert!(report.evaluation.is_none());
}

#[test]
fn source_script_counts_and_final_clocks_come_from_provider_text_not_the_hint() {
    let clip = synthetic_clip();
    let mut observer = Observer::new(&clip);
    observer
        .record(&caption(1, "Aé 中あアｱ。", false), 100, false)
        .unwrap();
    observer
        .record(&caption(1, "Aé 中あアｱ。", true), 200, true)
        .unwrap();
    let report = observer.report(&clip, Arm::Baseline, None);
    assert_eq!(report.metrics.draft_latin_characters, 2);
    assert_eq!(report.metrics.draft_han_characters, 1);
    assert_eq!(report.metrics.draft_kana_characters, 3);
    assert_eq!(report.metrics.final_han_characters, 1);
    assert_eq!(report.metrics.finals[0].received_ms, 200);
    assert_eq!(report.metrics.finals[0].end_time_ms, Some(50));
    assert_eq!(report.metrics.finals[0].begin_time_ms, None);
    assert_eq!(report.metrics.finals[0].character_classes.han, 1);
    assert!(matches!(
        report.metrics.finals[0].language_hint,
        Language::English
    ));
    let output = serde_json::to_string(&report).unwrap();
    assert!(!output.contains("Aé"));
    assert!(!output.contains('中'));
    assert!(!output.contains('あ'));
}

#[test]
fn control_frames_and_invalid_binary_return_only_fixed_categories() {
    assert!(text_frame(Message::Ping(vec![1, 2, 3].into()))
        .unwrap()
        .is_none());
    assert!(matches!(
        text_frame(Message::Binary(vec![0xff].into())),
        Err(Failure::ProtocolInvalid)
    ));
    assert_eq!(
        text_frame(Message::Binary(b"synthetic".to_vec().into()))
            .unwrap()
            .unwrap(),
        "synthetic"
    );
    assert!(matches!(
        text_frame(Message::Close(None)),
        Err(Failure::NetworkFailed)
    ));
}

#[test]
fn manifest_rejects_excess_clips_unknown_fields_paths_labels_and_languages() {
    let fixture = std::env::temp_dir().join("mimi-asr-benchmark-synthetic");
    let valid = serde_json::json!({"clips":[{"label":"speech-en","pcm_path":fixture.join("en.pcm"),"reference_path":fixture.join("en.txt"),"language":"English","unit":"word"}]});
    assert!(parse_manifest(&serde_json::to_vec(&valid).unwrap()).is_ok());
    for (field, value) in [
        ("language", "unknown"),
        ("unit", "unknown"),
        ("preparation", "arbitrary-private-label"),
        ("label", "private words with spaces"),
        ("pcm_path", "relative.pcm"),
        ("reference_path", "/private/tmp/corpus/.env"),
    ] {
        let mut invalid = valid.clone();
        invalid["clips"][0][field] = value.into();
        assert!(parse_manifest(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut invalid = valid.clone();
    invalid["clips"][0]["api_key"] = "synthetic-private".into();
    assert!(parse_manifest(&serde_json::to_vec(&invalid).unwrap()).is_err());
    invalid = valid.clone();
    invalid["clips"] = serde_json::Value::Array(vec![valid["clips"][0].clone(); 11]);
    assert!(parse_manifest(&serde_json::to_vec(&invalid).unwrap()).is_err());
    assert!(parse_manifest(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    assert!(Arm::parse("private unknown").is_err());
}

#[test]
fn input_limits_refuse_outside_files_links_and_oversized_pcm() {
    let directory = tempfile::tempdir().unwrap();
    let pcm = directory.path().join("clip.pcm");
    let reference = directory.path().join("clip.txt");
    let manifest = directory.path().join("manifest.json");
    std::fs::write(&pcm, [0, 0]).unwrap();
    std::fs::write(&reference, "Safe synthetic example.").unwrap();
    let write = |pcm: &Path| {
        std::fs::write(&manifest, serde_json::to_vec(&serde_json::json!({"clips":[{"label":"synthetic","pcm_path":pcm,"reference_path":reference,"language":"Chinese","unit":"char"}]})).unwrap()).unwrap()
    };
    write(&pcm);
    assert_eq!(prepare_manifest(&manifest).unwrap().len(), 1);
    let file = File::create(&pcm).unwrap();
    file.set_len((MAX_PCM_BYTES + 2) as u64).unwrap();
    assert!(prepare_manifest(&manifest).is_err());
    std::fs::write(&pcm, [0]).unwrap();
    assert!(prepare_manifest(&manifest).is_err());
    let outside = tempfile::tempdir().unwrap();
    let foreign = outside.path().join("other.pcm");
    std::fs::write(&foreign, [0, 0]).unwrap();
    write(&foreign);
    assert!(prepare_manifest(&manifest).is_err());
    #[cfg(unix)]
    {
        let link = directory.path().join("link.pcm");
        std::os::unix::fs::symlink(&foreign, &link).unwrap();
        write(&link);
        assert!(prepare_manifest(&manifest).is_err());
    }
}

#[test]
fn arms_change_only_allowlisted_model_and_context_fields() {
    let baseline = Arm::Baseline
        .request(SourceLanguage::English, "synthetic")
        .unwrap();
    let mut no_context = Arm::NoContext
        .request(SourceLanguage::English, "synthetic")
        .unwrap();
    let mut model31 = Arm::Model31
        .request(SourceLanguage::English, "synthetic")
        .unwrap();
    assert_eq!(
        model31["payload"]["model"],
        "qwen-audio-3.1-asr-flash-streaming"
    );
    assert_eq!(no_context["payload"]["input"], serde_json::json!({}));
    model31["payload"]["model"] = Audio3ASREndpoint::MODEL.into();
    assert_eq!(model31, no_context);
    no_context["payload"]["input"] = baseline["payload"]["input"].clone();
    assert_eq!(no_context, baseline);
}

#[tokio::test]
async fn local_socket_comparison_requires_ready_and_finished_without_extra_pcm() {
    use crate::core::network_proxy::{ProxyConfig, ProxyMode};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        assert!(matches!(
            ws.next().await.unwrap().unwrap(),
            Message::Text(_)
        ));
        assert!(tokio::time::timeout(Duration::from_millis(40), ws.next())
            .await
            .is_err());
        ws.send(Message::Text(
            r#"{"header":{"event":"task-started"}}"#.into(),
        ))
        .await
        .unwrap();
        let mut frames = 0;
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    assert_eq!(bytes.len(), FRAME_BYTES);
                    frames += 1;
                }
                Message::Text(_) => break,
                other => panic!("unexpected synthetic frame kind: {}", other.is_close()),
            }
        }
        assert_eq!(frames, 1 + TAIL_FRAMES);
        ws.send(Message::Text(
            caption(1, "private synthetic token", true).into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Text(
            r#"{"header":{"event":"task-finished"},"payload":{"usage":{"duration":3}}}"#.into(),
        ))
        .await
        .unwrap();
    });
    let direct = ProviderNetwork::resolve(&ProxyConfig {
        mode: ProxyMode::Direct,
        url: None,
    })
    .unwrap();
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        compare(
            &synthetic_clip(),
            Arm::NoContext,
            "synthetic-key",
            &direct,
            &endpoint,
        ),
    )
    .await
    .unwrap();
    assert!(report.failure.is_none());
    assert_eq!(report.metrics.task_started, 1);
    assert_eq!(report.metrics.task_finished, 1);
    assert_eq!(report.metrics.audio_frames, 1);
    assert_eq!(report.metrics.tail_frames, TAIL_FRAMES as u64);
    assert_eq!(report.metrics.final_after_finish, 1);
    assert_eq!(report.evaluation.unwrap().edit_distance, 0);
    server.await.unwrap();
}
