//! Volcano Engine Doubao Simultaneous Interpretation 2.0 wire protocol.
//!
//! The service sends one raw `TranslateRequest`/`TranslateResponse` protobuf
//! message in each WebSocket binary frame. This module intentionally encodes
//! and decodes only the fields Mimi needs instead of pulling a protobuf
//! runtime into the desktop app.

use crate::core::models::{SourceLanguage, TargetLanguage};
use thiserror::Error;

pub const SOURCE_LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "pt", "es", "ja", "id", "de", "fr", "ru", "it", "ko", "ar", "tr", "ms", "vi", "th",
    "nl", "ro", "pl", "cs", "yue", "wuu", "zh_en",
];
pub const TARGET_LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "pt", "es", "ja", "id", "de", "fr", "ru", "it", "ko", "ar", "tr", "ms", "vi", "th",
    "nl", "ro", "pl", "cs", "zh_en",
];
const S2T_LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "pt", "es", "ja", "id", "de", "fr", "ru", "it", "ko", "ar", "tr", "ms", "vi", "th",
    "nl", "ro", "pl", "cs",
];

const EVENT_START_SESSION: u32 = 100;
const EVENT_FINISH_SESSION: u32 = 102;
const EVENT_SESSION_STARTED: u32 = 150;
const EVENT_SESSION_FINISHED: u32 = 152;
const EVENT_SESSION_FAILED: u32 = 153;
const EVENT_TASK_REQUEST: u32 = 200;
const EVENT_SOURCE_SUBTITLE_START: u32 = 650;
const EVENT_SOURCE_SUBTITLE_RESPONSE: u32 = 651;
const EVENT_SOURCE_SUBTITLE_END: u32 = 652;
const EVENT_TRANSLATION_SUBTITLE_START: u32 = 653;
const EVENT_TRANSLATION_SUBTITLE_RESPONSE: u32 = 654;
const EVENT_TRANSLATION_SUBTITLE_END: u32 = 655;

const WIRE_VARINT: u8 = 0;
const WIRE_FIXED_64: u8 = 1;
const WIRE_LENGTH_DELIMITED: u8 = 2;
const WIRE_FIXED_32: u8 = 5;
const MAXIMUM_SESSION_ID_BYTES: usize = 128;
const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;
const MAXIMUM_RESPONSE_META_BYTES: usize = 64 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VolcanoEngineProtocolError {
    #[error("The Volcano Engine endpoint could not be created.")]
    InvalidEndpoint,
    #[error("Volcano Engine requires a non-empty session identifier.")]
    InvalidSessionID,
    #[error("Volcano Engine requires an explicit supported source language.")]
    UnsupportedSourceLanguage,
    #[error("Volcano Engine does not support this translation direction.")]
    UnsupportedTargetLanguage,
    #[error("Volcano Engine expected {expected_bytes} audio bytes, got {actual_bytes}.")]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("The Volcano Engine protobuf message exceeds its safety limit.")]
    MessageTooLarge,
    #[error("The Volcano Engine protobuf message is malformed.")]
    MalformedProtobuf,
    #[error("The Volcano Engine protobuf message uses an unsupported wire type.")]
    UnsupportedWireType,
    #[error("The Volcano Engine response is missing its event.")]
    MissingEvent,
    #[error("The Volcano Engine subtitle response is missing its text.")]
    MissingText,
    #[error("The Volcano Engine protobuf message repeats a singular field.")]
    DuplicateField,
}

pub struct VolcanoEngineEndpoint;

impl VolcanoEngineEndpoint {
    pub const WEBSOCKET_URL: &'static str =
        "wss://openspeech.bytedance.com/api/v4/ast/v2/translate";
    pub const RESOURCE_ID: &'static str = "volc.service_type.10053";
    pub const SAMPLE_RATE_HZ: u32 = 16_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 80;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;
    pub const MAXIMUM_SERVER_FRAME_BYTES: usize = 1_024 * 1_024;

    /// Returns the credential-free endpoint. Authentication is attached only
    /// to the transient WebSocket upgrade request.
    pub fn url() -> Result<url::Url, VolcanoEngineProtocolError> {
        url::Url::parse(Self::WEBSOCKET_URL)
            .map_err(|_| VolcanoEngineProtocolError::InvalidEndpoint)
    }
}

pub struct VolcanoEngineRequestEncoder;

impl VolcanoEngineRequestEncoder {
    pub fn validate_languages(
        source_language: SourceLanguage,
        target_language: TargetLanguage,
    ) -> Result<(), VolcanoEngineProtocolError> {
        source_language_code(source_language)?;
        target_language_code(target_language)?;
        if !supported_target_codes(source_language.raw_value())
            .contains(&target_language.raw_value())
        {
            return Err(VolcanoEngineProtocolError::UnsupportedTargetLanguage);
        }
        Ok(())
    }

    /// Encodes the official `TranslateRequest` StartSession shape. Mimi uses
    /// speech-to-text mode because it consumes subtitles, not generated audio.
    pub fn start_session(
        session_id: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
    ) -> Result<Vec<u8>, VolcanoEngineProtocolError> {
        validate_session_id(session_id)?;
        Self::validate_languages(source_language, target_language)?;
        let source_language = source_language_code(source_language)?;
        let target_language = target_language_code(target_language)?;

        let mut message = Vec::with_capacity(128);
        write_message_field(&mut message, 1, |request_meta| {
            write_string_field(request_meta, 6, session_id);
        });
        write_varint_field(&mut message, 2, EVENT_START_SESSION as u64);
        write_message_field(&mut message, 4, |source_audio| {
            // AST v2 names the raw PCM container `wav` and its codec `raw`.
            // The TaskRequest bytes themselves remain headerless PCM16.
            write_string_field(source_audio, 4, "wav");
            write_string_field(source_audio, 5, "raw");
            write_varint_field(
                source_audio,
                7,
                VolcanoEngineEndpoint::SAMPLE_RATE_HZ as u64,
            );
            write_varint_field(
                source_audio,
                8,
                VolcanoEngineEndpoint::BITS_PER_SAMPLE as u64,
            );
            write_varint_field(source_audio, 9, VolcanoEngineEndpoint::CHANNEL_COUNT as u64);
        });
        write_message_field(&mut message, 6, |request| {
            write_string_field(request, 1, "s2t");
            write_string_field(request, 2, source_language);
            write_string_field(request, 3, target_language);
        });
        Ok(message)
    }

    /// Encodes one exact 80 ms PCM16 mono audio frame as TaskRequest.
    pub fn audio(session_id: &str, pcm_data: &[u8]) -> Result<Vec<u8>, VolcanoEngineProtocolError> {
        validate_session_id(session_id)?;
        if pcm_data.len() != VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(VolcanoEngineProtocolError::InvalidAudioFrame {
                expected_bytes: VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT,
                actual_bytes: pcm_data.len(),
            });
        }

        let mut message = Vec::with_capacity(pcm_data.len().saturating_add(64));
        write_message_field(&mut message, 1, |request_meta| {
            write_string_field(request_meta, 6, session_id);
        });
        write_varint_field(&mut message, 2, EVENT_TASK_REQUEST as u64);
        write_message_field(&mut message, 4, |source_audio| {
            write_bytes_field(source_audio, 14, pcm_data);
        });
        Ok(message)
    }

    pub fn finish_session(session_id: &str) -> Result<Vec<u8>, VolcanoEngineProtocolError> {
        validate_session_id(session_id)?;
        let mut message = Vec::with_capacity(64);
        write_message_field(&mut message, 1, |request_meta| {
            write_string_field(request_meta, 6, session_id);
        });
        write_varint_field(&mut message, 2, EVENT_FINISH_SESSION as u64);
        Ok(message)
    }
}

fn validate_session_id(session_id: &str) -> Result<(), VolcanoEngineProtocolError> {
    if session_id.is_empty() || session_id.len() > MAXIMUM_SESSION_ID_BYTES {
        Err(VolcanoEngineProtocolError::InvalidSessionID)
    } else {
        Ok(())
    }
}

/// S2T requires Chinese or English on one side, except explicit bilingual reversal.
pub fn supported_target_codes(source: &str) -> &'static [&'static str] {
    match source {
        "zh" | "en" => S2T_LANGUAGE_CODES,
        "zh_en" => &["zh_en"],
        _ if SOURCE_LANGUAGE_CODES.contains(&source) => &["zh", "en"],
        _ => &[],
    }
}

fn wire_language_code(code: &'static str) -> &'static str {
    match code {
        "yue" => "yue-CN",
        "wuu" => "sh-CN",
        "zh_en" => "zhen",
        _ => code,
    }
}

fn source_language_code(
    source_language: SourceLanguage,
) -> Result<&'static str, VolcanoEngineProtocolError> {
    let code = source_language.raw_value();
    if SOURCE_LANGUAGE_CODES.contains(&code) {
        Ok(wire_language_code(code))
    } else {
        Err(VolcanoEngineProtocolError::UnsupportedSourceLanguage)
    }
}

fn target_language_code(
    target_language: TargetLanguage,
) -> Result<&'static str, VolcanoEngineProtocolError> {
    let code = target_language.raw_value();
    if TARGET_LANGUAGE_CODES.contains(&code) {
        Ok(wire_language_code(code))
    } else {
        Err(VolcanoEngineProtocolError::UnsupportedTargetLanguage)
    }
}

/// A content-safe representation of one official `TranslateResponse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolcanoEngineServerEvent {
    SessionStarted,
    SourceSubtitleStarted,
    SourceSubtitleDraft(String),
    SourceSubtitleFinal {
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    },
    TranslationSubtitleStarted,
    TranslationSubtitleDraft(String),
    TranslationSubtitleFinal {
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    },
    SessionFinished,
    SessionFailed {
        status_code: Option<i32>,
    },
    Ignored {
        event: i32,
    },
}

impl VolcanoEngineServerEvent {
    pub fn decode(frame: &[u8]) -> Result<Self, VolcanoEngineProtocolError> {
        if frame.len() > VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES {
            return Err(VolcanoEngineProtocolError::MessageTooLarge);
        }

        let mut reader = ProtoReader::new(frame);
        let mut event = None;
        let mut text = None;
        let mut status_code = None;
        let mut start_time_ms = None;
        let mut end_time_ms = None;
        let mut saw_response_meta = false;
        while !reader.is_empty() {
            let (field, wire_type) = reader.read_key()?;
            match field {
                1 => {
                    require_wire_type(wire_type, WIRE_LENGTH_DELIMITED)?;
                    if saw_response_meta {
                        return Err(VolcanoEngineProtocolError::DuplicateField);
                    }
                    saw_response_meta = true;
                    let response_meta = reader.read_length_delimited()?;
                    if response_meta.len() > MAXIMUM_RESPONSE_META_BYTES {
                        return Err(VolcanoEngineProtocolError::MessageTooLarge);
                    }
                    status_code = decode_response_meta(response_meta)?;
                }
                2 => {
                    require_wire_type(wire_type, WIRE_VARINT)?;
                    if event.is_some() {
                        return Err(VolcanoEngineProtocolError::DuplicateField);
                    }
                    event = Some(read_nonnegative_i32(&mut reader)?);
                }
                4 => {
                    require_wire_type(wire_type, WIRE_LENGTH_DELIMITED)?;
                    if text.is_some() {
                        return Err(VolcanoEngineProtocolError::DuplicateField);
                    }
                    let bytes = reader.read_length_delimited()?;
                    if bytes.len() > MAXIMUM_TRANSCRIPT_BYTES {
                        return Err(VolcanoEngineProtocolError::MessageTooLarge);
                    }
                    text = Some(
                        std::str::from_utf8(bytes)
                            .map_err(|_| VolcanoEngineProtocolError::MalformedProtobuf)?
                            .to_string(),
                    );
                }
                5 => {
                    require_wire_type(wire_type, WIRE_VARINT)?;
                    if start_time_ms.is_some() {
                        return Err(VolcanoEngineProtocolError::DuplicateField);
                    }
                    start_time_ms = Some(read_nonnegative_i32(&mut reader)?);
                }
                6 => {
                    require_wire_type(wire_type, WIRE_VARINT)?;
                    if end_time_ms.is_some() {
                        return Err(VolcanoEngineProtocolError::DuplicateField);
                    }
                    end_time_ms = Some(read_nonnegative_i32(&mut reader)?);
                }
                _ => reader.skip_value(wire_type)?,
            }
        }

        match event.ok_or(VolcanoEngineProtocolError::MissingEvent)? as u32 {
            EVENT_SESSION_STARTED => Ok(Self::SessionStarted),
            EVENT_SOURCE_SUBTITLE_START => Ok(Self::SourceSubtitleStarted),
            EVENT_SOURCE_SUBTITLE_RESPONSE => Ok(Self::SourceSubtitleDraft(
                text.ok_or(VolcanoEngineProtocolError::MissingText)?,
            )),
            EVENT_SOURCE_SUBTITLE_END => Ok(Self::SourceSubtitleFinal {
                text: text.ok_or(VolcanoEngineProtocolError::MissingText)?,
                start_time_ms,
                end_time_ms,
            }),
            EVENT_TRANSLATION_SUBTITLE_START => Ok(Self::TranslationSubtitleStarted),
            EVENT_TRANSLATION_SUBTITLE_RESPONSE => Ok(Self::TranslationSubtitleDraft(
                text.ok_or(VolcanoEngineProtocolError::MissingText)?,
            )),
            EVENT_TRANSLATION_SUBTITLE_END => Ok(Self::TranslationSubtitleFinal {
                text: text.ok_or(VolcanoEngineProtocolError::MissingText)?,
                start_time_ms,
                end_time_ms,
            }),
            EVENT_SESSION_FINISHED => Ok(Self::SessionFinished),
            EVENT_SESSION_FAILED => Ok(Self::SessionFailed { status_code }),
            event => Ok(Self::Ignored {
                event: i32::try_from(event)
                    .map_err(|_| VolcanoEngineProtocolError::MalformedProtobuf)?,
            }),
        }
    }
}

fn decode_response_meta(frame: &[u8]) -> Result<Option<i32>, VolcanoEngineProtocolError> {
    let mut reader = ProtoReader::new(frame);
    let mut status_code = None;
    while !reader.is_empty() {
        let (field, wire_type) = reader.read_key()?;
        match field {
            3 => {
                require_wire_type(wire_type, WIRE_VARINT)?;
                if status_code.is_some() {
                    return Err(VolcanoEngineProtocolError::DuplicateField);
                }
                status_code = Some(read_nonnegative_i32(&mut reader)?);
            }
            // The provider's free-form error message can contain user audio
            // transcripts. Validate and discard it so it never reaches logs.
            4 => {
                require_wire_type(wire_type, WIRE_LENGTH_DELIMITED)?;
                let message = reader.read_length_delimited()?;
                if message.len() > MAXIMUM_RESPONSE_META_BYTES
                    || std::str::from_utf8(message).is_err()
                {
                    return Err(VolcanoEngineProtocolError::MalformedProtobuf);
                }
            }
            _ => reader.skip_value(wire_type)?,
        }
    }
    Ok(status_code)
}

fn read_nonnegative_i32(reader: &mut ProtoReader<'_>) -> Result<i32, VolcanoEngineProtocolError> {
    let value = reader.read_varint()?;
    i32::try_from(value).map_err(|_| VolcanoEngineProtocolError::MalformedProtobuf)
}

fn require_wire_type(actual: u8, expected: u8) -> Result<(), VolcanoEngineProtocolError> {
    if actual == expected {
        Ok(())
    } else {
        Err(VolcanoEngineProtocolError::MalformedProtobuf)
    }
}

fn write_key(output: &mut Vec<u8>, field: u32, wire_type: u8) {
    write_varint(output, (u64::from(field) << 3) | u64::from(wire_type));
}

fn write_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn write_varint_field(output: &mut Vec<u8>, field: u32, value: u64) {
    write_key(output, field, WIRE_VARINT);
    write_varint(output, value);
}

fn write_string_field(output: &mut Vec<u8>, field: u32, value: &str) {
    write_bytes_field(output, field, value.as_bytes());
}

fn write_bytes_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
    write_key(output, field, WIRE_LENGTH_DELIMITED);
    write_varint(output, value.len() as u64);
    output.extend_from_slice(value);
}

fn write_message_field(output: &mut Vec<u8>, field: u32, encode: impl FnOnce(&mut Vec<u8>)) {
    let mut message = Vec::new();
    encode(&mut message);
    write_bytes_field(output, field, &message);
}

struct ProtoReader<'a> {
    remaining: &'a [u8],
}

impl<'a> ProtoReader<'a> {
    fn new(frame: &'a [u8]) -> Self {
        Self { remaining: frame }
    }

    fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }

    fn read_key(&mut self) -> Result<(u32, u8), VolcanoEngineProtocolError> {
        let key = self.read_varint()?;
        let field =
            u32::try_from(key >> 3).map_err(|_| VolcanoEngineProtocolError::MalformedProtobuf)?;
        if field == 0 {
            return Err(VolcanoEngineProtocolError::MalformedProtobuf);
        }
        let wire_type = (key & 0x07) as u8;
        Ok((field, wire_type))
    }

    fn read_varint(&mut self) -> Result<u64, VolcanoEngineProtocolError> {
        let mut value = 0_u64;
        for shift in (0..70).step_by(7) {
            let byte = *self
                .remaining
                .first()
                .ok_or(VolcanoEngineProtocolError::MalformedProtobuf)?;
            self.remaining = &self.remaining[1..];
            if shift == 63 && byte > 1 {
                return Err(VolcanoEngineProtocolError::MalformedProtobuf);
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(VolcanoEngineProtocolError::MalformedProtobuf)
    }

    fn read_length_delimited(&mut self) -> Result<&'a [u8], VolcanoEngineProtocolError> {
        let length = usize::try_from(self.read_varint()?)
            .map_err(|_| VolcanoEngineProtocolError::MessageTooLarge)?;
        if length > self.remaining.len() {
            return Err(VolcanoEngineProtocolError::MalformedProtobuf);
        }
        let (value, remaining) = self.remaining.split_at(length);
        self.remaining = remaining;
        Ok(value)
    }

    fn skip_value(&mut self, wire_type: u8) -> Result<(), VolcanoEngineProtocolError> {
        match wire_type {
            WIRE_VARINT => {
                self.read_varint()?;
            }
            WIRE_FIXED_64 => self.skip_exact(8)?,
            WIRE_LENGTH_DELIMITED => {
                self.read_length_delimited()?;
            }
            WIRE_FIXED_32 => self.skip_exact(4)?,
            _ => return Err(VolcanoEngineProtocolError::UnsupportedWireType),
        }
        Ok(())
    }

    fn skip_exact(&mut self, length: usize) -> Result<(), VolcanoEngineProtocolError> {
        if self.remaining.len() < length {
            return Err(VolcanoEngineProtocolError::MalformedProtobuf);
        }
        self.remaining = &self.remaining[length..];
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_s2t_catalog_validates_all_directions_and_serializes_every_legal_setup() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/translation-contracts.json"
        ))
        .unwrap();
        let catalog = &contract["providerLanguageCatalogs"]["volcanoEngine"];
        let pairs = catalog["targetsBySource"].as_object().unwrap();
        assert_eq!(
            serde_json::to_value(SOURCE_LANGUAGE_CODES).unwrap(),
            catalog["sourceLanguages"]
        );
        assert_eq!(
            serde_json::to_value(TARGET_LANGUAGE_CODES).unwrap(),
            catalog["targetLanguages"]
        );
        for source in SourceLanguage::ALL {
            for target in TargetLanguage::ALL {
                let allowed = pairs
                    .get(source.raw_value())
                    .and_then(|targets| targets.as_array())
                    .is_some_and(|targets| {
                        targets
                            .iter()
                            .any(|code| code.as_str() == Some(target.raw_value()))
                    });
                assert_eq!(
                    VolcanoEngineRequestEncoder::validate_languages(source, target).is_ok(),
                    allowed,
                    "{} -> {}",
                    source.raw_value(),
                    target.raw_value()
                );
            }
        }
        for case in catalog["setups"].as_array().unwrap() {
            let source = serde_json::from_value(case["source"].clone()).unwrap();
            let target = serde_json::from_value(case["target"].clone()).unwrap();
            let bytes =
                VolcanoEngineRequestEncoder::start_session("catalog-session", source, target)
                    .unwrap();
            let hex = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(hex, case["expectedHex"].as_str().unwrap());
        }
    }

    fn field_bytes(message: &[u8], wanted_field: u32) -> Option<&[u8]> {
        let mut reader = ProtoReader::new(message);
        while !reader.is_empty() {
            let (field, wire_type) = reader.read_key().ok()?;
            if field == wanted_field && wire_type == WIRE_LENGTH_DELIMITED {
                return reader.read_length_delimited().ok();
            }
            reader.skip_value(wire_type).ok()?;
        }
        None
    }

    fn field_varint(message: &[u8], wanted_field: u32) -> Option<u64> {
        let mut reader = ProtoReader::new(message);
        while !reader.is_empty() {
            let (field, wire_type) = reader.read_key().ok()?;
            if field == wanted_field && wire_type == WIRE_VARINT {
                return reader.read_varint().ok();
            }
            reader.skip_value(wire_type).ok()?;
        }
        None
    }

    fn response(event: u64, text: Option<&str>) -> Vec<u8> {
        let mut message = Vec::new();
        write_varint_field(&mut message, 2, event);
        if let Some(text) = text {
            write_string_field(&mut message, 4, text);
        }
        message
    }

    fn timed_response(event: u64, text: &str, start_time_ms: u64, end_time_ms: u64) -> Vec<u8> {
        let mut message = response(event, Some(text));
        write_varint_field(&mut message, 5, start_time_ms);
        write_varint_field(&mut message, 6, end_time_ms);
        message
    }

    #[test]
    fn endpoint_and_audio_contract_match_the_official_ast_v2_document() {
        assert_eq!(
            VolcanoEngineEndpoint::url().unwrap().as_str(),
            "wss://openspeech.bytedance.com/api/v4/ast/v2/translate"
        );
        assert_eq!(
            VolcanoEngineEndpoint::RESOURCE_ID,
            "volc.service_type.10053"
        );
        assert_eq!(VolcanoEngineEndpoint::SAMPLE_RATE_HZ, 16_000);
        assert_eq!(VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT, 2_560);
    }

    #[test]
    fn start_session_encodes_the_official_translate_request_fields() {
        let encoded = VolcanoEngineRequestEncoder::start_session(
            "session-123",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        )
        .unwrap();

        assert_eq!(field_varint(&encoded, 2), Some(EVENT_START_SESSION as u64));
        let meta = field_bytes(&encoded, 1).unwrap();
        assert_eq!(field_bytes(meta, 6), Some("session-123".as_bytes()));

        let source_audio = field_bytes(&encoded, 4).unwrap();
        assert_eq!(field_bytes(source_audio, 4), Some("wav".as_bytes()));
        assert_eq!(field_bytes(source_audio, 5), Some("raw".as_bytes()));
        assert_eq!(field_varint(source_audio, 7), Some(16_000));
        assert_eq!(field_varint(source_audio, 8), Some(16));
        assert_eq!(field_varint(source_audio, 9), Some(1));

        let request = field_bytes(&encoded, 6).unwrap();
        assert_eq!(field_bytes(request, 1), Some("s2t".as_bytes()));
        assert_eq!(field_bytes(request, 2), Some("ja".as_bytes()));
        assert_eq!(field_bytes(request, 3), Some("zh".as_bytes()));
    }

    #[test]
    fn task_request_carries_exactly_one_official_80_ms_audio_frame() {
        let pcm = vec![0x4a; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let encoded = VolcanoEngineRequestEncoder::audio("session-audio", &pcm).unwrap();
        assert_eq!(field_varint(&encoded, 2), Some(EVENT_TASK_REQUEST as u64));
        let source_audio = field_bytes(&encoded, 4).unwrap();
        assert_eq!(field_bytes(source_audio, 14), Some(pcm.as_slice()));

        assert_eq!(
            VolcanoEngineRequestEncoder::audio("session-audio", &[0; 10]).unwrap_err(),
            VolcanoEngineProtocolError::InvalidAudioFrame {
                expected_bytes: 2_560,
                actual_bytes: 10,
            }
        );
    }

    #[test]
    fn finish_session_uses_the_official_event_and_session_id() {
        let encoded = VolcanoEngineRequestEncoder::finish_session("session-finish").unwrap();
        assert_eq!(field_varint(&encoded, 2), Some(EVENT_FINISH_SESSION as u64));
        assert_eq!(
            field_bytes(field_bytes(&encoded, 1).unwrap(), 6),
            Some("session-finish".as_bytes())
        );
    }

    #[test]
    fn automatic_source_and_original_output_remain_unsupported() {
        for source in [
            SourceLanguage::Chinese,
            SourceLanguage::English,
            SourceLanguage::Japanese,
        ] {
            VolcanoEngineRequestEncoder::validate_languages(source, TargetLanguage::English)
                .unwrap();
        }
        assert_eq!(
            VolcanoEngineRequestEncoder::validate_languages(
                SourceLanguage::Automatic,
                TargetLanguage::English,
            )
            .unwrap_err(),
            VolcanoEngineProtocolError::UnsupportedSourceLanguage
        );
        assert_eq!(
            VolcanoEngineRequestEncoder::validate_languages(
                SourceLanguage::Chinese,
                TargetLanguage::Original,
            )
            .unwrap_err(),
            VolcanoEngineProtocolError::UnsupportedTargetLanguage
        );
    }

    #[test]
    fn subtitle_and_lifecycle_events_decode_from_official_field_numbers() {
        let cases = [
            (
                response(EVENT_SESSION_STARTED as u64, None),
                VolcanoEngineServerEvent::SessionStarted,
            ),
            (
                response(EVENT_SOURCE_SUBTITLE_START as u64, None),
                VolcanoEngineServerEvent::SourceSubtitleStarted,
            ),
            (
                response(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("hello")),
                VolcanoEngineServerEvent::SourceSubtitleDraft("hello".into()),
            ),
            (
                response(EVENT_SOURCE_SUBTITLE_END as u64, Some("hello.")),
                VolcanoEngineServerEvent::SourceSubtitleFinal {
                    text: "hello.".into(),
                    start_time_ms: None,
                    end_time_ms: None,
                },
            ),
            (
                response(EVENT_TRANSLATION_SUBTITLE_START as u64, None),
                VolcanoEngineServerEvent::TranslationSubtitleStarted,
            ),
            (
                response(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("你好")),
                VolcanoEngineServerEvent::TranslationSubtitleDraft("你好".into()),
            ),
            (
                response(EVENT_TRANSLATION_SUBTITLE_END as u64, Some("你好。")),
                VolcanoEngineServerEvent::TranslationSubtitleFinal {
                    text: "你好。".into(),
                    start_time_ms: None,
                    end_time_ms: None,
                },
            ),
            (
                response(EVENT_SESSION_FINISHED as u64, None),
                VolcanoEngineServerEvent::SessionFinished,
            ),
        ];

        for (wire, expected) in cases {
            assert_eq!(VolcanoEngineServerEvent::decode(&wire).unwrap(), expected);
        }
    }

    #[test]
    fn final_subtitle_events_preserve_official_sentence_timing_fields() {
        assert_eq!(
            VolcanoEngineServerEvent::decode(&timed_response(
                EVENT_SOURCE_SUBTITLE_END as u64,
                "hello.",
                1_200,
                2_340,
            ))
            .unwrap(),
            VolcanoEngineServerEvent::SourceSubtitleFinal {
                text: "hello.".into(),
                start_time_ms: Some(1_200),
                end_time_ms: Some(2_340),
            }
        );
        assert_eq!(
            VolcanoEngineServerEvent::decode(&timed_response(
                EVENT_TRANSLATION_SUBTITLE_END as u64,
                "你好。",
                1_200,
                2_340,
            ))
            .unwrap(),
            VolcanoEngineServerEvent::TranslationSubtitleFinal {
                text: "你好。".into(),
                start_time_ms: Some(1_200),
                end_time_ms: Some(2_340),
            }
        );
    }

    #[test]
    fn session_failure_keeps_only_the_numeric_status_code() {
        let mut response_meta = Vec::new();
        write_varint_field(&mut response_meta, 3, 45_000_001);
        write_string_field(
            &mut response_meta,
            4,
            "private provider detail that must never reach diagnostics",
        );
        let mut message = Vec::new();
        write_bytes_field(&mut message, 1, &response_meta);
        write_varint_field(&mut message, 2, EVENT_SESSION_FAILED as u64);

        assert_eq!(
            VolcanoEngineServerEvent::decode(&message).unwrap(),
            VolcanoEngineServerEvent::SessionFailed {
                status_code: Some(45_000_001),
            }
        );
    }

    #[test]
    fn unknown_forward_compatible_fields_are_skipped_by_wire_type() {
        let mut message = response(EVENT_SESSION_STARTED as u64, None);
        write_key(&mut message, 20, WIRE_VARINT);
        write_varint(&mut message, 42);
        write_key(&mut message, 21, WIRE_FIXED_64);
        message.extend_from_slice(&[0; 8]);
        write_key(&mut message, 22, WIRE_LENGTH_DELIMITED);
        write_varint(&mut message, 3);
        message.extend_from_slice(&[1, 2, 3]);
        write_key(&mut message, 23, WIRE_FIXED_32);
        message.extend_from_slice(&[0; 4]);
        assert_eq!(
            VolcanoEngineServerEvent::decode(&message).unwrap(),
            VolcanoEngineServerEvent::SessionStarted
        );
    }

    #[test]
    fn malformed_or_ambiguous_protobuf_is_rejected() {
        let cases = [
            vec![],
            vec![0],
            vec![0x10, 0x80],
            vec![0x12, 0x01, 0x96],
            vec![0x10, 0x96, 0x01, 0x10, 0x96, 0x01],
            vec![0x10, 0x8b, 0x05],
            vec![0x10, 0x8b, 0x05, 0x22, 0x01, 0xff],
            vec![0x10, 0x96, 0x01, 0x1b],
        ];
        for wire in cases {
            assert!(VolcanoEngineServerEvent::decode(&wire).is_err(), "{wire:?}");
        }
    }

    #[test]
    fn server_frame_and_transcript_safety_limits_are_enforced() {
        let oversized = vec![0; VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES + 1];
        assert_eq!(
            VolcanoEngineServerEvent::decode(&oversized).unwrap_err(),
            VolcanoEngineProtocolError::MessageTooLarge
        );

        let mut message = response(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, None);
        write_key(&mut message, 4, WIRE_LENGTH_DELIMITED);
        write_varint(&mut message, (MAXIMUM_TRANSCRIPT_BYTES + 1) as u64);
        message.resize(message.len() + MAXIMUM_TRANSCRIPT_BYTES + 1, b'x');
        assert_eq!(
            VolcanoEngineServerEvent::decode(&message).unwrap_err(),
            VolcanoEngineProtocolError::MessageTooLarge
        );
    }
}
