//! Explicit custom speech-recognition contracts, separate from text Chat APIs.

use crate::core::models::{subtitle_text_within_limit, SourceLanguage};
use crate::core::provider::ProviderKind;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::VecDeque;
use thiserror::Error;

pub const MAX_CUSTOM_SPEECH_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_PCM_CHUNK_BYTES: usize = 128 * 1024;
pub const PCM_SAMPLE_RATE: u32 = 24_000;
#[cfg(test)]
const VAD_FRAME_BYTES: usize = 960; // 20 ms of mono PCM16 at 24 kHz.
const PREROLL_FRAMES: usize = 30;
const SILENCE_FRAMES: usize = 30;
const MAX_TURN_FRAMES: usize = 400;
#[cfg(test)]
const MIN_COMMIT_PCM_BYTES: usize = 4800; // Realtime requires at least 100 ms.
const VOICE_MEAN_SQUARE: u64 = 256 * 256;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CustomSpeechProtocolError {
    #[error("custom_speech_endpoint_invalid")]
    Endpoint,
    #[error("custom_speech_model_invalid")]
    Model,
    #[error("custom_speech_protocol_invalid")]
    Event,
    #[error("custom_speech_audio_invalid")]
    Audio,
}

/// Only an explicit full WebSocket URL is accepted. Credentials and query
/// parameters cannot be smuggled through the address. This canonical address
/// is safe to validate again after it is loaded from secure storage.
pub fn endpoint(
    value: &str,
    provider: ProviderKind,
) -> Result<url::Url, CustomSpeechProtocolError> {
    if !matches!(
        provider,
        ProviderKind::CustomDashScopeASR | ProviderKind::CustomOpenAIASR
    ) || value.len() > 4096
        || value.chars().any(char::is_control)
    {
        return Err(CustomSpeechProtocolError::Endpoint);
    }
    let url = url::Url::parse(value.trim()).map_err(|_| CustomSpeechProtocolError::Endpoint)?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "wss" || (url.scheme() == "ws" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CustomSpeechProtocolError::Endpoint);
    }
    Ok(url)
}

pub fn validate_model(value: &str) -> Result<String, CustomSpeechProtocolError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(CustomSpeechProtocolError::Model);
    }
    Ok(value.to_owned())
}

pub fn session_update(model: &str, source: SourceLanguage, event_id: &str) -> Value {
    let mut transcription = json!({ "model": model });
    if source != SourceLanguage::Automatic {
        // Current live transcription models accept expected languages as an
        // array; older committed-turn models use the singular field.
        if model == "gpt-live-transcribe" || model.starts_with("gpt-live-transcribe-") {
            transcription["languages"] = json!([source.raw_value()]);
        } else {
            transcription["language"] = json!(source.raw_value());
        }
    }
    json!({
        "type": "session.update", "event_id": event_id,
        "session": {
            "type": "transcription",
            "audio": { "input": {
                "format": { "type": "audio/pcm", "rate": PCM_SAMPLE_RATE },
                "transcription": transcription,
                "turn_detection": null
            }}
        }
    })
}

pub fn audio_append(pcm: &[u8]) -> Value {
    json!({ "type": "input_audio_buffer.append", "audio": base64::engine::general_purpose::STANDARD.encode(pcm) })
}

pub fn audio_commit() -> Value {
    json!({ "type": "input_audio_buffer.commit" })
}

pub fn audio_clear() -> Value {
    json!({ "type": "input_audio_buffer.clear" })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerEvent {
    Created,
    Updated { model: String },
    Committed { item_id: String },
    Cleared,
    Delta { item_id: String, text: String },
    Completed { item_id: String, text: String },
    Rejected,
    AuthenticationRejected,
    Ignored,
}

pub fn decode(text: &str) -> Result<ServerEvent, CustomSpeechProtocolError> {
    if text.len() > MAX_CUSTOM_SPEECH_MESSAGE_BYTES {
        return Err(CustomSpeechProtocolError::Event);
    }
    let event: Value = serde_json::from_str(text).map_err(|_| CustomSpeechProtocolError::Event)?;
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .ok_or(CustomSpeechProtocolError::Event)?;
    match kind {
        "session.created" => Ok(ServerEvent::Created),
        "session.updated" => {
            if event.pointer("/session/type").and_then(Value::as_str) != Some("transcription")
                || event
                    .pointer("/session/audio/input/format/type")
                    .and_then(Value::as_str)
                    != Some("audio/pcm")
                || event
                    .pointer("/session/audio/input/format/rate")
                    .and_then(Value::as_u64)
                    != Some(u64::from(PCM_SAMPLE_RATE))
                || event
                    .pointer("/session/audio/input/turn_detection")
                    .is_none_or(|value| !value.is_null())
            {
                return Err(CustomSpeechProtocolError::Event);
            }
            let model = event
                .pointer("/session/audio/input/transcription/model")
                .and_then(Value::as_str)
                .ok_or(CustomSpeechProtocolError::Event)?;
            Ok(ServerEvent::Updated {
                model: validate_model(model)?,
            })
        }
        "input_audio_buffer.committed" => Ok(ServerEvent::Committed {
            item_id: item_id(&event)?,
        }),
        "input_audio_buffer.cleared" => Ok(ServerEvent::Cleared),
        "conversation.item.input_audio_transcription.delta" => Ok(ServerEvent::Delta {
            item_id: item_id(&event)?,
            text: transcript(&event, "delta")?,
        }),
        "conversation.item.input_audio_transcription.completed" => Ok(ServerEvent::Completed {
            item_id: item_id(&event)?,
            text: transcript(&event, "transcript")?,
        }),
        // Never retain provider messages or arbitrary error codes.
        "error" | "conversation.item.input_audio_transcription.failed" => {
            if matches!(
                event.pointer("/error/code").and_then(Value::as_str),
                Some("invalid_api_key" | "authentication_error")
            ) {
                Ok(ServerEvent::AuthenticationRejected)
            } else {
                Ok(ServerEvent::Rejected)
            }
        }
        _ => Ok(ServerEvent::Ignored),
    }
}

fn item_id(event: &Value) -> Result<String, CustomSpeechProtocolError> {
    let id = event
        .get("item_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control))
        .ok_or(CustomSpeechProtocolError::Event)?;
    Ok(id.to_owned())
}

fn transcript(event: &Value, field: &str) -> Result<String, CustomSpeechProtocolError> {
    let text = event
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| subtitle_text_within_limit(text))
        .ok_or(CustomSpeechProtocolError::Event)?;
    Ok(text.to_owned())
}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioTurnAction {
    Start,
    Append(Vec<u8>),
    Commit,
}

/// RMS turn detection retains at most 600 ms of pre-roll and one partial
/// 20 ms frame. Active PCM is sent immediately, never collected as a recording.
pub struct PcmTurnGate {
    frame_bytes: usize,
    minimum_bytes: usize,
    frame: Vec<u8>,
    preroll: VecDeque<Vec<u8>>,
    active: bool,
    silence_frames: usize,
    turn_frames: usize,
    sent_bytes: usize,
}

impl Default for PcmTurnGate {
    fn default() -> Self {
        Self::with_sample_rate(24_000)
    }
}

impl PcmTurnGate {
    pub fn with_sample_rate(rate: usize) -> Self {
        assert!(matches!(rate, 16_000 | 24_000));
        Self {
            frame_bytes: rate / 50 * 2,
            minimum_bytes: rate / 10 * 2,
            frame: Vec::new(),
            preroll: VecDeque::new(),
            active: false,
            silence_frames: 0,
            turn_frames: 0,
            sent_bytes: 0,
        }
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn push(&mut self, pcm: &[u8]) -> Result<Vec<AudioTurnAction>, CustomSpeechProtocolError> {
        if pcm.len() > MAX_PCM_CHUNK_BYTES || !pcm.len().is_multiple_of(2) {
            return Err(CustomSpeechProtocolError::Audio);
        }
        let mut actions = Vec::new();
        for part in pcm.chunks(self.frame_bytes) {
            let mut remaining = part;
            while !remaining.is_empty() {
                let count = remaining.len().min(self.frame_bytes - self.frame.len());
                self.frame.extend_from_slice(&remaining[..count]);
                remaining = &remaining[count..];
                if self.frame.len() == self.frame_bytes {
                    let frame = std::mem::take(&mut self.frame);
                    self.push_frame(frame, &mut actions);
                }
            }
        }
        Ok(actions)
    }

    fn push_frame(&mut self, frame: Vec<u8>, actions: &mut Vec<AudioTurnAction>) {
        let voiced = is_voiced(&frame);
        if !self.active {
            self.preroll.push_back(frame);
            while self.preroll.len() > PREROLL_FRAMES {
                self.preroll.pop_front();
            }
            if !voiced {
                return;
            }
            self.active = true;
            self.turn_frames = self.preroll.len();
            self.sent_bytes = self.turn_frames * self.frame_bytes;
            self.silence_frames = 0;
            actions.push(AudioTurnAction::Start);
            while let Some(frame) = self.preroll.pop_front() {
                append(actions, frame);
            }
            return;
        }
        append(actions, frame);
        self.sent_bytes += self.frame_bytes;
        self.turn_frames += 1;
        self.silence_frames = if voiced { 0 } else { self.silence_frames + 1 };
        if self.silence_frames >= SILENCE_FRAMES || self.turn_frames >= MAX_TURN_FRAMES {
            actions.push(AudioTurnAction::Commit);
            self.active = false;
            self.silence_frames = 0;
            self.turn_frames = 0;
            self.sent_bytes = 0;
        }
    }

    pub fn finish(&mut self) -> Vec<AudioTurnAction> {
        let mut actions = Vec::new();
        if !self.active && is_voiced(&self.frame) {
            actions.push(AudioTurnAction::Start);
            self.sent_bytes = self.preroll.len() * self.frame_bytes;
            while let Some(frame) = self.preroll.pop_front() {
                append(&mut actions, frame);
            }
            self.active = true;
        }
        if self.active {
            self.sent_bytes += self.frame.len();
            append(&mut actions, std::mem::take(&mut self.frame));
            if self.sent_bytes < self.minimum_bytes {
                append(&mut actions, vec![0; self.minimum_bytes - self.sent_bytes]);
            }
            actions.push(AudioTurnAction::Commit);
        }
        *self = Self::with_sample_rate(self.frame_bytes * 25);
        actions
    }
}

pub const MAX_PENDING_TURNS: usize = 8;
const MAX_RETIRED_ITEM_IDS: usize = 32;

struct PendingTurn {
    sequence: u64,
    revision: u64,
    item_id: Option<String>,
    committed: bool,
    acknowledged: bool,
    draft: String,
    final_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptOutput {
    pub sequence: u64,
    pub revision: u64,
    pub text: String,
    pub is_final: bool,
}

/// Final callbacks can arrive out of order. Audio-turn order is allocated
/// before transmission, and only an acknowledged prefix can become durable.
#[derive(Default)]
pub struct TurnSequencer {
    next_sequence: u64,
    pending: VecDeque<PendingTurn>,
    retired_ids: VecDeque<String>,
    ignore_unbound_deltas: bool,
}

impl TurnSequencer {
    pub fn start(&mut self, revision: u64) -> Result<u64, CustomSpeechProtocolError> {
        if self.pending.len() >= MAX_PENDING_TURNS
            || self.pending.back().is_some_and(|turn| !turn.committed)
        {
            return Err(CustomSpeechProtocolError::Event);
        }
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        self.pending.push_back(PendingTurn {
            sequence: self.next_sequence,
            revision,
            item_id: None,
            committed: false,
            acknowledged: false,
            draft: String::new(),
            final_text: None,
        });
        Ok(self.next_sequence)
    }

    pub fn commit(&mut self) -> Result<(), CustomSpeechProtocolError> {
        let turn = self
            .pending
            .back_mut()
            .filter(|turn| !turn.committed)
            .ok_or(CustomSpeechProtocolError::Event)?;
        turn.committed = true;
        Ok(())
    }

    pub fn acknowledge(&mut self, item_id: String) -> Result<(), CustomSpeechProtocolError> {
        if self.retired_ids.contains(&item_id) {
            return Ok(());
        }
        let turn = if let Some(index) = self
            .pending
            .iter()
            .position(|turn| turn.item_id.as_ref() == Some(&item_id))
        {
            self.pending.get_mut(index).unwrap()
        } else {
            self.pending
                .iter_mut()
                .find(|turn| turn.committed && turn.item_id.is_none())
                .ok_or(CustomSpeechProtocolError::Event)?
        };
        if !turn.committed {
            return Err(CustomSpeechProtocolError::Event);
        }
        turn.item_id = Some(item_id);
        turn.acknowledged = true;
        Ok(())
    }

    pub fn delta(
        &mut self,
        item_id: String,
        delta: &str,
        revision: u64,
    ) -> Result<Option<TranscriptOutput>, CustomSpeechProtocolError> {
        if self.retired_ids.contains(&item_id) {
            return Ok(None);
        }
        let index = if let Some(index) = self
            .pending
            .iter()
            .position(|turn| turn.item_id.as_ref() == Some(&item_id))
        {
            index
        } else {
            // Live models may preview an uncommitted current turn. After a
            // clear, wait for its explicit commit identity instead of guessing
            // whether an unknown late delta belongs to old captured audio.
            if self.ignore_unbound_deltas
                || self
                    .pending
                    .iter()
                    .any(|turn| turn.committed && turn.item_id.is_none())
            {
                return Ok(None);
            }
            let Some(index) = self
                .pending
                .iter()
                .position(|turn| !turn.committed && turn.item_id.is_none())
            else {
                return Ok(None);
            };
            self.pending[index].item_id = Some(item_id);
            index
        };
        let latest_sequence = self.pending.back().map(|turn| turn.sequence);
        let turn = &mut self.pending[index];
        if turn.revision != revision || turn.final_text.is_some() {
            return Ok(None);
        }
        if turn.draft.len().saturating_add(delta.len())
            > crate::core::models::MAX_SUBTITLE_TEXT_BYTES
        {
            return Err(CustomSpeechProtocolError::Event);
        }
        turn.draft.push_str(delta);
        Ok(
            (Some(turn.sequence) == latest_sequence).then(|| TranscriptOutput {
                sequence: turn.sequence,
                revision: turn.revision,
                text: turn.draft.clone(),
                is_final: false,
            }),
        )
    }

    pub fn complete(
        &mut self,
        item_id: String,
        text: String,
        revision: u64,
    ) -> Result<Vec<TranscriptOutput>, CustomSpeechProtocolError> {
        if self.retired_ids.contains(&item_id) {
            return Ok(Vec::new());
        }
        let turn = self
            .pending
            .iter_mut()
            .find(|turn| turn.item_id.as_ref() == Some(&item_id))
            .ok_or(CustomSpeechProtocolError::Event)?;
        if !turn.committed || !turn.acknowledged || !subtitle_text_within_limit(&text) {
            return Err(CustomSpeechProtocolError::Event);
        }
        if turn.final_text.is_none() {
            turn.draft.clear();
            turn.final_text = Some(if turn.revision == revision {
                text
            } else {
                String::new()
            });
        }
        let mut output = Vec::new();
        while self
            .pending
            .front()
            .is_some_and(|turn| turn.committed && turn.acknowledged && turn.final_text.is_some())
        {
            let turn = self.pending.pop_front().unwrap();
            if let Some(item_id) = turn.item_id {
                self.retire(item_id);
            }
            let text = turn.final_text.unwrap();
            if turn.revision == revision && !text.trim().is_empty() {
                output.push(TranscriptOutput {
                    sequence: turn.sequence,
                    revision: turn.revision,
                    text,
                    is_final: true,
                });
            }
        }
        Ok(output)
    }

    pub fn clear(&mut self) {
        if self.pending.back().is_some_and(|turn| !turn.committed) {
            if let Some(turn) = self.pending.pop_back() {
                if let Some(item_id) = turn.item_id {
                    self.retire(item_id);
                }
            }
        }
        for turn in &mut self.pending {
            turn.draft.clear();
            if turn.final_text.is_some() {
                turn.final_text = Some(String::new());
            }
        }
        self.ignore_unbound_deltas = true;
    }

    /// The server acknowledged clearing its current uncommitted audio. Older
    /// committed identities remain tracked, but fresh live previews may bind
    /// to the next active audio turn again.
    pub fn buffer_cleared(&mut self) {
        self.ignore_unbound_deltas = false;
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn oldest_sequence(&self) -> Option<u64> {
        self.pending.front().map(|turn| turn.sequence)
    }

    fn retire(&mut self, item_id: String) {
        self.retired_ids.push_back(item_id);
        while self.retired_ids.len() > MAX_RETIRED_ITEM_IDS {
            self.retired_ids.pop_front();
        }
    }
}

fn append(actions: &mut Vec<AudioTurnAction>, pcm: Vec<u8>) {
    if pcm.is_empty() {
        return;
    }
    match actions.last_mut() {
        Some(AudioTurnAction::Append(previous))
            if previous.len() + pcm.len() <= MAX_PCM_CHUNK_BYTES =>
        {
            previous.extend(pcm)
        }
        _ => actions.push(AudioTurnAction::Append(pcm)),
    }
}

fn is_voiced(pcm: &[u8]) -> bool {
    let count = pcm.len() / 2;
    if count == 0 {
        return false;
    }
    let squares: u64 = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| {
            let value = i64::from(i16::from_le_bytes([sample[0], sample[1]]));
            (value * value) as u64
        })
        .sum();
    squares / count as u64 >= VOICE_MEAN_SQUARE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(frames: usize, amplitude: i16) -> Vec<u8> {
        amplitude
            .to_le_bytes()
            .into_iter()
            .cycle()
            .take(frames * VAD_FRAME_BYTES)
            .collect()
    }

    #[test]
    fn local_16khz_turns_keep_the_same_time_bounds_after_finish() {
        let mut gate = PcmTurnGate::with_sample_rate(16_000);
        for _ in 0..2 {
            let voice = 3000_i16.to_le_bytes().repeat(320);
            assert!(matches!(
                gate.push(&voice).unwrap().first(),
                Some(AudioTurnAction::Start)
            ));
            for _ in 0..398 {
                gate.push(&voice).unwrap();
            }
            assert!(matches!(
                gate.push(&voice).unwrap().last(),
                Some(AudioTurnAction::Commit)
            ));
            gate.finish();
            assert_eq!(gate.frame_bytes, 640);
            assert_eq!(gate.minimum_bytes, 3200);
        }
    }

    #[test]
    fn custom_addresses_are_explicit_secure_and_do_not_forward_queries() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            for address in [
                "https://example.com",
                "ws://example.com/asr",
                "wss://key:secret@example.com/asr",
                "wss://example.com/asr?api_key=secret",
                "wss://example.com/asr#key",
                "wss://example.com/asr\n",
            ] {
                assert_eq!(
                    endpoint(address, provider),
                    Err(CustomSpeechProtocolError::Endpoint)
                );
            }
            assert!(endpoint("ws://127.0.0.1:8765/asr", provider).is_ok());
            assert!(endpoint("ws://[::1]:8765/asr", provider).is_ok());
        }
        assert_eq!(
            endpoint(
                "wss://example.com/api-ws/v1/inference",
                ProviderKind::CustomDashScopeASR
            )
            .unwrap()
            .query(),
            None
        );
        let canonical = endpoint(
            "wss://example.com/v1/realtime",
            ProviderKind::CustomOpenAIASR,
        )
        .unwrap();
        assert!(canonical.query().is_none());
        assert_eq!(
            endpoint(canonical.as_str(), ProviderKind::CustomOpenAIASR).unwrap(),
            canonical
        );
        assert!(endpoint("wss://example.com/asr", ProviderKind::AlibabaCloud).is_err());
    }

    #[test]
    fn transcription_wire_contract_is_distinct_from_translation() {
        let update = session_update("synthetic-model", SourceLanguage::English, "setup");
        assert_eq!(update["session"]["type"], "transcription");
        assert_eq!(
            update["session"]["audio"]["input"]["transcription"]["language"],
            "en"
        );
        let live = session_update("gpt-live-transcribe", SourceLanguage::English, "setup");
        assert_eq!(
            live["session"]["audio"]["input"]["transcription"]["languages"],
            json!(["en"])
        );
        assert!(live["session"]["audio"]["input"]["transcription"]
            .get("language")
            .is_none());
        assert!(update["session"]["audio"]["input"]["turn_detection"].is_null());
        assert_eq!(audio_append(&[1, 2])["type"], "input_audio_buffer.append");
        assert_eq!(audio_commit()["type"], "input_audio_buffer.commit");
        let mut ack = update;
        ack["type"] = json!("session.updated");
        assert_eq!(
            decode(&ack.to_string()).unwrap(),
            ServerEvent::Updated {
                model: "synthetic-model".into()
            }
        );
        ack["session"]["type"] = json!("realtime");
        assert!(decode(&ack.to_string()).is_err());
        assert_eq!(
            decode(r#"{"type":"error","error":{"message":"synthetic-secret"}}"#).unwrap(),
            ServerEvent::Rejected
        );
    }

    #[test]
    fn silence_retention_is_bounded_and_speech_starts_with_preroll() {
        let mut gate = PcmTurnGate::default();
        for _ in 0..100 {
            assert!(gate.push(&pcm(5, 0)).unwrap().is_empty());
        }
        assert_eq!(gate.preroll.len(), PREROLL_FRAMES);
        let actions = gate.push(&pcm(1, 3000)).unwrap();
        assert_eq!(actions[0], AudioTurnAction::Start);
        assert!(
            matches!(&actions[1], AudioTurnAction::Append(bytes) if bytes.len() == PREROLL_FRAMES * VAD_FRAME_BYTES)
        );
        let actions = gate.push(&pcm(SILENCE_FRAMES, 0)).unwrap();
        assert_eq!(actions.last(), Some(&AudioTurnAction::Commit));
        assert!(!gate.active);
        assert!(gate.finish().is_empty());
    }

    #[test]
    fn continuous_audio_commits_at_eight_seconds_and_stop_flushes_partial_tail() {
        let mut gate = PcmTurnGate::default();
        let mut commits = 0;
        for _ in 0..MAX_TURN_FRAMES {
            commits += gate
                .push(&pcm(1, 3000))
                .unwrap()
                .into_iter()
                .filter(|action| *action == AudioTurnAction::Commit)
                .count();
        }
        assert_eq!(commits, 1);
        assert!(!gate.active);
        gate.push(&3000_i16.to_le_bytes()).unwrap();
        assert!(
            matches!(gate.finish().as_slice(), [AudioTurnAction::Start, AudioTurnAction::Append(bytes), AudioTurnAction::Commit] if bytes.len() == MIN_COMMIT_PCM_BYTES && bytes[..2] == 3000_i16.to_le_bytes())
        );
        assert!(gate.push(&[1]).is_err());
        assert!(gate.push(&vec![0; MAX_PCM_CHUNK_BYTES + 2]).is_err());
    }

    #[test]
    fn acknowledged_turns_finalize_in_capture_order_with_authoritative_text() {
        let mut turns = TurnSequencer::default();
        turns.start(0).unwrap();
        turns.commit().unwrap();
        turns.acknowledge("one".into()).unwrap();
        turns.start(0).unwrap();
        turns.commit().unwrap();
        turns.acknowledge("two".into()).unwrap();
        assert!(turns
            .delta("one".into(), "older draft", 0)
            .unwrap()
            .is_none());
        assert_eq!(
            turns
                .delta("two".into(), "latest", 0)
                .unwrap()
                .unwrap()
                .text,
            "latest"
        );
        assert!(turns
            .complete("two".into(), "second final".into(), 0)
            .unwrap()
            .is_empty());
        let result = turns
            .complete("one".into(), "first final".into(), 0)
            .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|output| (output.sequence, output.text.as_str()))
                .collect::<Vec<_>>(),
            [(1, "first final"), (2, "second final")]
        );
        assert!(turns
            .complete("one".into(), "duplicate".into(), 0)
            .unwrap()
            .is_empty());
        assert!(turns.is_empty());
    }

    #[test]
    fn live_deltas_bind_only_one_active_turn_and_are_replaceable() {
        let mut turns = TurnSequencer::default();
        turns.start(0).unwrap();
        assert_eq!(
            turns
                .delta("live".into(), "first ", 0)
                .unwrap()
                .unwrap()
                .text,
            "first "
        );
        assert_eq!(
            turns
                .delta("live".into(), "draft", 0)
                .unwrap()
                .unwrap()
                .text,
            "first draft"
        );
        turns.commit().unwrap();
        turns.acknowledge("live".into()).unwrap();
        assert_eq!(
            turns
                .complete("live".into(), "corrected final".into(), 0)
                .unwrap()[0]
                .text,
            "corrected final"
        );
        assert!(turns.delta("live".into(), "late", 0).unwrap().is_none());
    }

    #[test]
    fn clear_preserves_commit_identity_but_drops_old_text_and_ambiguous_previews() {
        let mut turns = TurnSequencer::default();
        turns.start(0).unwrap();
        turns.commit().unwrap();
        turns.start(0).unwrap();
        turns.delta("dropped".into(), "old preview", 0).unwrap();
        turns.clear();
        turns.start(1).unwrap();
        turns.acknowledge("old-commit".into()).unwrap();
        assert!(turns
            .delta("late-unknown".into(), "stale", 1)
            .unwrap()
            .is_none());
        assert!(turns
            .complete("old-commit".into(), "old final".into(), 1)
            .unwrap()
            .is_empty());
        assert!(turns.delta("dropped".into(), "stale", 1).unwrap().is_none());
        turns.commit().unwrap();
        turns.acknowledge("new".into()).unwrap();
        let output = turns.complete("new".into(), "fresh".into(), 1).unwrap();
        assert_eq!(output[0].revision, 1);
        assert_eq!(output[0].sequence, 3);
        assert_eq!(output[0].text, "fresh");
    }

    #[test]
    fn pending_items_drafts_and_protocol_frames_have_finite_limits() {
        let mut turns = TurnSequencer::default();
        for _ in 0..MAX_PENDING_TURNS {
            turns.start(0).unwrap();
            turns.commit().unwrap();
        }
        assert!(turns.start(0).is_err());
        let mut turns = TurnSequencer::default();
        turns.start(0).unwrap();
        turns
            .delta(
                "one".into(),
                &"x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES),
                0,
            )
            .unwrap();
        assert!(turns.delta("one".into(), "x", 0).is_err());
        turns.commit().unwrap();
        assert!(turns
            .complete("one".into(), "unacknowledged".into(), 0)
            .is_err());
        assert!(decode(&"x".repeat(MAX_CUSTOM_SPEECH_MESSAGE_BYTES + 1)).is_err());
        assert!(decode(
            r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":""}"#
        )
        .is_err());
    }

    #[test]
    fn verified_buffer_clear_restores_live_previews_without_reviving_old_ids() {
        let mut turns = TurnSequencer::default();
        turns.start(0).unwrap();
        turns.delta("old".into(), "old preview", 0).unwrap();
        turns.clear();
        turns.buffer_cleared();
        turns.start(1).unwrap();
        assert!(turns
            .delta("old".into(), "late old preview", 1)
            .unwrap()
            .is_none());
        assert_eq!(
            turns
                .delta("new".into(), "new preview", 1)
                .unwrap()
                .unwrap()
                .text,
            "new preview"
        );
        turns.commit().unwrap();
        turns.acknowledge("new".into()).unwrap();
        assert_eq!(
            turns.complete("new".into(), "new final".into(), 1).unwrap()[0].text,
            "new final"
        );
    }
}
