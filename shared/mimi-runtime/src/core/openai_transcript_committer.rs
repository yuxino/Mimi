//! Desktop event adapter for shared OpenAI alignment and Gemini caption assembly.
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use mimi_core::openai_transcript_committer::{
    OpenAITranscriptPairCommitter as SharedCommitter, TranscriptEvent,
};

pub struct OpenAITranscriptPairCommitter {
    stream: SharedCommitter,
    progress: super::transcript_progress::SourceTranscriptProgress,
    clock: std::time::Instant,
    isolate_stale_source: bool,
}
impl OpenAITranscriptPairCommitter {
    fn with_stream(stream: SharedCommitter) -> Self {
        Self {
            stream,
            progress: Default::default(),
            clock: std::time::Instant::now(),
            isolate_stale_source: true,
        }
    }
    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    #[cfg(feature = "test-support")]
    pub fn disable_source_isolation_for_comparison(&mut self) {
        self.isolate_stale_source = false;
    }
    pub fn new_gemini() -> Self {
        Self {
            isolate_stale_source: false,
            ..Self::with_stream(SharedCommitter::new_gemini())
        }
    }
    pub fn settle(&mut self, now_ms: u64) -> Vec<LiveTranslateServerEvent> {
        self.stream.settle(now_ms).into_iter().map(adapt).collect()
    }
    pub fn new(limit: usize, language: Option<String>) -> Self {
        Self::with_stream(SharedCommitter::new(limit, language))
    }
    pub fn append_source_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<LiveTranslateServerEvent> {
        let events = self.stream.append_source_delta(delta, elapsed_ms);
        if !delta.trim().is_empty()
            && events
                .iter()
                .any(|event| matches!(event, TranscriptEvent::SourceDraft { .. }))
        {
            self.progress.source();
        }
        events.into_iter().map(adapt).collect()
    }
    pub fn append_translation_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<LiveTranslateServerEvent> {
        let mut events = Vec::new();
        if self.isolate_stale_source
            && !self.progress.stale()
            && self
                .progress
                .translation(self.now_ms(), delta.trim().chars().count())
        {
            events.extend(self.stream.isolate_source());
        }
        events.extend(self.stream.append_translation_delta(delta, elapsed_ms));
        events.into_iter().map(adapt).collect()
    }

    pub fn finish(&mut self) -> Vec<LiveTranslateServerEvent> {
        self.stream.finish().into_iter().map(adapt).collect()
    }
    pub fn has_pending(&self) -> bool {
        self.stream.has_pending()
    }
    pub fn reset(&mut self) {
        self.stream.reset();
        self.progress = Default::default();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn translation_at(
        stream: &mut OpenAITranscriptPairCommitter,
        milliseconds: u64,
    ) -> Vec<LiveTranslateServerEvent> {
        stream.clock = std::time::Instant::now() - std::time::Duration::from_millis(milliseconds);
        stream.append_translation_delta("Synthetic translated fragment ", None)
    }

    #[test]
    fn source_omission_retires_stale_original_without_interrupting_translation() {
        let mut stream = OpenAITranscriptPairCommitter::default();
        stream.append_source_delta("Old unfinished original", None);
        for time in (0..20_000).step_by(2_000) {
            assert!(translation_at(&mut stream, time)
                .iter()
                .all(|event| !matches!(event, LiveTranslateServerEvent::Error { .. })));
        }
        let isolated = translation_at(&mut stream, 20_001);
        assert!(
            matches!(isolated.as_slice(), [LiveTranslateServerEvent::SourceDraft { text, .. }, LiveTranslateServerEvent::TranslationDraft(_)] if text.is_empty())
        );
        assert!(translation_at(&mut stream, 22_000)
            .iter()
            .all(|event| !matches!(
                event,
                LiveTranslateServerEvent::Error { .. }
                    | LiveTranslateServerEvent::SubtitleFinalPair { .. }
            )));
        let resumed = stream.append_source_delta("New original.", None);
        assert!(
            matches!(resumed.as_slice(), [LiveTranslateServerEvent::TranslationDraft(text), LiveTranslateServerEvent::SourceDraft { .. }] if text.is_empty())
        );
        assert!(
            matches!(stream.append_translation_delta("新原文。", None).last(), Some(LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }) if source == "New original." && translation == "新原文。")
        );
        stream.reset();
        stream.append_source_delta("新的完整原文。", None);
        assert!(
            matches!(stream.append_translation_delta("新的完整原文。", None).last(),
            Some(LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. })
                if source == "新的完整原文。" && translation == source)
        );
    }

    #[test]
    fn waiting_without_output_then_resuming_cannot_be_mistaken_for_source_stall() {
        let mut stream = OpenAITranscriptPairCommitter::default();
        stream.append_source_delta("First.", None);
        stream.append_translation_delta("第一句。", None);
        // Local waiting does not arm a timer; sparse delayed tails also restart
        // the window. Provider elapsed_ms cannot fast-forward the watchdog.
        assert!(translation_at(&mut stream, 600_000)
            .iter()
            .all(|event| !matches!(event, LiveTranslateServerEvent::Error { .. })));
        stream.append_source_delta(" Resumed", Some(u64::MAX));
        assert!(translation_at(&mut stream, 602_000)
            .iter()
            .all(|event| !matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[test]
    fn gemini_whole_block_stream_does_not_isolate_delayed_originals() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        for time in (0..30_000).step_by(2_000) {
            assert!(translation_at(&mut stream, time)
                .iter()
                .all(|event| !matches!(event, LiveTranslateServerEvent::Error { .. })));
        }
        stream.append_source_delta("Delayed original.", None);
        assert!(stream
            .finish()
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
    }
}
