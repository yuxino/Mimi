//! Test-only standalone Qwen3 ASR Realtime arm. This is not LiveTranslate/MT.
//! See the accepted benchmark note for the exact official event references.
use super::*;
use crate::core::protocols::realtime::live_translate::{
    LiveTranslateRequestEncoder, DASHSCOPE_REALTIME_WS,
};

pub(super) const MODEL: &str = "qwen3-asr-flash-realtime-2026-02-10";
const MAX_ITEM_ID_BYTES: usize = 256;

pub(super) fn endpoint() -> String {
    format!("{DASHSCOPE_REALTIME_WS}?model={MODEL}")
}

fn session_update(language: Language) -> serde_json::Value {
    let mut transcription = serde_json::json!({});
    if !matches!(language, Language::Automatic) {
        transcription["language"] = language.source().raw_value().into();
    }
    // Normative client format is `pcm`; the server reference also uses `pcm16`
    // in acknowledgement examples. Never send a translation/corpus/prompt.
    serde_json::json!({
        "event_id": uuid::Uuid::new_v4().to_string(),
        "type": "session.update",
        "session": {
            "modalities": ["text"],
            "input_audio_format": "pcm",
            "sample_rate": 16000,
            "input_audio_transcription": transcription,
            "turn_detection": {
                "type": "server_vad", "threshold": 0.2, "silence_duration_ms": 800
            }
        }
    })
}

// Fixed bit labels are documented in the note; never serialize the ack itself.
fn ack_mismatch_mask(value: &serde_json::Value, language: Language) -> u64 {
    let Some(session) = value.get("session").filter(|session| session.is_object()) else {
        return 1;
    };
    let format_valid = matches!(
        session
            .get("input_audio_format")
            .and_then(|value| value.as_str()),
        Some("pcm" | "pcm16")
    );
    let language_valid = match language {
        Language::Automatic => session
            .pointer("/input_audio_transcription/language")
            .is_none_or(|value| value.is_null() || value.as_str() == Some("")),
        _ => {
            session
                .pointer("/input_audio_transcription/language")
                .and_then(|value| value.as_str())
                == Some(language.source().raw_value())
        }
    };
    let rate_valid = session
        .get("sample_rate")
        .is_none_or(|value| value.as_u64() == Some(16000));
    let modalities_valid = session
        .get("modalities")
        .is_none_or(|value| *value == serde_json::json!(["text"]));
    let model_valid = session
        .get("model")
        .is_none_or(|value| matches!(value.as_str(), Some(MODEL | "qwen3-asr-flash-realtime")));
    let vad_valid = session
        .pointer("/turn_detection/type")
        .and_then(|value| value.as_str())
        == Some("server_vad");
    let threshold_valid = session
        .pointer("/turn_detection/threshold")
        .and_then(|value| value.as_f64())
        == Some(0.2);
    let silence_valid = session
        .pointer("/turn_detection/silence_duration_ms")
        .and_then(|value| value.as_u64())
        == Some(800);
    (u64::from(!format_valid) * 2)
        | (u64::from(!language_valid) * 4)
        | (u64::from(!rate_valid) * 8)
        | (u64::from(!modalities_valid) * 16)
        | (u64::from(!model_valid) * 32)
        | (u64::from(!vad_valid) * 64)
        | (u64::from(!threshold_valid) * 128)
        | (u64::from(!silence_valid) * 256)
}

fn acknowledged(value: &serde_json::Value, language: Language) -> bool {
    ack_mismatch_mask(value, language) == 0
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Input,
    Connect,
    Ready,
    Streaming,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Signal {
    Ready,
    Finished,
    Other,
}

struct Item {
    id: String,
    ordinal: u64,
    begin_ms: Option<u64>,
    end_ms: Option<u64>,
    stop_received_ms: Option<u64>,
    final_seen: bool,
}

struct RealtimeObserver {
    observer: Observer,
    // Opaque actual IDs are bounded private bookkeeping, never serialized.
    // Ordinals are only numeric report labels, not ordered server watermarks.
    items: Vec<Item>,
    // Only the latest item's text/stash are retained for revision comparison.
    latest: Option<(u64, String, String)>,
}

impl RealtimeObserver {
    fn new(clip: &PreparedClip) -> Self {
        Self {
            observer: Observer::new(clip),
            items: Vec::new(),
            latest: None,
        }
    }

    fn item(&mut self, value: &serde_json::Value) -> Result<usize, Failure> {
        let id = value
            .get("item_id")
            .and_then(|value| value.as_str())
            .filter(|id| !id.is_empty() && id.len() <= MAX_ITEM_ID_BYTES)
            .ok_or(Failure::ProtocolInvalid)?;
        if let Some(index) = self.items.iter().position(|item| item.id == id) {
            return Ok(index);
        }
        if self.items.len() == MAX_FINAL_SAMPLES {
            return Err(Failure::EventLimit);
        }
        self.items.push(Item {
            id: id.to_owned(),
            ordinal: self.items.len() as u64 + 1,
            begin_ms: None,
            end_ms: None,
            stop_received_ms: None,
            final_seen: false,
        });
        Ok(self.items.len() - 1)
    }

    fn record(
        &mut self,
        wire: &str,
        at_ms: u64,
        finished_sending: bool,
    ) -> Result<Signal, Failure> {
        if wire.len() > MAX_AUDIO3_MESSAGE_BYTES {
            return Err(Failure::ProtocolInvalid);
        }
        self.observer.events += 1;
        if self.observer.events > MAX_EVENTS {
            return Err(Failure::EventLimit);
        }
        let value: serde_json::Value =
            serde_json::from_str(wire).map_err(|_| Failure::ProtocolInvalid)?;
        let kind = value
            .get("type")
            .and_then(|value| value.as_str())
            .ok_or(Failure::ProtocolInvalid)?;
        match kind {
            "error" | "conversation.item.input_audio_transcription.failed" => {
                // Do not inspect/format free-text messages, params or event IDs.
                let code = value
                    .pointer("/error/code")
                    .and_then(|value| value.as_str());
                let category = value
                    .pointer("/error/type")
                    .and_then(|value| value.as_str());
                return Err(
                    if matches!(
                        code,
                        Some(
                            "invalid_api_key"
                                | "authentication_error"
                                | "unauthorized"
                                | "invalid_authentication"
                        )
                    ) || category == Some("authentication_error")
                    {
                        Failure::AuthenticationRejected
                    } else {
                        Failure::TaskRejected
                    },
                );
            }
            "session.updated" => {
                let mismatch = ack_mismatch_mask(&value, self.observer.language);
                self.observer.metrics.ready_ack_mismatch_mask = mismatch;
                if mismatch != 0 {
                    return Err(Failure::ProtocolInvalid);
                }
                return Ok(Signal::Ready);
            }
            "session.finished" => return Ok(Signal::Finished),
            "input_audio_buffer.speech_started" | "input_audio_buffer.speech_stopped" => {
                let index = self.item(&value)?;
                let item = &mut self.items[index];
                let numeric = |name: &str| {
                    value
                        .get(name)
                        .and_then(|value| value.as_u64())
                        .filter(|value| *value <= 1_000_000_000)
                };
                if kind == "input_audio_buffer.speech_started" {
                    item.begin_ms = numeric("audio_start_ms");
                } else {
                    item.end_ms = numeric("audio_end_ms");
                    item.stop_received_ms = item.end_ms.map(|_| at_ms);
                }
            }
            "conversation.item.input_audio_transcription.text" => {
                let index = self.item(&value)?;
                let ordinal = self.items[index].ordinal;
                let text = value
                    .get("text")
                    .and_then(|value| value.as_str())
                    .ok_or(Failure::ProtocolInvalid)?;
                let stash = value
                    .get("stash")
                    .and_then(|value| value.as_str())
                    .ok_or(Failure::ProtocolInvalid)?;
                if text.len().saturating_add(stash.len()) > MAX_TEXT_BYTES {
                    return Err(Failure::ProtocolInvalid);
                }
                let metrics = &mut self.observer.metrics;
                metrics.stable_prefix_events += 1;
                if !text.is_empty() {
                    metrics.first_stable_prefix_ms.get_or_insert(at_ms);
                }
                let prefix_classes = character_classes(text);
                metrics.stable_prefix_latin_characters += prefix_classes.latin;
                metrics.stable_prefix_han_characters += prefix_classes.han;
                metrics.stable_prefix_kana_characters += prefix_classes.kana;
                let stash_classes = character_classes(stash);
                metrics.stash_latin_characters += stash_classes.latin;
                metrics.stash_han_characters += stash_classes.han;
                metrics.stash_kana_characters += stash_classes.kana;
                if let Some((owner, previous_text, previous_stash)) = &self.latest {
                    if *owner == ordinal {
                        metrics.stable_prefix_changes += u64::from(previous_text != text);
                        let retracted = retracted_characters(previous_text, text);
                        metrics.stable_prefix_retractions += u64::from(retracted > 0);
                        metrics.stable_prefix_retracted_characters += retracted;
                        metrics.stable_prefix_retracted_han_characters +=
                            retracted_han(previous_text, text);
                        metrics.stash_revisions += u64::from(previous_stash != stash);
                        let retracted = retracted_characters(previous_stash, stash);
                        metrics.stash_retractions += u64::from(retracted > 0);
                        metrics.stash_retracted_characters += retracted;
                    }
                }
                self.latest = Some((ordinal, text.to_owned(), stash.to_owned()));
                // The protocol defines literal text + stash. No guessed spaces,
                // language correction or sentence stitching are introduced.
                self.observer.observe_caption(
                    CaptionObservation {
                        caption: &format!("{text}{stash}"),
                        is_final: false,
                        sentence_id: Some(ordinal),
                        duplicate_final: false,
                        begin_time_ms: None,
                        end_time_ms: None,
                    },
                    at_ms,
                    finished_sending,
                )?;
            }
            "conversation.item.input_audio_transcription.completed" => {
                let index = self.item(&value)?;
                let item = &mut self.items[index];
                let transcript = value
                    .get("transcript")
                    .and_then(|value| value.as_str())
                    .ok_or(Failure::ProtocolInvalid)?;
                if transcript.len() > MAX_TEXT_BYTES {
                    return Err(Failure::ProtocolInvalid);
                }
                let duplicate = item.final_seen;
                if !duplicate {
                    if let Some((owner, text, _)) = &self.latest {
                        if *owner == item.ordinal {
                            self.observer.metrics.stable_prefix_final_retractions +=
                                u64::from(retracted_characters(text, transcript) > 0);
                            self.observer
                                .metrics
                                .stable_prefix_final_retracted_han_characters +=
                                retracted_han(text, transcript);
                        }
                    }
                }
                self.observer.observe_caption(
                    CaptionObservation {
                        caption: transcript,
                        is_final: true,
                        sentence_id: Some(item.ordinal),
                        duplicate_final: duplicate,
                        begin_time_ms: item.begin_ms,
                        end_time_ms: None,
                    },
                    at_ms,
                    finished_sending,
                )?;
                // These are real provider clocks, but their zero point is not
                // proven equal to our ready->send clock. Do not manufacture the
                // Audio3 end-delay by subtracting unlike clocks.
                let sample = self.observer.metrics.finals.last_mut().unwrap();
                sample.end_time_ms = item.end_ms;
                sample.speech_stop_to_final_ms = item
                    .stop_received_ms
                    .filter(|received| *received <= at_ms)
                    .map(|received| at_ms - received);
                if self
                    .latest
                    .as_ref()
                    .is_some_and(|(owner, _, _)| *owner == item.ordinal)
                {
                    self.latest = None;
                }
                item.final_seen = true;
            }
            _ => {}
        }
        Ok(Signal::Other)
    }
}

fn retracted_characters(previous: &str, current: &str) -> u64 {
    let common = previous
        .chars()
        .zip(current.chars())
        .take_while(|(a, b)| a == b)
        .count();
    previous.chars().count().saturating_sub(common) as u64
}

fn retracted_han(previous: &str, current: &str) -> u64 {
    let common = previous
        .chars()
        .zip(current.chars())
        .take_while(|(a, b)| a == b)
        .count();
    character_classes(&previous.chars().skip(common).collect::<String>()).han
}

pub(super) async fn compare(
    clip: &PreparedClip,
    api_key: &str,
    network: &ProviderNetwork,
    endpoint: &str,
) -> Report {
    let mut state = RealtimeObserver::new(clip);
    let mut sent = Metrics::default();
    let operation = async {
        state.observer.metrics.realtime_stage = Some(Stage::Input);
        if clip.pcm.is_empty() || clip.pcm.len() > MAX_PCM_BYTES || !clip.pcm.len().is_multiple_of(2) {
            return Err(Failure::InputInvalid);
        }
        let mut request = endpoint.into_client_request().map_err(|_| Failure::NetworkFailed)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| Failure::CredentialsUnavailable)?;
        authorization.set_sensitive(true);
        request.headers_mut().insert("Authorization", authorization);
        state.observer.metrics.realtime_stage = Some(Stage::Connect);
        let connecting = Instant::now();
        let (mut socket, _) = provider_network::websocket_with_message_limit(request, network, MAX_AUDIO3_MESSAGE_BYTES).await.map_err(websocket_failure)?;
        tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(session_update(clip.language).to_string().into())))
            .await.map_err(|_| Failure::SendTimeout)?.map_err(websocket_failure)?;
        state.observer.metrics.realtime_stage = Some(Stage::Ready);
        let ready_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let message = tokio::time::timeout_at(ready_deadline, socket.next()).await.map_err(|_| Failure::ReadyTimeout)?
                .ok_or(Failure::NetworkFailed)?.map_err(websocket_failure)?;
            let Some(text) = text_frame(message)? else { continue; };
            match state.record(&text, 0, false)? {
                Signal::Ready => break,
                Signal::Finished => return Err(Failure::ProtocolInvalid),
                Signal::Other => {
                    // A real transcription before session.updated invalidates
                    // this arm; it must not be scored at a fabricated zero time.
                    if state.observer.metrics.draft_events + state.observer.metrics.final_events > 0 {
                        return Err(Failure::ProtocolInvalid);
                    }
                }
            }
        }
        state.observer.metrics.realtime_stage = Some(Stage::Streaming);
        state.observer.metrics.task_started = 1;
        state.observer.metrics.ready_ms = connecting.elapsed().as_millis() as u64;
        let (mut sink, mut stream) = socket.split();
        let started = Instant::now();
        let overall_deadline = started + Duration::from_millis(clip.pcm.len() as u64 / 32 + 2000 + 30_000);
        let (finish_tx, mut finish_rx) = watch::channel(None::<Instant>);
        let sender = async {
            let zeros = [0u8; FRAME_BYTES];
            let mut schedule = started;
            for (is_tail, bytes) in clip.pcm.chunks(FRAME_BYTES).map(|bytes| (false, bytes))
                .chain((0..TAIL_FRAMES).map(|_| (true, zeros.as_slice()))) {
                tokio::time::sleep_until(schedule).await;
                sent.maximum_send_lateness_ms = sent.maximum_send_lateness_ms.max(Instant::now().saturating_duration_since(schedule).as_millis() as u64);
                let append = LiveTranslateRequestEncoder::audio_append(bytes, None).map_err(|_| Failure::ProtocolInvalid)?;
                tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Text(append.to_string().into())))
                    .await.map_err(|_| Failure::SendTimeout)?.map_err(websocket_failure)?;
                sent.audio_frames += u64::from(!is_tail);
                sent.tail_frames += u64::from(is_tail);
                sent.sent_bytes += bytes.len() as u64;
                schedule += Duration::from_micros(bytes.len() as u64 * 1_000_000 / BYTES_PER_SECOND as u64);
            }
            tokio::time::sleep_until(schedule).await;
            let finish = LiveTranslateRequestEncoder::finish(None).map_err(|_| Failure::ProtocolInvalid)?;
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
                        changed.map_err(|_| Failure::NetworkFailed)?; continue;
                    }
                    message = tokio::time::timeout_at(deadline, stream.next()) => message
                        .map_err(|_| Failure::FinishTimeout)?.ok_or(Failure::NetworkFailed)?.map_err(websocket_failure)?,
                };
                let Some(text) = text_frame(message)? else { continue; };
                if state.record(&text, started.elapsed().as_millis() as u64, finish_rx.borrow().is_some())? == Signal::Finished {
                    if finish_rx.borrow().is_none() { return Err(Failure::ProtocolInvalid); }
                    state.observer.metrics.realtime_stage = Some(Stage::Finished);
                    state.observer.metrics.task_finished = 1;
                    state.observer.metrics.elapsed_ms = started.elapsed().as_millis() as u64;
                    return Ok::<_, Failure>(());
                }
            }
        };
        tokio::time::timeout_at(overall_deadline, async { tokio::try_join!(sender, receiver) })
            .await.map_err(|_| Failure::FinishTimeout)??;
        Ok::<_, Failure>(())
    }.await;
    state.observer.metrics.audio_frames = sent.audio_frames;
    state.observer.metrics.tail_frames = sent.tail_frames;
    state.observer.metrics.sent_bytes = sent.sent_bytes;
    state.observer.metrics.maximum_send_lateness_ms = sent.maximum_send_lateness_ms;
    state
        .observer
        .report(clip, Arm::RealtimeAsr, operation.err())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use base64::Engine;
    use serde_json::json;
    use tokio::net::TcpListener;

    fn updated(language: Language) -> String {
        let mut value = session_update(language);
        value["type"] = "session.updated".into();
        value["session"]["model"] = MODEL.into();
        value.to_string()
    }

    fn draft(id: &str, text: &str, stash: &str) -> String {
        json!({"type":"conversation.item.input_audio_transcription.text", "item_id":id,
            "text":text,"stash":stash,"language":"en"})
        .to_string()
    }

    fn final_result(id: &str, text: &str) -> String {
        json!({"type":"conversation.item.input_audio_transcription.completed", "item_id":id,
            "transcript":text,"language":"en"})
        .to_string()
    }

    fn direct() -> ProviderNetwork {
        ProviderNetwork::resolve(&ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        })
        .unwrap()
    }

    #[test]
    fn request_is_fixed_standalone_asr_and_requires_a_matching_updated_ack() {
        let request = session_update(Language::English);
        assert_eq!(request["type"], "session.update");
        assert_eq!(request["session"]["input_audio_format"], "pcm");
        assert_eq!(request["session"]["sample_rate"], 16000);
        assert_eq!(
            request["session"]["input_audio_transcription"],
            json!({"language":"en"})
        );
        assert_eq!(
            request["session"]["turn_detection"],
            json!({"type":"server_vad","threshold":0.2,"silence_duration_ms":800})
        );
        assert!(request["session"].get("translation").is_none());
        assert!(request["session"].get("instructions").is_none());
        assert!(request["session"]["input_audio_transcription"]
            .get("corpus")
            .is_none());
        assert!(
            session_update(Language::Automatic)["session"]["input_audio_transcription"]
                .get("language")
                .is_none()
        );
        assert_eq!(endpoint(), "wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model=qwen3-asr-flash-realtime-2026-02-10");
        assert_eq!(Arm::parse("realtime-asr").unwrap(), Arm::RealtimeAsr);
        assert!(Arm::RealtimeAsr
            .request(SourceLanguage::English, "synthetic")
            .is_err());
        let mut valid: serde_json::Value =
            serde_json::from_str(&updated(Language::English)).unwrap();
        assert!(acknowledged(&valid, Language::English));
        valid["session"]["input_audio_format"] = "pcm16".into();
        assert!(acknowledged(&valid, Language::English));
        for (pointer, value) in [
            ("/session/input_audio_transcription/language", json!("zh")),
            ("/session/sample_rate", json!(8000)),
            ("/session/model", json!("different-model")),
            ("/session/turn_detection/silence_duration_ms", json!(400)),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(!acknowledged(&invalid, Language::English));
        }
    }

    #[test]
    fn stable_and_stash_counts_and_retractions_are_scoped_to_one_real_item() {
        let clip = synthetic_clip();
        let mut state = RealtimeObserver::new(&clip);
        state
            .record(&draft("private-item-A", "中文", "かな"), 10, false)
            .unwrap();
        state
            .record(&draft("private-item-A", "English", "suffix"), 20, false)
            .unwrap();
        // A new real item is not a retraction of A's stable prefix.
        state
            .record(&draft("private-item-B", "文", "x"), 30, false)
            .unwrap();
        state
            .record(&final_result("private-item-B", "different"), 50, false)
            .unwrap();
        let report = state.observer.report(&clip, Arm::RealtimeAsr, None);
        assert_eq!(report.metrics.first_stable_prefix_ms, Some(10));
        assert_eq!(report.metrics.stable_prefix_events, 3);
        assert_eq!(report.metrics.stable_prefix_han_characters, 3);
        assert_eq!(report.metrics.stable_prefix_latin_characters, 7);
        assert_eq!(report.metrics.stash_kana_characters, 2);
        assert_eq!(report.metrics.stable_prefix_changes, 1);
        assert_eq!(report.metrics.stable_prefix_retractions, 1);
        assert_eq!(report.metrics.stable_prefix_retracted_han_characters, 2);
        assert_eq!(report.metrics.stable_prefix_final_retractions, 1);
        assert_eq!(
            report.metrics.stable_prefix_final_retracted_han_characters,
            1
        );
        assert_eq!(report.metrics.stash_revisions, 1);
        assert_eq!(report.metrics.stash_retracted_characters, 2);
        let output = serde_json::to_string(&report).unwrap();
        for forbidden in [
            "private-item",
            "English",
            "suffix",
            "different",
            "中文",
            "かな",
        ] {
            // The allowed configured Language::English is a fixed label.
            if forbidden != "English" {
                assert!(!output.contains(forbidden));
            }
        }
    }

    #[test]
    fn opaque_ids_keep_real_repeated_finals_and_do_not_use_an_ordered_watermark() {
        let mut clip = synthetic_clip();
        clip.reference = Some(normalize("same same", Unit::Word));
        let mut state = RealtimeObserver::new(&clip);
        state
            .record(&draft("A-private-id", "", "same"), 10, false)
            .unwrap();
        state
            .record(&draft("B-private-id", "", "same"), 20, false)
            .unwrap();
        // B was observed second but finishes first: A must still be evaluated.
        state
            .record(&final_result("B-private-id", "same"), 30, false)
            .unwrap();
        state
            .record(&final_result("A-private-id", "same"), 40, false)
            .unwrap();
        state
            .record(&final_result("B-private-id", "same"), 50, false)
            .unwrap();
        let report = state.observer.report(&clip, Arm::RealtimeAsr, None);
        assert_eq!(report.metrics.final_events, 3);
        assert_eq!(report.metrics.duplicate_final_ids, 1);
        assert_eq!(report.metrics.final_latin_characters, 8);
        assert_eq!(report.evaluation.unwrap().edit_distance, 0);
        assert_eq!(
            report
                .metrics
                .finals
                .iter()
                .map(|sample| sample.sentence_id)
                .collect::<Vec<_>>(),
            vec![Some(2), Some(1), Some(2)]
        );
        assert!(report.metrics.sentence_end_delay_max_ms.is_none());
    }

    #[test]
    fn real_item_clocks_are_kept_but_only_local_stop_to_final_is_subtracted() {
        let clip = synthetic_clip();
        let mut state = RealtimeObserver::new(&clip);
        state.record(&json!({"type":"input_audio_buffer.speech_started","item_id":"A","audio_start_ms":1000}).to_string(), 50, false).unwrap();
        state.record(&json!({"type":"input_audio_buffer.speech_stopped","item_id":"A","audio_end_ms":1200}).to_string(), 100, false).unwrap();
        state
            .record(&final_result("A", "private synthetic token"), 175, true)
            .unwrap();
        state.record(&final_result("B", "more"), 200, true).unwrap();
        let report = state.observer.report(&clip, Arm::RealtimeAsr, None);
        assert_eq!(report.metrics.finals[0].begin_time_ms, Some(1000));
        assert_eq!(report.metrics.finals[0].end_time_ms, Some(1200));
        assert_eq!(report.metrics.finals[0].speech_stop_to_final_ms, Some(75));
        assert_eq!(report.metrics.finals[1].speech_stop_to_final_ms, None);
        assert_eq!(report.metrics.speech_stop_to_final_max_ms, Some(75));
        assert_eq!(report.metrics.sentence_end_delay_max_ms, None);
    }

    #[test]
    fn parser_size_identity_and_failure_outputs_are_bounded_and_content_free() {
        let clip = synthetic_clip();
        let mut state = RealtimeObserver::new(&clip);
        let rejected = json!({"type":"error", "error":{"code":"private-secret-code", "message":"private-body", "param":"private-key"}}).to_string();
        assert!(matches!(
            state.record(&rejected, 0, false),
            Err(Failure::TaskRejected)
        ));
        let report = state
            .observer
            .report(&clip, Arm::RealtimeAsr, Some(Failure::TaskRejected));
        let output = serde_json::to_string(&report).unwrap();
        for forbidden in ["private-secret-code", "private-body", "private-key"] {
            assert!(!output.contains(forbidden));
        }
        assert!(report.evaluation.is_none());
        let mut state = RealtimeObserver::new(&clip);
        assert!(state
            .record(&draft("A", &"a".repeat(MAX_TEXT_BYTES), "b"), 1, false)
            .is_err());
        assert!(state
            .record(&draft(&"A".repeat(MAX_ITEM_ID_BYTES + 1), "", ""), 1, false)
            .is_err());
        assert!(state
            .record(&" ".repeat(MAX_AUDIO3_MESSAGE_BYTES + 1), 1, false)
            .is_err());
        let mut state = RealtimeObserver::new(&clip);
        for index in 0..MAX_FINAL_SAMPLES {
            state
                .record(&draft(&format!("synthetic-{index}"), "", ""), 1, false)
                .unwrap();
        }
        assert!(matches!(
            state.record(&draft("extra", "", ""), 1, false),
            Err(Failure::EventLimit)
        ));
    }

    #[tokio::test]
    async fn local_realtime_socket_waits_for_updated_and_finishes_exact_paced_pcm() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let mut clip = synthetic_clip();
        clip.pcm = vec![7; FRAME_BYTES];
        let expected_pcm = clip.pcm.clone();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let Message::Text(request) = ws.next().await.unwrap().unwrap() else {
                panic!("expected synthetic session update");
            };
            let value: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(value["type"], "session.update");
            assert!(value["session"].get("translation").is_none());
            ws.send(Message::Text(
                json!({"type":"session.created","session":{"model":MODEL}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(40), ws.next())
                .await
                .is_err());
            ws.send(Message::Text(updated(Language::English).into()))
                .await
                .unwrap();
            let mut frames = 0;
            loop {
                let Message::Text(wire) = ws.next().await.unwrap().unwrap() else {
                    panic!("expected synthetic append/finish text frame");
                };
                let value: serde_json::Value = serde_json::from_str(&wire).unwrap();
                if value["type"] == "session.finish" {
                    break;
                }
                assert_eq!(value["type"], "input_audio_buffer.append");
                let pcm = base64::engine::general_purpose::STANDARD
                    .decode(value["audio"].as_str().unwrap())
                    .unwrap();
                if frames == 0 {
                    assert_eq!(pcm, expected_pcm);
                    ws.send(Message::Text(
                        draft("private-synthetic-id", "private synthetic ", "token").into(),
                    ))
                    .await
                    .unwrap();
                } else {
                    assert_eq!(pcm, vec![0; FRAME_BYTES]);
                }
                frames += 1;
            }
            assert_eq!(frames, 1 + TAIL_FRAMES);
            ws.send(Message::Text(
                final_result("private-synthetic-id", "private synthetic token").into(),
            ))
            .await
            .unwrap();
            ws.send(Message::Text(
                json!({"type":"session.finished"}).to_string().into(),
            ))
            .await
            .unwrap();
        });
        let report = tokio::time::timeout(
            Duration::from_secs(5),
            compare(&clip, "synthetic-key", &direct(), &endpoint),
        )
        .await
        .unwrap();
        assert!(report.failure.is_none());
        assert_eq!(report.metrics.task_started, 1);
        assert_eq!(report.metrics.task_finished, 1);
        assert_eq!(report.metrics.audio_frames, 1);
        assert_eq!(report.metrics.tail_frames, TAIL_FRAMES as u64);
        assert_eq!(
            report.metrics.sent_bytes,
            (FRAME_BYTES * (TAIL_FRAMES + 1)) as u64
        );
        assert!(report.metrics.first_stable_prefix_ms.is_some());
        assert_eq!(report.metrics.final_after_finish, 1);
        assert_eq!(report.evaluation.unwrap().edit_distance, 0);
        assert!(report.metrics.sentence_end_delay_max_ms.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn invalid_ready_or_early_finished_sends_no_audio_and_cleans_up_socket() {
        for premature in [
            json!({"type":"session.updated","session":{"input_audio_format":"pcm","input_audio_transcription":{"language":"zh"}}}),
            json!({"type":"session.finished"}),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("ws://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                assert!(matches!(
                    ws.next().await.unwrap().unwrap(),
                    Message::Text(_)
                ));
                ws.send(Message::Text(premature.to_string().into()))
                    .await
                    .unwrap();
                // Dropping the failed operation must close TCP; no append can
                // escape merely because the WS handshake/session.created worked.
                let next = tokio::time::timeout(Duration::from_secs(1), ws.next())
                    .await
                    .unwrap();
                assert!(!matches!(next, Some(Ok(Message::Text(_)))));
            });
            let report = tokio::time::timeout(
                Duration::from_secs(2),
                compare(&synthetic_clip(), "synthetic-key", &direct(), &endpoint),
            )
            .await
            .unwrap();
            assert_eq!(report.failure, Some(Failure::ProtocolInvalid));
            assert_eq!(report.metrics.audio_frames, 0);
            assert!(report.evaluation.is_none());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn maximum_input_is_checked_before_connecting() {
        let mut clip = synthetic_clip();
        for pcm in [vec![], vec![0], vec![0; MAX_PCM_BYTES + 2]] {
            clip.pcm = pcm;
            let report = compare(&clip, "synthetic-key", &direct(), "invalid-endpoint").await;
            assert_eq!(report.failure, Some(Failure::InputInvalid));
            assert_eq!(report.metrics.sent_bytes, 0);
        }
    }
}
