//! Dedicated Gemini Live transcription (not Live agent input transcription).
//! https://ai.google.dev/gemini-api/docs/live-api/live-transcribe
use super::gemini_live::GeminiLiveEndpoint;
use crate::core::models::{subtitle_text_within_limit, SourceLanguage};
use base64::Engine;
use serde_json::{json, Map, Value};

pub const MODEL: &str = "gemini-3.5-transcribe-live";
pub const PCM_SAMPLE_RATE: u32 = 16_000;
pub const MAX_PCM_CHUNK_BYTES: usize = 128 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolError;

pub fn endpoint(source: SourceLanguage) -> Result<url::Url, ProtocolError> {
    if source != SourceLanguage::Automatic && language_hint(source).is_none() {
        return Err(ProtocolError);
    }
    GeminiLiveEndpoint::url().map_err(|_| ProtocolError)
}

pub fn setup(source: SourceLanguage) -> Result<Value, ProtocolError> {
    let languages = if source == SourceLanguage::Automatic {
        vec![]
    } else {
        vec![language_hint(source).ok_or(ProtocolError)?]
    };
    Ok(json!({"setup": {
        "model": format!("models/{MODEL}"),
        "generationConfig": { "responseModalities": ["TEXT"] },
        "inputAudioTranscription": { "languageCodes": languages }
    }}))
}

pub fn audio(pcm: &[u8]) -> Value {
    let mut audio = Map::new();
    audio.insert(
        "data".into(),
        Value::String(base64::engine::general_purpose::STANDARD.encode(pcm)),
    );
    audio.insert("mimeType".into(), "audio/pcm;rate=16000".into());
    let mut input = Map::new();
    input.insert("audio".into(), Value::Object(audio));
    let mut message = Map::new();
    message.insert("realtimeInput".into(), Value::Object(input));
    Value::Object(message)
}
pub fn finish() -> Value {
    json!({"realtimeInput":{"audioStreamEnd":true}})
}

// BCP-47 language hints come from the dedicated transcription language table.
pub fn language_hint(source: SourceLanguage) -> Option<&'static str> {
    super::realtime_asr::language_hint(
        crate::core::provider::ProviderKind::GoogleGeminiLive,
        source,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    Created,
    Transcript {
        interim: Option<String>,
        final_text: Option<String>,
        turn_complete: bool,
    },
    Rejected,
    GoAway,
    AuthenticationRejected,
    Ignored,
}

pub fn decode(text: &str) -> Result<ServerEvent, ProtocolError> {
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| ProtocolError)?;
    if let Some(error) = value.get("error") {
        return Ok(
            if matches!(error.get("code").and_then(Value::as_i64), Some(401 | 403)) {
                ServerEvent::AuthenticationRejected
            } else {
                ServerEvent::Rejected
            },
        );
    }
    if value.get("setupComplete").is_some() {
        return Ok(ServerEvent::Created);
    }
    if value.get("goAway").is_some() {
        return Ok(ServerEvent::GoAway);
    }
    if let Some(content) = value.get("serverContent") {
        let read = |field: &str| -> Result<Option<String>, ProtocolError> {
            let Some(transcript) = content.get(field) else {
                return Ok(None);
            };
            let text = transcript
                .get("text")
                .and_then(Value::as_str)
                .ok_or(ProtocolError)?;
            if !subtitle_text_within_limit(text) {
                return Err(ProtocolError);
            }
            Ok(Some(text.to_owned()))
        };
        // These fields have distinct semantics: interim is replaceable and
        // inputTranscription is a complete authoritative speech segment.
        return Ok(ServerEvent::Transcript {
            interim: read("interimInputTranscription")?,
            final_text: read("inputTranscription")?,
            turn_complete: content
                .get("turnComplete")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    if value.get("usageMetadata").is_some() || value.get("sessionResumptionUpdate").is_some() {
        return Ok(ServerEvent::Ignored);
    }
    Err(ProtocolError)
}

#[derive(Debug, PartialEq, Eq)]
pub struct TranscriptOutput {
    pub sequence: u64,
    pub text: String,
    pub is_final: bool,
}
#[derive(Default)]
pub struct TranscriptAssembler {
    sequence: u64,
}
impl TranscriptAssembler {
    pub fn reset(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
    }
    pub fn apply(&mut self, event: ServerEvent) -> Result<Vec<TranscriptOutput>, ProtocolError> {
        let ServerEvent::Transcript {
            interim,
            final_text,
            ..
        } = event
        else {
            return Err(ProtocolError);
        };
        let mut output = Vec::new();
        if let Some(text) = final_text {
            output.push(TranscriptOutput {
                sequence: self.sequence,
                text,
                is_final: true,
            });
            self.sequence = self.sequence.wrapping_add(1);
        } else if let Some(text) = interim {
            output.push(TranscriptOutput {
                sequence: self.sequence,
                text,
                is_final: false,
            });
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authoritative_segments_replace_interims_and_advance_without_retaining_history() {
        let mut transcript = TranscriptAssembler::default();
        let draft = transcript
            .apply(
                decode(r#"{"serverContent":{"interimInputTranscription":{"text":"draft"}}}"#)
                    .unwrap(),
            )
            .unwrap();
        assert!(!draft[0].is_final);
        let final_text = transcript.apply(decode(r#"{"serverContent":{"interimInputTranscription":{"text":"stale"},"inputTranscription":{"text":"final"}}}"#).unwrap()).unwrap();
        assert_eq!(final_text.len(), 1);
        assert!(final_text[0].is_final);
        assert_eq!(draft[0].sequence, final_text[0].sequence);
        let next = transcript
            .apply(decode(r#"{"serverContent":{"inputTranscription":{"text":"next"}}}"#).unwrap())
            .unwrap();
        assert_ne!(next[0].sequence, final_text[0].sequence);
    }
    #[test]
    fn dedicated_transcription_never_requests_agent_translation_or_audio_output() {
        let setup = setup(SourceLanguage::English).unwrap();
        assert_eq!(
            setup["setup"]["generationConfig"],
            json!({"responseModalities":["TEXT"]})
        );
        assert_eq!(
            setup["setup"]["inputAudioTranscription"]["languageCodes"],
            json!(["en-US"])
        );
        assert!(setup["setup"].get("systemInstruction").is_none());
    }
    #[test]
    fn malformed_and_unbounded_transcripts_fail_closed() {
        assert!(decode(r#"{"serverContent":{"inputTranscription":{}}}"#).is_err());
        assert!(decode(
            &json!({"serverContent":{"inputTranscription":{"text":"x".repeat(100_000)}}})
                .to_string()
        )
        .is_err());
        assert_eq!(
            decode(r#"{"goAway":{"timeLeft":"0s"}}"#),
            Ok(ServerEvent::GoAway)
        );
    }
}
