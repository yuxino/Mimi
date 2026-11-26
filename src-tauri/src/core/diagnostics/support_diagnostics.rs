//! A fixed, content-free support snapshot. Never accept logs, endpoint URLs,
//! profile/device names, paths, credentials, or subtitle content as inputs.
use crate::core::diagnostics::latency::{TranslationLatencyKind, TranslationRecovery};
use crate::core::models::TranslationMode;
use crate::core::services::provider::{AlibabaRealtimeMode, ProviderKind};
use serde::Serialize;
use std::collections::VecDeque;

pub const RECENT_EVENT_LIMIT: usize = 24;
pub const REPORT_BYTE_LIMIT: usize = 16 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportIssue {
    pub url: String,
    pub report: String,
    pub requires_paste: bool,
}

/// Fixed destination and typed facts only; never accept a frontend URL/body.
pub fn issue_link(facts: DiagnosticFacts) -> SupportIssue {
    let report = render(facts);
    let body = format!("## What happened?\nPlease describe what you expected and what happened. Review this public report before submitting.\n\n## Safe diagnostics\n```json\n{report}\n```\n\nRefs #74, #75\n");
    let mut url = url::Url::parse("https://github.com/yuxino/mimi/issues/new").unwrap();
    url.query_pairs_mut()
        .append_pair("title", "Mimi troubleshooting report")
        .append_pair("body", &body);
    let requires_paste = url.as_str().len() > 6000;
    if requires_paste {
        url.set_query(None);
        url.query_pairs_mut().append_pair("title", "Mimi troubleshooting report").append_pair("body", "## What happened?\nDescribe the problem.\n\n## Safe diagnostics\nPaste the diagnostic snapshot copied from Mimi here. Review before submitting.\n\nRefs #74, #75\n");
    }
    SupportIssue {
        url: url.into(),
        report,
        requires_paste,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    Retrying,
    Recovered,
    RetriesExhausted,
    UserStopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SafeFailure {
    phase: &'static str,
    category: &'static str,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_status_code: Option<i64>,
}

impl SafeFailure {
    pub fn from_error(error: &str) -> Self {
        let unknown = Self {
            phase: "unknown",
            category: "unknown",
            code: "OTHER",
            provider_status_code: None,
        };
        if error.len() > 128 {
            return unknown;
        }
        if let Some(failure) = Self::from_volcano_error(error) {
            return failure;
        }
        if let Some(failure) = Self::from_apple_speech_error(error) {
            return failure;
        }
        if let Some(failure) = Self::from_alibaba_connection_error(error) {
            return failure;
        }
        if let Some(failure) = Self::from_baidu_or_tencent_error(error) {
            return failure;
        }
        if let Some(failure) = Self::from_deeplx_error(error) {
            return failure;
        }
        let translation_failure = match error {
            "translation_rate_limited" => Some(("rate_limit", "TRANSLATION_RATE_LIMITED")),
            "translation_temporarily_unavailable" => {
                Some(("service_error", "TRANSLATION_TEMPORARILY_UNAVAILABLE"))
            }
            "translation_backlog_overflow"
            | "Translation fell behind live audio. mimi is reconnecting." => {
                Some(("backlog", "TRANSLATION_BACKLOG_OVERFLOW"))
            }
            _ => None,
        };
        if let Some((category, code)) = translation_failure {
            return Self {
                phase: "text_translation",
                category,
                code,
                provider_status_code: None,
            };
        }
        let runtime_failure = match error {
            "openai_realtime_session_expired" | "openai_realtime_setup_expired" => Some((
                "websocket_connection",
                "timeout",
                "OPENAI_REALTIME_SESSION_EXPIRED",
            )),
            "openai_realtime_permission_denied" => Some((
                "websocket_connection",
                "configuration",
                "OPENAI_REALTIME_PERMISSION_DENIED",
            )),
            "openai_realtime_quota_exhausted" => Some((
                "websocket_connection",
                "configuration",
                "OPENAI_REALTIME_QUOTA_EXHAUSTED",
            )),
            "openai_realtime_rate_limited" => Some((
                "websocket_connection",
                "rate_limit",
                "OPENAI_REALTIME_RATE_LIMITED",
            )),
            "openai_realtime_model_unavailable" => Some((
                "websocket_connection",
                "configuration",
                "OPENAI_REALTIME_MODEL_UNAVAILABLE",
            )),
            "openai_realtime_invalid_request" => Some((
                "websocket_connection",
                "request_rejected",
                "OPENAI_REALTIME_INVALID_REQUEST",
            )),
            "openai_realtime_service_error" | "openai_realtime_service_unavailable" => Some((
                "websocket_connection",
                "service_error",
                "OPENAI_REALTIME_SERVICE_ERROR",
            )),
            "gemini_live_service_unavailable" => Some((
                "websocket_connection",
                "service_error",
                "GEMINI_LIVE_SERVICE_UNAVAILABLE",
            )),
            "openai_realtime_provider_error"
            | "xai_realtime_provider_error"
            | "gemini_live_provider_error"
            | "baidu_realtime_provider_error"
            | "audio3_provider_error" => Some((
                "websocket_connection",
                "request_rejected",
                "PROVIDER_REQUEST_REJECTED",
            )),
            "alibaba_realtime_idle_timeout"
            | "The Alibaba realtime session timed out while idle." => Some((
                "websocket_connection",
                "timeout",
                "ALIBABA_REALTIME_IDLE_TIMEOUT",
            )),
            "alibaba_realtime_service_unavailable"
            | "The Alibaba realtime service is temporarily unavailable." => Some((
                "websocket_connection",
                "service_error",
                "ALIBABA_REALTIME_SERVICE_UNAVAILABLE",
            )),
            "alibaba_realtime_provider_error"
            | "The Alibaba realtime service rejected the request." => Some((
                "websocket_connection",
                "request_rejected",
                "ALIBABA_REALTIME_PROVIDER_ERROR",
            )),
            "alibaba_realtime_response_failed" | "The Alibaba realtime response failed." => Some((
                "text_translation",
                "service_error",
                "ALIBABA_REALTIME_RESPONSE_FAILED",
            )),
            "live_translate_unexpected_session_end"
            | "The live translation session ended unexpectedly." => Some((
                "websocket_connection",
                "service_error",
                "LIVE_TRANSLATE_UNEXPECTED_SESSION_END",
            )),
            "subtitle_text_too_large" => {
                Some(("subtitles", "size_limit", "SUBTITLE_TEXT_TOO_LARGE"))
            }
            "transport_error" => Some(("websocket_connection", "transport", "TRANSPORT_ERROR")),
            "baidu_unexpected_session_end"
            | "Baidu realtime translation ended the session unexpectedly." => Some((
                "websocket_connection",
                "service_error",
                "BAIDU_UNEXPECTED_SESSION_END",
            )),
            "provider_event_backlog_overflow" => Some((
                "provider_events",
                "backlog",
                "PROVIDER_EVENT_BACKLOG_OVERFLOW",
            )),
            "System audio capture permission was denied." | "capture.permission_denied" => Some((
                "capture_setup",
                "permission",
                "SYSTEM_AUDIO_PERMISSION_DENIED",
            )),
            "System audio capture was stopped by the user." | "capture.user_stopped" => {
                Some(("capture", "user_stopped", "CAPTURE_USER_STOPPED"))
            }
            "Microphone capture permission was denied." => Some((
                "capture_setup",
                "permission",
                "MICROPHONE_PERMISSION_DENIED",
            )),
            "No default microphone is available." => Some((
                "capture_setup",
                "device_unavailable",
                "MICROPHONE_UNAVAILABLE",
            )),
            "Microphone capture could not be started." => {
                Some(("capture_setup", "start", "MICROPHONE_START_FAILED"))
            }
            "capture.native_stopped" => Some(("capture", "stopped", "CAPTURE_STOPPED")),
            "capture.audio_processing_failed" => {
                Some(("capture", "processing", "AUDIO_PROCESSING_FAILED"))
            }
            "capture.backpressure" => Some(("capture", "backlog", "CAPTURE_BACKPRESSURE")),
            "audio_pipeline.transport_stopped" => {
                Some(("audio_send", "transport", "AUDIO_TRANSPORT_STOPPED"))
            }
            _ => None,
        };
        if let Some((phase, category, code)) = runtime_failure {
            return Self {
                phase,
                category,
                code,
                provider_status_code: None,
            };
        }
        if error == "credential_authentication_failed" {
            return Self {
                phase: "unknown",
                category: "authentication",
                code: "AUTHENTICATION_FAILED",
                provider_status_code: None,
            };
        }
        let storage_code = match error {
            "credential_store_unavailable" => Some("CREDENTIAL_STORE_UNAVAILABLE"),
            "credential_service_unavailable" => Some("CREDENTIAL_SERVICE_UNAVAILABLE"),
            "credential_store_access_denied" => Some("CREDENTIAL_STORE_ACCESS_DENIED"),
            _ => None,
        };
        if let Some(code) = storage_code {
            return Self {
                phase: "setup",
                category: "credential_storage",
                code,
                provider_status_code: None,
            };
        }
        if error == "The selected sound output is unavailable. Stop subtitles and choose another sound source in Settings." { return Self { phase: "capture_setup", category: "device_unavailable", code: "OUTPUT_UNAVAILABLE", provider_status_code: None }; }
        let mut parts = error.split('.');
        if parts.next() != Some("audio3_error") {
            return unknown;
        }
        let phase = match parts.next() {
            Some("setup") => "asr_setup",
            Some("recognition") => "asr_recognition",
            Some("connection") => "websocket_connection",
            _ => return unknown,
        };
        let category = match parts.next() {
            Some("timeout" | "local_timeout") => "timeout",
            Some("local_overload") => "backlog",
            Some("authentication") => "authentication",
            Some("request") => "request_rejected",
            Some("unsupported_language") => "configuration",
            Some("service") => "service_error",
            Some("rate_limit") => "rate_limit",
            Some("task_failed") => "unknown",
            _ => return unknown,
        };
        let code = match parts.next() {
            Some("CLIENT_ERROR") => "CLIENT_ERROR",
            Some("UNSUPPORTED_LANGUAGE") => "UNSUPPORTED_LANGUAGE",
            Some("LOCAL_ASR_OVERLOADED") => "LOCAL_ASR_OVERLOADED",
            Some("LOCAL_ASR_TIMEOUT") => "LOCAL_ASR_TIMEOUT",
            Some("SERVER_ERROR") => "SERVER_ERROR",
            Some("INVALID_API_KEY") => "INVALID_API_KEY",
            Some("UNAUTHORIZED") => "UNAUTHORIZED",
            Some("REQUEST_TIMEOUT") => "REQUEST_TIMEOUT",
            Some("THROTTLED") => "THROTTLED",
            Some("LOCAL_TIMEOUT") => "LOCAL_TIMEOUT",
            Some("HTTP_AUTH") => "HTTP_AUTH",
            Some("OTHER") => "OTHER",
            _ => return unknown,
        };
        if parts.next().is_some() {
            return unknown;
        }
        Self {
            phase,
            category,
            code,
            provider_status_code: None,
        }
    }

    /// Exact client-owned messages only: never copy arbitrary transport details
    /// or infer a network/provider root cause from an unanswered health probe.
    fn from_alibaba_connection_error(error: &str) -> Option<Self> {
        let (phase, category, code) = match error {
            "The live translation session is not connected." => (
                "websocket_connection",
                "transport",
                "LIVE_TRANSLATE_NOT_CONNECTED",
            ),
            "The live translation connection stopped responding." => (
                "websocket_connection",
                "timeout",
                "LIVE_TRANSLATE_HEALTH_TIMEOUT",
            ),
            "The live translation connection could not be established in time." => (
                "websocket_connection",
                "timeout",
                "LIVE_TRANSLATE_CONNECTION_TIMEOUT",
            ),
            "The live translation session setup timed out." => {
                ("setup", "timeout", "LIVE_TRANSLATE_SETUP_TIMEOUT")
            }
            "The live translation session setup was rejected." => {
                ("setup", "request_rejected", "LIVE_TRANSLATE_SETUP_REJECTED")
            }
            "The live translation transport failed."
            | "The live translation connection closed." => (
                "websocket_connection",
                "transport",
                "LIVE_TRANSLATE_TRANSPORT_ERROR",
            ),
            "The speech recognition session is not connected." => {
                ("websocket_connection", "transport", "ASR_NOT_CONNECTED")
            }
            "The speech recognition connection stopped responding." => {
                ("websocket_connection", "timeout", "ASR_HEALTH_TIMEOUT")
            }
            "The speech recognition connection could not be established in time." => {
                ("websocket_connection", "timeout", "ASR_CONNECTION_TIMEOUT")
            }
            "The speech recognition transport failed."
            | "The speech recognition connection closed." => {
                ("websocket_connection", "transport", "ASR_TRANSPORT_ERROR")
            }
            _ => return None,
        };
        Some(Self {
            phase,
            category,
            code,
            provider_status_code: None,
        })
    }

    /// Preserve the observed Apple recognition boundary without retaining
    /// native descriptions or guessing the cause of an unclassified failure.
    fn from_apple_speech_error(error: &str) -> Option<Self> {
        let (phase, category, code) = match error {
            "apple_speech_recognition_failed" => (
                "asr_recognition",
                "recognition",
                "APPLE_SPEECH_RECOGNITION_FAILED",
            ),
            "apple_speech_runtime_service_unavailable" => (
                "asr_recognition",
                "service_error",
                "APPLE_SPEECH_RUNTIME_SERVICE_UNAVAILABLE",
            ),
            "apple_speech_runtime_timeout" => {
                ("asr_recognition", "timeout", "APPLE_SPEECH_RUNTIME_TIMEOUT")
            }
            "apple_speech_stream_closed" => (
                "asr_recognition",
                "recognition",
                "APPLE_SPEECH_STREAM_CLOSED",
            ),
            "apple_speech_result_backlog" => {
                ("asr_recognition", "backlog", "APPLE_SPEECH_RESULT_BACKLOG")
            }
            "apple_speech_invalid_result" => (
                "asr_recognition",
                "recognition",
                "APPLE_SPEECH_INVALID_RESULT",
            ),
            "apple_speech_audio_failed" => {
                ("audio_send", "recognition", "APPLE_SPEECH_AUDIO_FAILED")
            }
            "apple_speech_finalize_timeout" => {
                ("session_close", "timeout", "APPLE_SPEECH_FINALIZE_TIMEOUT")
            }
            "apple_speech_service_unavailable" => (
                "asr_setup",
                "service_error",
                "APPLE_SPEECH_SERVICE_UNAVAILABLE",
            ),
            "apple_speech_setup_timeout" => ("asr_setup", "timeout", "APPLE_SPEECH_SETUP_TIMEOUT"),
            "apple_speech_start_failed" => {
                ("asr_setup", "recognition", "APPLE_SPEECH_START_FAILED")
            }
            _ => return None,
        };
        Some(Self {
            phase,
            category,
            code,
            provider_status_code: None,
        })
    }

    /// Only locally emitted labels and canonical nonnegative protobuf int32 codes
    /// are accepted. Never retain the provider's free-form response message.
    fn from_volcano_error(error: &str) -> Option<Self> {
        if let Some(status) = error.strip_prefix("volcano_provider_error.") {
            if status.is_empty()
                || status.len() > 10
                || !status.bytes().all(|byte| byte.is_ascii_digit())
                || (status.len() > 1 && status.starts_with('0'))
            {
                return None;
            }
            let status = status.parse::<u32>().ok()?;
            if status > i32::MAX as u32 {
                return None;
            }
            return Some(Self {
                phase: "websocket_connection",
                category: "service_error",
                code: "VOLCANO_PROVIDER_ERROR",
                provider_status_code: Some(i64::from(status)),
            });
        }
        let (phase, category, code) = match error {
            "volcano_provider_error" | "Volcano Engine rejected the translation session." => (
                "websocket_connection",
                "service_error",
                "VOLCANO_PROVIDER_ERROR",
            ),
            "volcano_unexpected_session_finished"
            | "Volcano Engine ended the translation session unexpectedly." => (
                "websocket_connection",
                "service_error",
                "VOLCANO_UNEXPECTED_SESSION_FINISHED",
            ),
            "volcano_protocol_error" | "Volcano Engine returned an invalid response." => (
                "websocket_connection",
                "service_error",
                "VOLCANO_PROTOCOL_ERROR",
            ),
            "volcano_session_close_failed" => {
                ("session_close", "transport", "VOLCANO_SESSION_CLOSE_FAILED")
            }
            "volcano_close_timeout" | "Volcano Engine did not finish closing in time." => {
                ("session_close", "timeout", "VOLCANO_CLOSE_TIMEOUT")
            }
            "The Volcano Engine connection failed." | "The Volcano Engine transport failed." => (
                "websocket_connection",
                "transport",
                "VOLCANO_TRANSPORT_ERROR",
            ),
            "The Volcano Engine translation session is not connected." => {
                ("websocket_connection", "transport", "VOLCANO_NOT_CONNECTED")
            }
            "The Volcano Engine connection stopped responding." => {
                ("websocket_connection", "timeout", "VOLCANO_HEALTH_TIMEOUT")
            }
            "The Volcano Engine connection could not be established in time." => (
                "websocket_connection",
                "timeout",
                "VOLCANO_CONNECTION_TIMEOUT",
            ),
            "Volcano Engine did not confirm the session configuration in time." => {
                ("setup", "timeout", "VOLCANO_SETUP_TIMEOUT")
            }
            "Volcano Engine rejected the session configuration." => {
                ("setup", "request_rejected", "VOLCANO_SETUP_REJECTED")
            }
            _ => return None,
        };
        Some(Self {
            phase,
            category,
            code,
            provider_status_code: None,
        })
    }

    /// These clients format typed i64 codes; accept precisely that formatting,
    /// never a prefix plus arbitrary provider text or a noncanonical number.
    fn canonical_status_code(value: &str) -> Option<i64> {
        if value.len() > 20 {
            return None;
        }
        let status = value.parse::<i64>().ok()?;
        (status.to_string() == value).then_some(status)
    }

    fn from_baidu_or_tencent_error(error: &str) -> Option<Self> {
        for (prefix, code) in [
            ("baidu_provider_", "BAIDU_PROVIDER_ERROR"),
            ("tencent_provider_", "TENCENT_PROVIDER_ERROR"),
        ] {
            if let Some(value) = error.strip_prefix(prefix) {
                if let Some(status) = Self::canonical_status_code(value) {
                    return Some(Self {
                        phase: "websocket_connection",
                        category: "request_rejected",
                        code,
                        provider_status_code: Some(status),
                    });
                }
            }
        }
        let (category, code) = match error {
            "baidu_protocol_error" | "Baidu realtime translation returned an invalid response." => {
                ("protocol", "BAIDU_PROTOCOL_ERROR")
            }
            "tencent_configuration_rejected" => ("configuration", "TENCENT_CONFIGURATION_REJECTED"),
            "tencent_service_activation_required" => {
                ("configuration", "TENCENT_SERVICE_ACTIVATION_REQUIRED")
            }
            "tencent_quota_exhausted" => ("quota", "TENCENT_QUOTA_EXHAUSTED"),
            "tencent_capacity_exceeded" => ("rate_limit", "TENCENT_CAPACITY_EXCEEDED"),
            "tencent_provider_rejected" => ("request_rejected", "TENCENT_PROVIDER_REJECTED"),
            _ => return None,
        };
        Some(Self {
            phase: "websocket_connection",
            category,
            code,
            // A fixed Display can represent more than one numeric status.
            provider_status_code: None,
        })
    }

    fn from_deeplx_error(error: &str) -> Option<Self> {
        let rejected = error
            .strip_prefix("DeepLX rejected the request (code ")
            .and_then(|value| value.strip_suffix("). Check the endpoint and optional Bearer token with your server administrator."))
            .or_else(|| error.strip_prefix("deeplx.rejected(code=").and_then(|value| value.strip_suffix(')')));
        if let Some(value) = rejected {
            let status = u16::try_from(Self::canonical_status_code(value)?).ok()?;
            return Some(Self {
                phase: "text_translation",
                category: "request_rejected",
                code: "DEEPLX_REQUEST_REJECTED",
                provider_status_code: Some(i64::from(status)),
            });
        }
        let (category, code) = match error {
            "deeplx.endpoint"
            | "Check the DeepLX endpoint in Settings. Use HTTPS, or HTTP on localhost, without URL credentials, query or fragment." => {
                ("configuration", "DEEPLX_ENDPOINT_INVALID")
            }
            "deeplx.timeout"
            | "DeepLX timed out. Check the endpoint, network and server, then restart subtitles." => {
                ("timeout", "DEEPLX_TIMEOUT")
            }
            "deeplx.connection"
            | "Could not connect to DeepLX. Check the endpoint, network and server, then restart subtitles." => {
                ("transport", "DEEPLX_CONNECTION_FAILED")
            }
            "deeplx.response"
            | "DeepLX returned an invalid or empty translation. Check that the endpoint supports the DeepLX /translate JSON API." => {
                ("protocol", "DEEPLX_INVALID_RESPONSE")
            }
            "deeplx.too_large"
            | "DeepLX returned too much data. Check the server's /translate response." => {
                ("size_limit", "DEEPLX_RESPONSE_TOO_LARGE")
            }
            _ => return None,
        };
        Some(Self {
            phase: "text_translation",
            category,
            code,
            provider_status_code: None,
        })
    }

    /// Known local transport/MT labels win over arbitrary provider messages.
    pub fn from_provider_error(code: &str, message: &str) -> Self {
        if let Some(failure) = Self::from_apple_speech_error(code) {
            return failure;
        }
        if let Some(failure) = Self::from_volcano_error(code) {
            return failure;
        }
        if let Some(failure) = Self::from_baidu_or_tencent_error(code) {
            return failure;
        }
        if let Some(failure) = Self::from_deeplx_error(code) {
            return failure;
        }
        match code {
            "credential_authentication_failed"
            | "openai_realtime_session_expired"
            | "openai_realtime_permission_denied"
            | "openai_realtime_quota_exhausted"
            | "openai_realtime_rate_limited"
            | "openai_realtime_model_unavailable"
            | "openai_realtime_invalid_request"
            | "openai_realtime_service_error"
            | "openai_realtime_service_unavailable"
            | "openai_realtime_setup_expired"
            | "translation_rate_limited"
            | "translation_temporarily_unavailable"
            | "translation_backlog_overflow"
            | "provider_event_backlog_overflow"
            | "subtitle_text_too_large"
            | "transport_error"
            | "live_translate_unexpected_session_end"
            | "alibaba_realtime_service_unavailable"
            | "alibaba_realtime_idle_timeout"
            | "alibaba_realtime_provider_error"
            | "alibaba_realtime_response_failed"
            | "gemini_live_service_unavailable"
            | "openai_realtime_provider_error"
            | "xai_realtime_provider_error"
            | "gemini_live_provider_error"
            | "baidu_realtime_provider_error"
            | "audio3_provider_error"
            | "baidu_unexpected_session_end" => Self::from_error(code),
            _ => Self::from_error(message),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CaptureObservation {
    pub pcm_data_recent: bool,
    pub sound_recent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStatus {
    Idle,
    Connecting,
    Stopping,
    Listening,
    Paused,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    StartRequested,
    StartBusy,
    StartAlreadyActive,
    StartSuperseded,
    StopRequested,
    PauseRequested,
    ResumeRequested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticEvent {
    Lifecycle {
        action: LifecycleAction,
    },
    Status {
        status: DiagnosticStatus,
    },
    Failure {
        classification: SafeFailure,
        #[serde(skip_serializing_if = "Option::is_none")]
        audio_source: Option<crate::core::audio::audio_input::AudioSource>,
    },
    Recovery {
        action: RecoveryAction,
    },
    TranslationBackoff {
        recovery: TranslationRecovery,
    },
}

#[derive(Clone, Copy)]
pub enum TextEventKind {
    SourceDraft,
    SourceFinal,
    TranslationDraft,
    TranslationFinal,
    ConfirmedPair,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TextEventCounts {
    source_draft_events: u64,
    source_final_events: u64,
    translation_draft_events: u64,
    translation_final_events: u64,
    confirmed_pair_events: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticEntry {
    sequence: u64,
    elapsed_since_app_start_ms: u64,
    #[serde(flatten)]
    event: DiagnosticEvent,
}

#[derive(Clone, Default, Serialize)]
pub struct DiagnosticJournalSnapshot {
    events_observed: u64,
    events_omitted: u64,
    text_event_counts: TextEventCounts,
    recent_events: Vec<DiagnosticEntry>,
}

/// In-memory fixed facts only, independent of transcript/history preferences.
/// Draft churn increments counters without evicting useful state transitions.
#[derive(Default)]
pub struct DiagnosticJournal {
    events: VecDeque<DiagnosticEntry>,
    sequence: u64,
    last_elapsed_ms: u64,
    last_status: Option<DiagnosticStatus>,
    counts: TextEventCounts,
}

impl DiagnosticJournal {
    pub fn record(&mut self, event: DiagnosticEvent, elapsed_ms: u64) {
        self.last_elapsed_ms = self.last_elapsed_ms.max(elapsed_ms);
        self.sequence = self.sequence.saturating_add(1);
        if self.events.len() == RECENT_EVENT_LIMIT {
            self.events.pop_front();
        }
        self.events.push_back(DiagnosticEntry {
            sequence: self.sequence,
            elapsed_since_app_start_ms: self.last_elapsed_ms,
            event,
        });
    }

    pub fn observe_status(&mut self, status: DiagnosticStatus, elapsed_ms: u64) {
        if self.last_status == Some(status) {
            return;
        }
        self.last_status = Some(status);
        self.record(DiagnosticEvent::Status { status }, elapsed_ms);
    }

    pub fn observe_text(&mut self, kind: TextEventKind) {
        let count = match kind {
            TextEventKind::SourceDraft => &mut self.counts.source_draft_events,
            TextEventKind::SourceFinal => &mut self.counts.source_final_events,
            TextEventKind::TranslationDraft => &mut self.counts.translation_draft_events,
            TextEventKind::TranslationFinal => &mut self.counts.translation_final_events,
            TextEventKind::ConfirmedPair => &mut self.counts.confirmed_pair_events,
        };
        *count = count.saturating_add(1);
    }

    pub fn snapshot(&self) -> DiagnosticJournalSnapshot {
        DiagnosticJournalSnapshot {
            events_observed: self.sequence,
            events_omitted: self.sequence.saturating_sub(self.events.len() as u64),
            text_event_counts: self.counts,
            recent_events: self.events.iter().copied().collect(),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputSelection {
    #[cfg(any(target_os = "windows", test))]
    SystemDefault,
    #[cfg(any(target_os = "windows", test))]
    ManualOutput,
    PlatformSystemAudio,
    SelectedApplication,
    DefaultMicrophone,
    SystemAndMicrophone,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    #[cfg(any(target_os = "windows", test))]
    Available,
    #[cfg(any(target_os = "windows", test))]
    Unavailable,
    Unknown,
}

pub struct DiagnosticFacts {
    pub provider: Option<ProviderKind>,
    pub mode: TranslationMode,
    pub alibaba_realtime_mode: Option<AlibabaRealtimeMode>,
    pub status: DiagnosticStatus,
    pub output_selection: OutputSelection,
    pub output_availability: Availability,
    pub capture: Option<(CaptureObservation, u64)>,
    pub translation_pending: bool,
    pub translation_preview_pending: bool,
    pub translation_timed_out: bool,
    pub last_error: Option<(SafeFailure, u64)>,
    pub recovery: Option<(RecoveryAction, u64)>,
    pub api_latency_ms: Option<u64>,
    pub translation_latency_ms: Option<u64>,
    pub translation_latency_kind: Option<TranslationLatencyKind>,
    pub translation_recovery: Option<TranslationRecovery>,
    pub journal: DiagnosticJournalSnapshot,
    pub elapsed_ms: u64,
}

impl Default for DiagnosticFacts {
    fn default() -> Self {
        Self {
            provider: None,
            mode: TranslationMode::LowLatency,
            alibaba_realtime_mode: None,
            status: DiagnosticStatus::Idle,
            output_selection: OutputSelection::PlatformSystemAudio,
            output_availability: Availability::Unknown,
            capture: None,
            translation_pending: false,
            translation_preview_pending: false,
            translation_timed_out: false,
            last_error: None,
            recovery: None,
            api_latency_ms: None,
            translation_latency_ms: None,
            translation_latency_kind: None,
            translation_recovery: None,
            journal: Default::default(),
            elapsed_ms: 0,
        }
    }
}

pub fn render(facts: DiagnosticFacts) -> String {
    let mut report = serde_json::json!({
        "schema": "mimi.support.v2",
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": { "family": std::env::consts::OS, "architecture": std::env::consts::ARCH, "version": "unknown" },
        "snapshot": { "elapsed_since_app_start_ms": facts.elapsed_ms, "consistency": "best_effort_observations", "audio_recent_window_ms": 2000 },
        "provider": facts.provider,
        "credentials": { "state": "unknown", "authentication": "unknown" },
        "mode": facts.mode,
        "alibaba_realtime_mode": facts.alibaba_realtime_mode,
        "session_status": facts.status,
        "capture": { "output_selection": facts.output_selection, "output_availability": facts.output_availability,
            "observation": facts.capture.map(|(observation, age)| serde_json::json!({ "pcm_data_recent": observation.pcm_data_recent, "sound_recent": observation.sound_recent, "observation_age_ms": age })) },
        "service": { "websocket_stage": "unknown", "asr_stage": "unknown", "last_failure_stage": facts.last_error.map(|(error, _)| error.phase),
            "translation_pending": facts.translation_pending, "translation_preview_pending": facts.translation_preview_pending,
            "translation_timed_out": facts.translation_timed_out,
            "api_round_trip_ms": facts.api_latency_ms, "translation_duration_ms": facts.translation_latency_ms,
            "translation_duration_kind": facts.translation_latency_kind, "translation_recovery": facts.translation_recovery },
        "journal": facts.journal,
        "last_error": facts.last_error.map(|(error, age)| serde_json::json!({ "classification": error, "observed_ago_ms": age })),
        "recent_recovery": facts.recovery.map(|(action, age)| serde_json::json!({ "action": action, "observed_ago_ms": age })),
        "unknown": ["root_cause", "credential_state", "authentication", "app_output_route", "device_display_name", "os_version", "live_service_internals"],
        "interpretation": "Observations do not prove incorrect device selection, speech presence, or the cause of a provider failure. Null means not observed.",
        "excluded": ["credentials", "tokens", "audio", "subtitle_text", "endpoint_urls", "usernames", "paths", "raw_provider_responses", "raw_logs"]
    });
    loop {
        let rendered =
            serde_json::to_string_pretty(&report).expect("fixed diagnostic schema serializes");
        if rendered.len() <= REPORT_BYTE_LIMIT {
            return rendered;
        }
        // Keep JSON valid if future safe fields grow: remove only older facts.
        let events = report["journal"]["recent_events"].as_array_mut().unwrap();
        assert!(
            !events.is_empty(),
            "fixed diagnostic fields fit the report bound"
        );
        events.remove(0);
        let omitted = report["journal"]["events_omitted"].as_u64().unwrap();
        report["journal"]["events_omitted"] = serde_json::json!(omitted.saturating_add(1));
    }
}

#[cfg(test)]
mod tests;
