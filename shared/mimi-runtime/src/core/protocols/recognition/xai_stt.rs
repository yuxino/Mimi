//! xAI's standalone streaming STT contract. Chunk finals are durable even
//! before a silence boundary; stitched utterance/session summaries only add
//! their uncommitted suffix. No transcript history is retained here.

use crate::core::models::{subtitle_text_within_limit, SourceLanguage};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use thiserror::Error;

pub const MODEL: &str = "grok-voice-transcribe-2.0";
pub const PCM_SAMPLE_RATE: u32 = 16_000;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_PCM_CHUNK_BYTES: usize = 128 * 1024;
/// Mimi codes for the languages explicitly documented by xAI, including its
/// accepted regional codes. Wire aliases are applied only at this boundary.
pub const LANGUAGE_CODES: &[&str] = &[
    "ar", "bs", "bg", "yue", "ca", "zh", "hr", "cs", "da", "nl", "en", "tl", "fi", "fr", "de",
    "el", "hi", "hu", "id", "it", "ja", "ko", "mk", "ms", "no", "fa", "pl", "pt", "ro", "ru", "sk",
    "es", "sv", "th", "tr", "uk", "ur", "vi", "es-MX", "es-ES", "pt-BR", "pt-PT", "ar-EG", "ar-AE",
    "ar-SA",
];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("xai_stt_protocol_invalid")]
pub struct ProtocolError;

pub fn endpoint(source: SourceLanguage) -> Result<url::Url, ProtocolError> {
    let mut endpoint = url::Url::parse("wss://api.x.ai/v1/stt").map_err(|_| ProtocolError)?;
    let mut query = endpoint.query_pairs_mut();
    query.append_pair("model", MODEL);
    query.append_pair("sample_rate", "16000");
    query.append_pair("encoding", "pcm");
    query.append_pair("interim_results", "true");
    if source != SourceLanguage::Automatic {
        let code = source.raw_value();
        if !LANGUAGE_CODES.contains(&code) {
            return Err(ProtocolError);
        }
        query.append_pair(
            "language",
            match code {
                "tl" => "fil",
                "no" => "nb",
                _ => code,
            },
        );
    }
    drop(query);
    Ok(endpoint)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    Created,
    Partial {
        text: String,
        is_final: bool,
        speech_final: bool,
        start: f64,
        end: f64,
    },
    Done {
        text: Option<String>,
    },
    Rejected,
    AuthenticationRejected,
    Ignored,
}

pub fn decode(message: &str) -> Result<ServerEvent, ProtocolError> {
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError);
    }
    let value: Value = serde_json::from_str(message).map_err(|_| ProtocolError)?;
    match value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ProtocolError)?
    {
        "transcript.created" => Ok(ServerEvent::Created),
        "transcript.partial" => {
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .ok_or(ProtocolError)?;
            let is_final = value
                .get("is_final")
                .and_then(Value::as_bool)
                .ok_or(ProtocolError)?;
            let speech_final = value
                .get("speech_final")
                .and_then(Value::as_bool)
                .ok_or(ProtocolError)?;
            let start = nonnegative(&value, "start")?;
            let end = start + nonnegative(&value, "duration")?;
            if !end.is_finite()
                || (speech_final && !is_final)
                || (!speech_final && !subtitle_text_within_limit(text))
                || value
                    .get("channel_index")
                    .is_some_and(|channel| channel.as_u64() != Some(0))
            {
                return Err(ProtocolError);
            }
            Ok(ServerEvent::Partial {
                text: text.to_owned(),
                is_final,
                speech_final,
                start,
                end,
            })
        }
        "transcript.done" => {
            nonnegative(&value, "duration")?;
            let text = value
                .get("text")
                .map(|text| text.as_str().map(str::to_owned).ok_or(ProtocolError))
                .transpose()?;
            Ok(ServerEvent::Done { text })
        }
        // Error messages may contain credentials or recognized text. Retain
        // only exact allowlisted codes; never surface the provider message.
        "error" => {
            if matches!(
                value.get("code").and_then(Value::as_str),
                Some("invalid_api_key" | "authentication_error" | "unauthorized")
            ) {
                Ok(ServerEvent::AuthenticationRejected)
            } else {
                Ok(ServerEvent::Rejected)
            }
        }
        _ => Ok(ServerEvent::Ignored),
    }
}

fn nonnegative(value: &Value, key: &str) -> Result<f64, ProtocolError> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)
        .ok_or(ProtocolError)
}

/// A content-free fingerprint and length of committed text. Whitespace is
/// ignored because xAI stitches chunks with separators. This makes both an
/// utterance summary and the final full-session summary safe to deduplicate
/// without keeping the session transcript in memory.
#[derive(Clone, Default)]
struct CommittedPrefix {
    characters: usize,
    digest: DefaultHasher,
}

impl CommittedPrefix {
    fn append(&mut self, text: &str) {
        for ch in text.chars().filter(|ch| !ch.is_whitespace()) {
            self.digest.write_u32(ch as u32);
            self.characters += 1;
        }
    }

    fn suffix<'a>(&self, text: &'a str) -> Result<&'a str, ProtocolError> {
        if self.characters == 0 {
            return Ok(text.trim());
        }
        let mut digest = DefaultHasher::new();
        let mut count = 0;
        for (offset, ch) in text.char_indices() {
            if ch.is_whitespace() {
                continue;
            }
            digest.write_u32(ch as u32);
            count += 1;
            if count == self.characters {
                if digest.finish() != self.digest.finish() {
                    return Err(ProtocolError);
                }
                return Ok(text[offset + ch.len_utf8()..].trim());
            }
        }
        Err(ProtocolError)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptOutput {
    pub sequence: u64,
    pub text: String,
    pub is_final: bool,
}

#[derive(Default)]
pub struct TranscriptAssembler {
    sequence: u64,
    draft_active: bool,
    utterance: CommittedPrefix,
    session: CommittedPrefix,
    committed_end: Option<f64>,
    utterance_end: Option<f64>,
}

impl TranscriptAssembler {
    /// A content clear or socket replacement retires audio/text state, while
    /// identities remain monotonic for the pipeline's retained replay fence.
    pub fn reset(&mut self) {
        *self = Self {
            sequence: self.sequence,
            ..Default::default()
        };
    }

    pub fn apply(&mut self, event: ServerEvent) -> Result<Option<TranscriptOutput>, ProtocolError> {
        match event {
            ServerEvent::Partial {
                text,
                is_final,
                speech_final,
                start: _,
                end,
            } => {
                if self.utterance_end.is_some_and(|old| end <= old) {
                    return Ok(None);
                }
                if speech_final {
                    let suffix = if text.trim().is_empty() {
                        ""
                    } else {
                        self.utterance.suffix(&text)?
                    };
                    let output = self.output(suffix, true)?;
                    self.utterance = CommittedPrefix::default();
                    self.utterance_end = Some(end);
                    self.committed_end = Some(end);
                    return Ok(output);
                }
                if self.committed_end.is_some_and(|old| end <= old) {
                    return Ok(None);
                }
                let output = self.output(&text, is_final)?;
                if is_final {
                    self.committed_end = Some(end);
                }
                Ok(output)
            }
            ServerEvent::Done { text: Some(text) } => {
                let suffix = self.session.suffix(&text)?;
                self.output(suffix, true)
            }
            ServerEvent::Done { text: None } if self.draft_active => Err(ProtocolError),
            _ => Ok(None),
        }
    }

    fn output(
        &mut self,
        text: &str,
        is_final: bool,
    ) -> Result<Option<TranscriptOutput>, ProtocolError> {
        let text = text.trim();
        if !subtitle_text_within_limit(text) {
            return Err(ProtocolError);
        }
        if text.is_empty() && !self.draft_active {
            return Ok(None);
        }
        if !self.draft_active {
            self.sequence = self.sequence.checked_add(1).ok_or(ProtocolError)?;
        }
        let output = TranscriptOutput {
            sequence: self.sequence,
            text: text.to_owned(),
            is_final,
        };
        self.draft_active = !is_final;
        if is_final {
            self.utterance.append(text);
            self.session.append(text);
        }
        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_language_codes_have_explicit_wire_aliases_and_auto_omits_hint() {
        for source in SourceLanguage::ALL {
            let result = endpoint(source);
            if source != SourceLanguage::Automatic && !LANGUAGE_CODES.contains(&source.raw_value())
            {
                assert!(result.is_err());
                continue;
            }
            let endpoint = result.unwrap();
            let query: std::collections::HashMap<_, _> = endpoint.query_pairs().collect();
            assert_eq!(query.get("model").unwrap(), MODEL);
            assert_eq!(query.get("sample_rate").unwrap(), "16000");
            assert_eq!(query.get("interim_results").unwrap(), "true");
            match source {
                SourceLanguage::Automatic => assert!(!query.contains_key("language")),
                SourceLanguage::Filipino => assert_eq!(query.get("language").unwrap(), "fil"),
                SourceLanguage::Norwegian => assert_eq!(query.get("language").unwrap(), "nb"),
                _ => assert_eq!(query.get("language").unwrap(), source.raw_value()),
            }
        }
    }

    fn result(text: &str, is_final: bool, speech_final: bool, start: f64, end: f64) -> ServerEvent {
        ServerEvent::Partial {
            text: text.to_owned(),
            is_final,
            speech_final,
            start,
            end,
        }
    }

    #[test]
    fn repeated_words_across_distinct_audio_spans_are_retained_and_old_spans_are_ignored() {
        let mut transcript = TranscriptAssembler::default();
        let first = transcript
            .apply(result("repeat", true, false, 0.0, 3.0))
            .unwrap()
            .unwrap();
        assert_eq!(first.sequence, 1);
        assert!(transcript
            .apply(result("repeat", true, false, 0.0, 3.0))
            .unwrap()
            .is_none());
        assert!(transcript
            .apply(result("repeat", true, true, 0.0, 3.0))
            .unwrap()
            .is_none());
        let second = transcript
            .apply(result("repeat", true, true, 3.0, 6.0))
            .unwrap()
            .unwrap();
        assert_eq!(second.sequence, 2);
        assert_eq!(second.text, "repeat");
        assert!(transcript
            .apply(result("stale", false, false, 0.0, 2.0))
            .unwrap()
            .is_none());
    }

    #[test]
    fn long_cumulative_summaries_emit_only_the_tail_without_retaining_prior_text() {
        let mut transcript = TranscriptAssembler::default();
        let chunk = "synthetic ".repeat(1000);
        for i in 0..10 {
            assert!(
                transcript
                    .apply(result(
                        &chunk,
                        true,
                        false,
                        f64::from(i * 3),
                        f64::from((i + 1) * 3)
                    ))
                    .unwrap()
                    .unwrap()
                    .is_final
            );
        }
        let summary = format!("{}tail", chunk.repeat(10));
        assert!(summary.len() > crate::core::models::MAX_SUBTITLE_TEXT_BYTES);
        let tail = transcript
            .apply(result(&summary, true, true, 0.0, 31.0))
            .unwrap()
            .unwrap();
        assert_eq!(tail.text, "tail");
        assert_eq!(tail.sequence, 11);
        assert!(transcript
            .apply(ServerEvent::Done {
                text: Some(summary)
            })
            .unwrap()
            .is_none());
    }

    #[test]
    fn stitched_summaries_ignore_spacing_but_reject_rewritten_locked_chunks() {
        let mut transcript = TranscriptAssembler::default();
        transcript
            .apply(result("合成字幕", true, false, 0.0, 3.0))
            .unwrap();
        let tail = transcript
            .apply(result("合成 字幕 继续", true, true, 0.0, 4.0))
            .unwrap()
            .unwrap();
        assert_eq!(tail.text, "继续");
        let mut transcript = TranscriptAssembler::default();
        transcript
            .apply(result("locked", true, false, 0.0, 3.0))
            .unwrap();
        assert!(transcript
            .apply(result("changed tail", true, true, 0.0, 4.0))
            .is_err());
    }

    #[test]
    fn empty_utterance_final_withdraws_only_the_current_draft() {
        let mut transcript = TranscriptAssembler::default();
        transcript
            .apply(result("locked", true, false, 0.0, 3.0))
            .unwrap();
        let draft = transcript
            .apply(result("unconfirmed", false, false, 3.0, 4.0))
            .unwrap()
            .unwrap();
        let withdrawal = transcript
            .apply(result("", true, true, 0.0, 4.0))
            .unwrap()
            .unwrap();
        assert_eq!(withdrawal.sequence, draft.sequence);
        assert!(withdrawal.is_final && withdrawal.text.is_empty());
        assert!(transcript
            .apply(ServerEvent::Done {
                text: Some("locked".into())
            })
            .unwrap()
            .is_none());
    }

    #[test]
    fn parser_bounds_drafts_and_sanitizes_errors() {
        let private = "synthetic-private-content";
        assert_eq!(
            decode(&json!({"type":"error","message":private,"code":private}).to_string()).unwrap(),
            ServerEvent::Rejected
        );
        assert!(decode(&json!({"type":"transcript.partial","text":"a", "is_final":false,"speech_final":true,"start":0,"duration":1}).to_string()).is_err());
        assert!(decode(&" ".repeat(MAX_MESSAGE_BYTES + 1)).is_err());
        assert!(decode(&json!({"type":"transcript.partial","text":"x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1), "is_final":false,"speech_final":false,"start":0,"duration":1}).to_string()).is_err());
    }
}
