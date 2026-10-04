//! Desktop event adapter for the shared append-only transcript aligner.
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use mimi_core::openai_transcript_committer::{
    OpenAITranscriptPairCommitter as SharedCommitter, TranscriptEvent,
};

pub struct OpenAITranscriptPairCommitter(SharedCommitter);
impl OpenAITranscriptPairCommitter {
    pub fn new_gemini() -> Self {
        Self(SharedCommitter::new_gemini())
    }
    pub fn settle(&mut self, now_ms: u64) -> Vec<LiveTranslateServerEvent> {
        self.0.settle(now_ms).into_iter().map(adapt).collect()
    }
    pub fn new(limit: usize, language: Option<String>) -> Self {
        Self(SharedCommitter::new(limit, language))
    }
    pub fn append_source_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<LiveTranslateServerEvent> {
        self.0
            .append_source_delta(delta, elapsed_ms)
            .into_iter()
            .map(adapt)
            .collect()
    }
    pub fn append_translation_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<LiveTranslateServerEvent> {
        self.0
            .append_translation_delta(delta, elapsed_ms)
            .into_iter()
            .map(adapt)
            .collect()
    }
    pub fn finish(&mut self) -> Vec<LiveTranslateServerEvent> {
        self.0.finish().into_iter().map(adapt).collect()
    }
    pub fn has_pending(&self) -> bool {
        self.0.has_pending()
    }
    pub fn reset(&mut self) {
        self.0.reset();
    }
}
impl Default for OpenAITranscriptPairCommitter {
    fn default() -> Self {
        Self::new(320, None)
    }
}
fn adapt(event: TranscriptEvent) -> LiveTranslateServerEvent {
    match event {
        TranscriptEvent::SourceDraft { text, language } => {
            LiveTranslateServerEvent::SourceDraft { text, language }
        }
        TranscriptEvent::TranslationDraft(text) => LiveTranslateServerEvent::TranslationDraft(text),
        TranscriptEvent::SubtitleFinalPair {
            source,
            language,
            translation,
        } => LiveTranslateServerEvent::SubtitleFinalPair {
            source,
            language,
            translation,
        },
        TranscriptEvent::Error { code, message } => {
            LiveTranslateServerEvent::Error { code, message }
        }
    }
}
