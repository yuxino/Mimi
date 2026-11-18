//! Offline views of shared protocol and preview contracts.
//! These adapters run the same decoders as production without opening a provider.

use super::{json, SourceLanguage, Value, VolcanoEngineEndpoint, VolcanoEngineServerEvent};

// An offline contract view of the shared wire decoder. This never creates a
// provider, session or audio pipeline; protocol rejection is represented by null.
pub(super) fn volcano_decode(value: &Value) -> Result<Value, &'static str> {
    let hex = value["hex"]
        .as_str()
        .filter(|hex| {
            hex.len() <= VolcanoEngineEndpoint::MAXIMUM_SERVER_FRAME_BYTES * 2
                && hex.len().is_multiple_of(2)
        })
        .ok_or("native_runtime_invalid_request")?;
    let frame = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| {
            let pair = std::str::from_utf8(bytes).map_err(|_| "native_runtime_invalid_request")?;
            u8::from_str_radix(pair, 16).map_err(|_| "native_runtime_invalid_request")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let event = VolcanoEngineServerEvent::decode(&frame).ok().map(|event| match event {
        VolcanoEngineServerEvent::SourceSubtitleStarted { start_time_ms } =>
            json!({"type":"sourceStart","startTimeMs":start_time_ms}),
        VolcanoEngineServerEvent::SourceSubtitleDraft(text) =>
            json!({"type":"sourceDraft","text":text}),
        VolcanoEngineServerEvent::SourceSubtitleFinal { text, start_time_ms, end_time_ms } =>
            json!({"type":"sourceFinal","text":text,"startTimeMs":start_time_ms,"endTimeMs":end_time_ms}),
        VolcanoEngineServerEvent::TranslationSubtitleStarted { start_time_ms } =>
            json!({"type":"translationStart","startTimeMs":start_time_ms}),
        VolcanoEngineServerEvent::TranslationSubtitleDraft(text) =>
            json!({"type":"translationDraft","text":text}),
        VolcanoEngineServerEvent::TranslationSubtitleFinal { text, start_time_ms, end_time_ms } =>
            json!({"type":"translationFinal","text":text,"startTimeMs":start_time_ms,"endTimeMs":end_time_ms}),
        VolcanoEngineServerEvent::SessionStarted => json!({"type":"sessionStarted"}),
        VolcanoEngineServerEvent::SessionFinished => json!({"type":"sessionFinished"}),
        VolcanoEngineServerEvent::SessionFailed { status_code } =>
            json!({"type":"sessionFailed","statusCode":status_code}),
        VolcanoEngineServerEvent::Ignored { event } => json!({"type":"ignored","event":event}),
    });
    Ok(json!({"event":event}))
}

// Offline contract access to the same bounded content comparison as desktop.
// Return no draft text, fingerprint, provider work or session state.
pub(super) fn preview_content_changed(value: &Value) -> Result<bool, &'static str> {
    fn language(value: &Value) -> Result<Option<&str>, &'static str> {
        match value {
            Value::Null => Ok(None),
            Value::String(language)
                if SourceLanguage::ALL.into_iter().any(|source| {
                    source != SourceLanguage::Automatic && source.raw_value() == language
                }) =>
            {
                Ok(Some(language))
            }
            _ => Err("native_runtime_invalid_request"),
        }
    }
    let text = |field: &str| {
        value[field]
            .as_str()
            .filter(|text| mimi_runtime::core::models::subtitle_text_within_limit(text))
            .ok_or("native_runtime_invalid_request")
    };
    let current = mimi_runtime::core::subtitles::preview_pacing::PreviewCandidate::new(
        text("draft")?,
        language(
            value
                .get("language")
                .ok_or("native_runtime_invalid_request")?,
        )?,
        0,
    );
    let previous = mimi_runtime::core::subtitles::preview_pacing::PreviewCandidate::new(
        text("previousDraft")?,
        language(
            value
                .get("previousLanguage")
                .ok_or("native_runtime_invalid_request")?,
        )?,
        0,
    );
    Ok(!current.matches_content(previous))
}

pub(super) fn qwen_streaming_text(value: &Value) -> Result<Value, &'static str> {
    use mimi_runtime::core::protocols::translation::qwen_mt::{QwenMTModel, QwenMTStreamText};
    let model: QwenMTModel = serde_json::from_value(value["model"].clone())
        .map_err(|_| "native_runtime_invalid_request")?;
    let contents = value["contents"]
        .as_array()
        .filter(|contents| contents.len() <= 32)
        .ok_or("native_runtime_invalid_request")?;
    let mut text = QwenMTStreamText::new(model);
    let mut outputs = Vec::with_capacity(contents.len());
    for content in contents {
        let changed = text
            .apply(content.as_str().ok_or("native_runtime_invalid_request")?)
            .map_err(|_| "native_runtime_invalid_request")?;
        outputs.push(json!({"text":text.text(),"changed":changed}));
    }
    Ok(json!({"outputs":outputs}))
}

pub(super) fn gemini_streaming_transcripts(value: &Value) -> Result<Value, &'static str> {
    let frames = value["frames"]
        .as_array()
        .filter(|frames| frames.len() <= 256)
        .ok_or("native_runtime_invalid_request")?;
    let mut assembler =
        mimi_runtime::core::protocols::recognition::gemini_transcribe::TranscriptAssembler::default(
        );
    let mut outputs = Vec::with_capacity(frames.len());
    for frame in frames {
        let event = mimi_runtime::core::protocols::recognition::gemini_transcribe::decode(
            &frame.to_string(),
        )
        .map_err(|_| "native_runtime_invalid_request")?;
        let output = assembler
            .apply(event)
            .map_err(|_| "native_runtime_invalid_request")?;
        outputs.push(output.into_iter().map(|output| json!({"sequence":output.sequence,"text":output.text,"isFinal":output.is_final})).collect::<Vec<_>>());
    }
    Ok(json!({"outputs":outputs}))
}

pub(super) fn xai_streaming_transcripts(value: &Value) -> Result<Value, &'static str> {
    let frames = value["frames"]
        .as_array()
        .filter(|frames| frames.len() <= 256)
        .ok_or("native_runtime_invalid_request")?;
    let mut assembler =
        mimi_runtime::core::protocols::recognition::xai_stt::TranscriptAssembler::default();
    let mut outputs = Vec::with_capacity(frames.len());
    for frame in frames {
        let event = mimi_runtime::core::protocols::recognition::xai_stt::decode(&frame.to_string())
            .map_err(|_| "native_runtime_invalid_request")?;
        let output = assembler
            .apply(event)
            .map_err(|_| "native_runtime_invalid_request")?;
        outputs.push(output.map(|output| json!({"sequence":output.sequence,"text":output.text,"isFinal":output.is_final})));
    }
    Ok(json!({"outputs":outputs}))
}

pub(super) fn preview_candidate(value: &Value) -> Result<Value, &'static str> {
    let minimum = value["minimumCharacters"]
        .as_u64()
        .filter(|n| (1..=65536).contains(n))
        .ok_or("native_runtime_invalid_request")?;
    let draft = value["draft"]
        .as_str()
        .filter(|text| mimi_runtime::core::models::subtitle_text_within_limit(text))
        .ok_or("native_runtime_invalid_request")?;
    let opening = value["opening"]
        .as_bool()
        .ok_or("native_runtime_invalid_request")?;
    let mut committer =
        mimi_runtime::core::subtitles::committer::ASRDraftCommitter::new(minimum as usize);
    committer.update_draft(draft);
    Ok(json!({"candidate":committer.preview_latest_draft(opening)}))
}
