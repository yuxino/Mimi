//! Bounded, content-free development evidence. No Tauri or provider payloads.

use super::audio_input::AudioSource;
use super::models::{SubtitleSnapshot, UtteranceRole};
use super::protocols::live_translate::LiveTranslateServerEvent;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const ENTRY_LIMIT: usize = 2_048;
pub const VIEW_LIMIT: usize = 240;
pub const BATCH_LIMIT: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSummary {
    pub source_characters: usize,
    pub translation_characters: usize,
    pub history_entries: usize,
    pub history_source_characters: usize,
    pub history_translation_characters: usize,
    pub preview_source_characters: usize,
    pub preview_translation_characters: usize,
    #[serde(default)]
    pub tracks: Vec<TrackSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrackSummary {
    pub source: AudioSource,
    pub source_characters: usize,
    pub translation_characters: usize,
    pub history_entries: usize,
    pub preview_source_characters: usize,
    pub preview_translation_characters: usize,
}

impl From<&SubtitleSnapshot> for SubtitleSummary {
    fn from(value: &SubtitleSnapshot) -> Self {
        Self {
            source_characters: if value.tracks.is_empty() {
                value.source.text.chars().count()
            } else {
                value
                    .tracks
                    .iter()
                    .map(|t| t.source.text.chars().count())
                    .sum()
            },
            translation_characters: if value.tracks.is_empty() {
                value.translation.text.chars().count()
            } else {
                value
                    .tracks
                    .iter()
                    .map(|t| t.translation.text.chars().count())
                    .sum()
            },
            history_entries: value.history.len(),
            history_source_characters: value.history.iter().map(|p| p.source.chars().count()).sum(),
            history_translation_characters: value
                .history
                .iter()
                .map(|p| p.translation.chars().count())
                .sum(),
            tracks: value
                .tracks
                .iter()
                .map(|track| TrackSummary {
                    source: track.audio_source,
                    source_characters: track.source.text.chars().count(),
                    translation_characters: track.translation.text.chars().count(),
                    history_entries: track.history.len(),
                    preview_source_characters: track
                        .preview_pair
                        .as_ref()
                        .map_or(0, |p| p.source.chars().count()),
                    preview_translation_characters: track
                        .preview_pair
                        .as_ref()
                        .map_or(0, |p| p.translation.chars().count()),
                })
                .collect(),
            preview_source_characters: value
                .tracks
                .iter()
                .filter_map(|t| t.preview_pair.as_ref())
                .map(|p| p.source.chars().count())
                .sum(),
            preview_translation_characters: value
                .tracks
                .iter()
                .filter_map(|t| t.preview_pair.as_ref())
                .map(|p| p.translation.chars().count())
                .sum(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Admission {
    QueueDraftAttempt,
    QueueReliableAttempt,
    ProducerClosed,
    ProducerBackpressureResult,
    ObsoleteProducer,
    ProducerBackpressure,
    OversizedText,
    Accepted,
    StaleContent,
    StaleGeneration,
    Paused,
    SetupAcknowledgement,
    DuplicateFinal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderEventKind {
    SessionCreated,
    SessionUpdated,
    SourceDraft,
    SourceFinal,
    TranslationStarted,
    PreviewStarted,
    PreviewFinished,
    PreviewPair,
    PreviewCleared,
    TranslationDeferred,
    TranslationDraft,
    TranslationFinal,
    FinalPair,
    ConfirmedPair,
    SessionFinished,
    Error,
    Ignored,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DebugProducer {
    #[default]
    Session,
    Provider,
    Recognition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderObservation {
    #[serde(default)]
    pub producer: DebugProducer,
    pub transport_sequence: Option<u64>,
    pub source: AudioSource,
    pub generation: u64,
    pub content_revision: u64,
    pub event_kind: ProviderEventKind,
    pub utterance_id: Option<u64>,
    pub source_characters: usize,
    pub translation_characters: usize,
    pub admission: Admission,
}

impl ProviderObservation {
    pub fn new(
        source: AudioSource,
        generation: u64,
        content_revision: u64,
        event: &LiveTranslateServerEvent,
        admission: Admission,
    ) -> Self {
        use LiveTranslateServerEvent as E;
        use ProviderEventKind as K;
        let (event_kind, utterance_id, source_characters, translation_characters) = match event {
            E::SessionCreated => (K::SessionCreated, None, 0, 0),
            E::SessionUpdated => (K::SessionUpdated, None, 0, 0),
            E::SourceDraft { text, .. } => (K::SourceDraft, None, text.chars().count(), 0),
            E::SourceFinal { text, .. } => (K::SourceFinal, None, text.chars().count(), 0),
            E::SourceUtteranceDraft {
                utterance_id, text, ..
            } => (K::SourceDraft, Some(*utterance_id), text.chars().count(), 0),
            E::SourceUtteranceFinal {
                utterance_id, text, ..
            } => (K::SourceFinal, Some(*utterance_id), text.chars().count(), 0),
            E::UtteranceText {
                role,
                text,
                is_final,
                ..
            } => match (role, is_final) {
                (UtteranceRole::Source, false) => (K::SourceDraft, None, text.chars().count(), 0),
                (UtteranceRole::Source, true) => (K::SourceFinal, None, text.chars().count(), 0),
                (UtteranceRole::Translation, false) => {
                    (K::TranslationDraft, None, 0, text.chars().count())
                }
                (UtteranceRole::Translation, true) => {
                    (K::TranslationFinal, None, 0, text.chars().count())
                }
            },
            E::TranslationStarted => (K::TranslationStarted, None, 0, 0),
            E::PreviewTranslationStarted { request_id } => {
                (K::PreviewStarted, Some(*request_id), 0, 0)
            }
            E::PreviewTranslationFinished { request_id } => {
                (K::PreviewFinished, Some(*request_id), 0, 0)
            }
            E::SubtitlePreviewPair {
                source_utterance_id,
                source,
                translation,
                ..
            } => (
                K::PreviewPair,
                *source_utterance_id,
                source.chars().count(),
                translation.chars().count(),
            ),
            E::SubtitlePreviewCleared => (K::PreviewCleared, None, 0, 0),
            E::TranslationDeferred(_) => (K::TranslationDeferred, None, 0, 0),
            E::TranslationDraft(text) => (K::TranslationDraft, None, 0, text.chars().count()),
            E::TranslationFinal(text) => (K::TranslationFinal, None, 0, text.chars().count()),
            E::SubtitleFinalPair {
                source,
                translation,
                ..
            } => (
                K::FinalPair,
                None,
                source.chars().count(),
                translation.chars().count(),
            ),
            E::SubtitleConfirmedPair {
                utterance_id,
                source,
                translation,
                ..
            } => (
                K::ConfirmedPair,
                Some(*utterance_id),
                source.chars().count(),
                translation.chars().count(),
            ),
            E::SessionFinished => (K::SessionFinished, None, 0, 0),
            E::Error { .. } => (K::Error, None, 0, 0),
            E::Ignored { .. } => (K::Ignored, None, 0, 0),
        };
        Self {
            producer: DebugProducer::Session,
            transport_sequence: None,
            source,
            generation,
            content_revision,
            event_kind,
            utterance_id,
            source_characters,
            translation_characters,
            admission,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendStage {
    WireReceived,
    StoreApplied,
    OverlayCommitted,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DebugWindow {
    Settings,
    Overlay,
    TrayPanel,
    OverlayControl,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Projection {
    Empty,
    Original,
    Translation,
    Dual,
    Collapsed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendObservation {
    pub stage: FrontendStage,
    pub window: DebugWindow,
    pub snapshot_id: u64,
    pub source_characters: u32,
    pub translation_characters: u32,
    pub history_entries: u32,
    pub audio_tracks: Option<u32>,
    pub visible_characters: Option<u32>,
    pub visible_blocks: Option<u32>,
    pub overflowed_blocks: Option<u32>,
    pub scroll_top: Option<f64>,
    pub scroll_height: Option<f64>,
    pub viewport_height: Option<f64>,
    pub viewport_width: Option<f64>,
    pub projection: Option<Projection>,
    pub selected_source_characters: Option<u32>,
    pub selected_translation_characters: Option<u32>,
    pub stable_source_characters: Option<u32>,
    pub stable_translation_characters: Option<u32>,
    pub client_dropped: Option<u32>,
    pub projection_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DebugEvent {
    Stopped {
        unflushed_windows: Vec<DebugWindow>,
    },
    Pipeline {
        label: String,
    },
    Provider {
        observation: ProviderObservation,
    },
    Reduced {
        source: AudioSource,
        generation: u64,
        changed: bool,
        before: SubtitleSummary,
        after: SubtitleSummary,
    },
    Snapshot {
        snapshot_id: u64,
        generation: u64,
        summary: SubtitleSummary,
    },
    Published {
        snapshot_id: u64,
        delivered: bool,
        overlay_requested: bool,
        collapsed: bool,
    },
    Frontend {
        observation: FrontendObservation,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugEntry {
    pub id: u64,
    pub elapsed_ms: u64,
    pub event: DebugEvent,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugSnapshot {
    pub schema_version: u8,
    pub available: bool,
    pub enabled: bool,
    pub entry_limit: usize,
    pub recorded: u64,
    pub evicted: u64,
    pub frontend_dropped: u64,
    #[serde(default)]
    pub stale_frontend_rejected: u64,
    pub entries: Vec<DebugEntry>,
}

/// A fixed case target. Sequence assignment acquires a dispatch ticket under
/// the journal lock; callbacks run after releasing that lock. Stop detaches
/// the sink and waits for all already assigned tickets before sealing files.
#[derive(Clone)]
pub struct TraceSink(Arc<TraceSinkInner>);
struct TraceSinkInner {
    callback: Box<dyn Fn(DebugEntry) + Send + Sync>,
    pending: AtomicUsize,
    idle_gate: Mutex<()>,
    idle: Condvar,
}
impl TraceSink {
    pub fn new(callback: impl Fn(DebugEntry) + Send + Sync + 'static) -> Self {
        Self(Arc::new(TraceSinkInner {
            callback: Box::new(callback),
            pending: AtomicUsize::new(0),
            idle_gate: Mutex::new(()),
            idle: Condvar::new(),
        }))
    }
    /// Call only after detaching this sink, and outside the journal lock.
    /// A timeout leaves the caller's case open so finalization can be retried.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut gate = self.0.idle_gate.lock().unwrap();
        loop {
            if self.0.pending.load(Ordering::Acquire) == 0 {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            // Atomic completion and notify need no blocking lock in the
            // callback. A short bounded recheck also closes missed wakes.
            gate = self
                .0
                .idle
                .wait_timeout(gate, remaining.min(Duration::from_millis(10)))
                .unwrap()
                .0;
        }
    }
    fn ticket(&self, entry: DebugEntry) -> TraceDelivery {
        self.0.pending.fetch_add(1, Ordering::AcqRel);
        TraceDelivery {
            sink: self.clone(),
            entry: Some(entry),
        }
    }
    #[cfg(test)]
    pub(crate) fn dispatch_for_test(&self, entry: DebugEntry) {
        self.ticket(entry).send();
    }
}
struct TraceDelivery {
    sink: TraceSink,
    entry: Option<DebugEntry>,
}
impl TraceDelivery {
    fn send(mut self) {
        if let Some(entry) = self.entry.take() {
            (self.sink.0.callback)(entry);
        }
    }
}
impl Drop for TraceDelivery {
    fn drop(&mut self) {
        if self.sink.0.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.sink.0.idle.notify_all();
        }
    }
}
fn dispatch(delivery: Option<TraceDelivery>) {
    if let Some(delivery) = delivery {
        delivery.send();
    }
}

pub struct DebugJournal {
    available: bool,
    enabled: bool,
    epoch: Instant,
    sequence: u64,
    snapshot_sequence: u64,
    evicted: u64,
    frontend_dropped: u64,
    first_snapshot_id: u64,
    stale_frontend_rejected: u64,
    entries: VecDeque<DebugEntry>,
    observed_windows: Vec<DebugWindow>,
    sink: Option<TraceSink>,
}
impl Default for DebugJournal {
    fn default() -> Self {
        Self {
            available: false,
            enabled: false,
            epoch: Instant::now(),
            sequence: 0,
            snapshot_sequence: 0,
            evicted: 0,
            frontend_dropped: 0,
            first_snapshot_id: 1,
            stale_frontend_rejected: 0,
            entries: VecDeque::new(),
            observed_windows: Vec::new(),
            sink: None,
        }
    }
}
impl DebugJournal {
    pub fn initialize(&mut self, available: bool) {
        self.available = available;
        self.enabled = false;
        self.sink = None;
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), &'static str> {
        if !self.available {
            return Err("development_debug_unavailable");
        }
        if enabled && !self.enabled {
            self.clear();
        }
        self.enabled = enabled;
        if !enabled {
            self.sink = None;
        }
        Ok(())
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.observed_windows.clear();
        self.sequence = 0;
        self.evicted = 0;
        self.frontend_dropped = 0;
        self.first_snapshot_id = self.snapshot_sequence.saturating_add(1);
        self.stale_frontend_rejected = 0;
        self.epoch = Instant::now();
        // Snapshot identities never reset; in-flight observations cannot alias.
    }
    fn record(&mut self, event: DebugEvent) -> Option<TraceDelivery> {
        if !self.enabled {
            return None;
        }
        if let DebugEvent::Frontend { observation } = &event {
            if observation.snapshot_id < self.first_snapshot_id
                || observation.snapshot_id > self.snapshot_sequence
            {
                self.stale_frontend_rejected += 1;
                return None;
            }
            if !self.observed_windows.contains(&observation.window) {
                self.observed_windows.push(observation.window);
            }
            self.frontend_dropped += u64::from(observation.client_dropped.unwrap_or_default());
        }
        self.sequence += 1;
        if self.entries.len() == ENTRY_LIMIT {
            self.entries.pop_front();
            self.evicted += 1;
        }
        let entry = DebugEntry {
            id: self.sequence,
            elapsed_ms: self.epoch.elapsed().as_millis() as u64,
            event,
        };
        let delivery = self.sink.as_ref().map(|sink| sink.ticket(entry.clone()));
        self.entries.push_back(entry);
        delivery
    }
    pub fn snapshot(&self, limit: usize) -> DebugSnapshot {
        DebugSnapshot {
            schema_version: 1,
            available: self.available,
            enabled: self.enabled,
            entry_limit: ENTRY_LIMIT,
            recorded: self.sequence,
            evicted: self.evicted,
            frontend_dropped: self.frontend_dropped,
            stale_frontend_rejected: self.stale_frontend_rejected,
            entries: self
                .entries
                .iter()
                .skip(self.entries.len().saturating_sub(limit))
                .cloned()
                .collect(),
        }
    }
}
fn hub() -> &'static Mutex<DebugJournal> {
    static HUB: OnceLock<Mutex<DebugJournal>> = OnceLock::new();
    HUB.get_or_init(Default::default)
}
pub fn initialize(available: bool) {
    hub().lock().unwrap().initialize(available);
}
pub fn is_enabled() -> bool {
    hub().lock().unwrap().enabled
}
pub fn set_enabled(enabled: bool) -> Result<(), &'static str> {
    hub().lock().unwrap().set_enabled(enabled)
}
pub fn set_enabled_at(enabled: bool, epoch: Instant) -> Result<(), &'static str> {
    let mut journal = hub().lock().unwrap();
    journal.set_enabled(enabled)?;
    if enabled {
        journal.epoch = epoch;
    }
    Ok(())
}

pub fn set_sink(sink: Option<TraceSink>) -> Result<(), &'static str> {
    let mut journal = hub().lock().unwrap();
    if sink.is_some() && !journal.available {
        return Err("development_debug_unavailable");
    }
    journal.sink = sink;
    Ok(())
}

pub fn record(event: DebugEvent) {
    let delivery = hub().lock().unwrap().record(event);
    dispatch(delivery);
}
pub fn snapshot(limit: usize) -> DebugSnapshot {
    hub().lock().unwrap().snapshot(limit)
}
pub fn observed_windows() -> Vec<DebugWindow> {
    hub().lock().unwrap().observed_windows.clone()
}
pub fn record_pipeline(arguments: std::fmt::Arguments<'_>) {
    let delivery = {
        let mut journal = hub().lock().unwrap();
        if journal.enabled {
            journal.record(DebugEvent::Pipeline {
                label: arguments.to_string().chars().take(512).collect(),
            })
        } else {
            None
        }
    };
    dispatch(delivery);
}
pub fn record_snapshot(generation: u64, subtitles: &SubtitleSnapshot) -> Option<u64> {
    let (snapshot_id, delivery) = {
        let mut journal = hub().lock().unwrap();
        if !journal.enabled {
            return None;
        }
        journal.snapshot_sequence += 1;
        let snapshot_id = journal.snapshot_sequence;
        (
            snapshot_id,
            journal.record(DebugEvent::Snapshot {
                snapshot_id,
                generation,
                summary: subtitles.into(),
            }),
        )
    };
    dispatch(delivery);
    Some(snapshot_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pipeline() -> DebugEvent {
        DebugEvent::Pipeline {
            label: "capture started".into(),
        }
    }
    #[test]
    fn private_sink_is_default_off_and_preserves_every_id_past_ring_capacity() {
        let stored = Arc::new(Mutex::new(Vec::new()));
        let target = stored.clone();
        let sink = TraceSink::new(move |entry| target.lock().unwrap().push(entry.id));
        let mut journal = DebugJournal::default();
        journal.initialize(true);
        journal.sink = Some(sink.clone());
        dispatch(journal.record(pipeline()));
        assert!(stored.lock().unwrap().is_empty());
        journal.set_enabled(true).unwrap();
        for _ in 0..ENTRY_LIMIT + 3 {
            dispatch(journal.record(pipeline()));
        }
        journal.set_enabled(false).unwrap();
        dispatch(journal.record(pipeline()));
        assert!(sink.wait_idle(Duration::ZERO));
        assert_eq!(journal.snapshot(ENTRY_LIMIT).evicted, 3);
        assert_eq!(
            *stored.lock().unwrap(),
            (1..=(ENTRY_LIMIT + 3) as u64).collect::<Vec<_>>()
        );
    }
    #[test]
    fn sink_tickets_keep_the_original_case_and_timeout_then_drain_after_stop() {
        let a = Arc::new(Mutex::new(Vec::new()));
        let a_target = a.clone();
        let sink_a = TraceSink::new(move |entry| a_target.lock().unwrap().push(entry.id));
        let b = Arc::new(Mutex::new(Vec::new()));
        let b_target = b.clone();
        let sink_b = TraceSink::new(move |entry| b_target.lock().unwrap().push(entry.id));
        let mut journal = DebugJournal::default();
        journal.initialize(true);
        journal.set_enabled(true).unwrap();
        journal.sink = Some(sink_a.clone());
        let allocated = journal.record(pipeline());
        journal.set_enabled(false).unwrap();
        assert!(!sink_a.wait_idle(Duration::from_millis(1)));
        journal.set_enabled(true).unwrap();
        journal.sink = Some(sink_b.clone());
        dispatch(allocated);
        assert!(sink_a.wait_idle(Duration::from_millis(1)));
        dispatch(journal.record(pipeline()));
        assert_eq!(*a.lock().unwrap(), vec![1]);
        assert_eq!(*b.lock().unwrap(), vec![1]);
    }
    #[test]
    fn a_bounded_nonblocking_sink_counts_its_own_loss_separately_from_ring_eviction() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicUsize::new(0));
        let dropped_target = dropped.clone();
        let target = TraceSink::new(move |entry| {
            if tx.try_send(entry).is_err() {
                dropped_target.fetch_add(1, Ordering::Relaxed);
            }
        });
        let mut journal = DebugJournal::default();
        journal.initialize(true);
        journal.set_enabled(true).unwrap();
        journal.sink = Some(target.clone());
        for _ in 0..3 {
            dispatch(journal.record(pipeline()));
        }
        journal.set_enabled(false).unwrap();
        assert!(target.wait_idle(Duration::ZERO));
        assert_eq!(rx.try_recv().unwrap().id, 1);
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        assert_eq!(journal.snapshot(ENTRY_LIMIT).evicted, 0);
        assert_eq!(journal.snapshot(ENTRY_LIMIT).recorded, 3);
    }
    #[test]
    fn production_cannot_enable_and_disabled_journal_retains_nothing() {
        let mut journal = DebugJournal::default();
        assert!(journal.set_enabled(true).is_err());
        let _ = journal.record(DebugEvent::Pipeline {
            label: "capture started".into(),
        });
        assert!(journal.snapshot(ENTRY_LIMIT).entries.is_empty());
    }
    #[test]
    fn ring_reports_loss_and_stop_preserves_evidence() {
        let mut journal = DebugJournal::default();
        journal.initialize(true);
        journal.set_enabled(true).unwrap();
        for _ in 0..ENTRY_LIMIT + 3 {
            let _ = journal.record(DebugEvent::Pipeline {
                label: "capture started".into(),
            });
        }
        journal.set_enabled(false).unwrap();
        let report = journal.snapshot(ENTRY_LIMIT);
        assert_eq!(report.entries.len(), ENTRY_LIMIT);
        assert_eq!(report.evicted, 3);
        assert_eq!(report.entries[0].id, 4);
        assert!(!report.enabled);
        journal.set_enabled(true).unwrap();
        assert!(journal.snapshot(ENTRY_LIMIT).entries.is_empty());
    }
    #[test]
    fn provider_observations_strip_content_errors_and_arbitrary_identifiers() {
        let marker = "private-secret-text";
        for event in [
            LiveTranslateServerEvent::Error {
                code: marker.into(),
                message: marker.into(),
            },
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: marker.into(),
                role: UtteranceRole::Source,
                text: marker.into(),
                is_final: true,
                language: Some(marker.into()),
            },
        ] {
            let observation =
                ProviderObservation::new(AudioSource::System, 3, 2, &event, Admission::Accepted);
            let json = serde_json::to_string(&observation).unwrap();
            assert!(!json.contains(marker));
        }
    }
    #[test]
    fn frontend_cannot_inject_text_or_arbitrary_labels() {
        let value = serde_json::json!({"stage":"wireReceived","window":"overlay","snapshotId":1,"sourceCharacters":2,"translationCharacters":3,"historyEntries":0,"text":"private"});
        assert!(serde_json::from_value::<FrontendObservation>(value).is_err());
    }
    #[test]
    fn a_new_trace_rejects_old_or_not_yet_published_snapshot_observations() {
        let mut journal = DebugJournal::default();
        journal.initialize(true);
        journal.snapshot_sequence = 8;
        journal.set_enabled(true).unwrap();
        let observation = |id| {
            serde_json::from_value::<FrontendObservation>(serde_json::json!({"stage":"wireReceived","window":"overlay","snapshotId":id,"sourceCharacters":2,"translationCharacters":3,"historyEntries":0})).unwrap()
        };
        let _ = journal.record(DebugEvent::Frontend {
            observation: observation(8),
        });
        let _ = journal.record(DebugEvent::Frontend {
            observation: observation(9),
        });
        assert_eq!(journal.snapshot(ENTRY_LIMIT).stale_frontend_rejected, 2);
        assert_eq!(journal.snapshot(ENTRY_LIMIT).recorded, 0);
        journal.snapshot_sequence = 9;
        let _ = journal.record(DebugEvent::Frontend {
            observation: observation(9),
        });
        assert_eq!(journal.snapshot(ENTRY_LIMIT).recorded, 1);
    }
}
