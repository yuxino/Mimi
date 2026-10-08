//! UI-independent subtitle and translation behavior shared by every Mimi platform.
//! Platform capture, network transport, persistence and presentation stay outside.

pub mod bridge;
pub mod live_pair_aligner;
pub mod models;
pub mod openai_transcript_committer;
pub mod subtitle_reducer;
pub mod translation_policy;

pub use models::{
    subtitle_text_within_limit, AudioSource, PreviewSubtitlePair, RealtimeSubtitlePreview,
    SourceSubtitleSnapshot, SubtitleEvent, SubtitleLine, SubtitlePair, SubtitleSnapshot,
    TranslationRecovery, TranslationRecoveryReason, UtteranceRole, MAX_SUBTITLE_TEXT_BYTES,
};
pub use subtitle_reducer::{ArchiveSink, NoopArchive, SubtitleReducer};
