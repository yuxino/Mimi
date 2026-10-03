//! A fixed, content-free support snapshot. Never accept logs, endpoint URLs,
//! profile/device names, paths, credentials, or subtitle content as inputs.
use crate::core::diagnostics::{TranslationLatencyKind, TranslationRecovery};
use crate::core::models::TranslationMode;
use crate::core::provider::ProviderKind;
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
}

impl SafeFailure {
    pub fn from_error(error: &str) -> Self {
        let unknown = Self {
            phase: "unknown",
            category: "unknown",
            code: "OTHER",
        };
        if error.len() > 128 {
            return unknown;
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
            };
        }
        let runtime_failure = match error {
            "subtitle_text_too_large" => {
                Some(("subtitles", "size_limit", "SUBTITLE_TEXT_TOO_LARGE"))
            }
            "transport_error" => Some(("websocket_connection", "transport", "TRANSPORT_ERROR")),
            "provider_event_backlog_overflow" => Some((
                "provider_events",
                "backlog",
                "PROVIDER_EVENT_BACKLOG_OVERFLOW",
            )),
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
            };
        }
        if error == "credential_authentication_failed" {
            return Self {
                phase: "unknown",
                category: "authentication",
                code: "AUTHENTICATION_FAILED",
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
            };
        }
        if error == "The selected sound output is unavailable. Stop subtitles and choose another sound source in Settings." { return Self { phase: "capture_setup", category: "device_unavailable", code: "OUTPUT_UNAVAILABLE" }; }
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
            Some("timeout") => "timeout",
            Some("authentication") => "authentication",
            Some("request") => "request_rejected",
            Some("service") => "service_error",
            Some("rate_limit") => "rate_limit",
            Some("task_failed") => "unknown",
            _ => return unknown,
        };
        let code = match parts.next() {
            Some("CLIENT_ERROR") => "CLIENT_ERROR",
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
        }
    }

    /// Known local transport/MT labels win over arbitrary provider messages.
    pub fn from_provider_error(code: &str, message: &str) -> Self {
        match code {
            "translation_rate_limited"
            | "translation_temporarily_unavailable"
            | "translation_backlog_overflow"
            | "provider_event_backlog_overflow"
            | "subtitle_text_too_large"
            | "transport_error" => Self::from_error(code),
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
    Lifecycle { action: LifecycleAction },
    Status { status: DiagnosticStatus },
    Failure { classification: SafeFailure },
    Recovery { action: RecoveryAction },
    TranslationBackoff { recovery: TranslationRecovery },
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
mod tests {
    use super::*;

    #[test]
    fn microphone_diagnostics_use_fixed_labels_without_device_names() {
        let facts = DiagnosticFacts {
            output_selection: OutputSelection::DefaultMicrophone,
            ..Default::default()
        };
        let report = render(facts);
        assert!(report.contains("default_microphone"));
        let dual = render(DiagnosticFacts {
            output_selection: OutputSelection::SystemAndMicrophone,
            ..Default::default()
        });
        assert!(dual.contains("system_and_microphone"));
        for (message, code) in [
            (
                "Microphone capture permission was denied.",
                "MICROPHONE_PERMISSION_DENIED",
            ),
            (
                "No default microphone is available.",
                "MICROPHONE_UNAVAILABLE",
            ),
            (
                "Microphone capture could not be started.",
                "MICROPHONE_START_FAILED",
            ),
        ] {
            assert_eq!(SafeFailure::from_error(message).code, code);
        }
        assert_eq!(
            SafeFailure::from_error("Private microphone name: failed").code,
            "OTHER"
        );
    }

    #[test]
    fn issue_link_has_a_fixed_public_destination_and_only_safe_facts() {
        let issue = issue_link(DiagnosticFacts {
            last_error: Some((
                SafeFailure::from_error(
                    "private-token /Users/private https://private.invalid original-text",
                ),
                7,
            )),
            ..Default::default()
        });
        let url = url::Url::parse(&issue.url).unwrap();
        assert_eq!(url.host_str(), Some("github.com"));
        assert_eq!(url.path(), "/yuxino/mimi/issues/new");
        assert!(issue.url.len() <= 6000);
        let body = url.query_pairs().find(|(key, _)| key == "body").unwrap().1;
        assert!(body.contains(&issue.report));
        for secret in [
            "private-token",
            "/Users/private",
            "private.invalid",
            "original-text",
        ] {
            assert!(!body.contains(secret));
        }
        assert!(body.contains("before submitting"));
        assert!(!issue.requires_paste);
    }

    #[test]
    fn windows_output_categories_are_values_not_private_names() {
        for (selection, availability) in [
            (OutputSelection::SystemDefault, Availability::Available),
            (OutputSelection::ManualOutput, Availability::Unavailable),
        ] {
            let report = render(DiagnosticFacts {
                output_selection: selection,
                output_availability: availability,
                ..Default::default()
            });
            assert!(report.contains("output_selection"));
        }
    }

    #[test]
    fn unknown_and_oversized_errors_cannot_leak_into_reports() {
        let marker =
            "synthetic-secret /Users/private audio-words https://private.invalid/api?token=secret";
        for error in [
            marker.to_string(),
            marker.repeat(10000),
            "audio3_error.recognition.timeout.private-token".into(),
            "audio3_error.recognition.timeout.CLIENT_ERROR.private-text".into(),
        ] {
            let report = render(DiagnosticFacts {
                last_error: Some((SafeFailure::from_error(&error), 0)),
                ..Default::default()
            });
            for excluded in [
                "synthetic-secret",
                "/Users/private",
                "audio-words",
                "https://private",
                "private-token",
                "private-text",
            ] {
                assert!(!report.contains(excluded));
            }
            assert!(report.len() < 4096);
        }
    }

    #[test]
    fn missing_state_is_unknown_and_timeout_does_not_assert_wrong_output() {
        let report = render(DiagnosticFacts::default());
        let json: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(json["capture"]["observation"], serde_json::Value::Null);
        assert_eq!(json["os"]["version"], "unknown");
        // Preparing diagnostics must not trigger another Keychain read or
        // mistake a saved credential/healthy transport for authentication.
        assert_eq!(json["credentials"]["state"], "unknown");
        assert_eq!(json["credentials"]["authentication"], "unknown");
        let error = SafeFailure::from_error("audio3_error.recognition.timeout.CLIENT_ERROR");
        assert_eq!(error.phase, "asr_recognition");
        assert_eq!(error.category, "timeout");
        assert_ne!(error.category, "device_unavailable");
    }

    #[test]
    fn credential_storage_categories_use_fixed_content_free_codes() {
        for (label, code) in [
            (
                "credential_store_unavailable",
                "CREDENTIAL_STORE_UNAVAILABLE",
            ),
            (
                "credential_service_unavailable",
                "CREDENTIAL_SERVICE_UNAVAILABLE",
            ),
            (
                "credential_store_access_denied",
                "CREDENTIAL_STORE_ACCESS_DENIED",
            ),
        ] {
            let failure = SafeFailure::from_error(label);
            assert_eq!(failure.phase, "setup");
            assert_eq!(failure.category, "credential_storage");
            assert_eq!(failure.code, code);
        }
        assert_eq!(
            SafeFailure::from_error("credential_service_unavailable: private detail").code,
            "OTHER"
        );
    }

    #[test]
    fn concurrent_reports_keep_schema_bounded_and_independent() {
        std::thread::scope(|scope| {
            for index in 0..16 {
                scope.spawn(move || {
                    for _ in 0..50 {
                        let report = render(DiagnosticFacts {
                            elapsed_ms: index,
                            last_error: Some((
                                SafeFailure::from_error(
                                    "audio3_error.setup.authentication.INVALID_API_KEY",
                                ),
                                1,
                            )),
                            ..Default::default()
                        });
                        let json: serde_json::Value = serde_json::from_str(&report).unwrap();
                        assert_eq!(json["schema"], "mimi.support.v2");
                        assert_eq!(
                            json["last_error"]["classification"]["category"],
                            "authentication"
                        );
                        assert_eq!(json["snapshot"]["elapsed_since_app_start_ms"], index);
                        assert!(report.len() < 4096);
                    }
                });
            }
        });
    }

    #[test]
    fn mt_failure_labels_and_provider_codes_remain_fixed_and_private() {
        let marker = "synthetic-secret /Users/private caption-words https://private.invalid";
        for (label, category, code) in [
            (
                "translation_rate_limited",
                "rate_limit",
                "TRANSLATION_RATE_LIMITED",
            ),
            (
                "translation_temporarily_unavailable",
                "service_error",
                "TRANSLATION_TEMPORARILY_UNAVAILABLE",
            ),
            (
                "translation_backlog_overflow",
                "backlog",
                "TRANSLATION_BACKLOG_OVERFLOW",
            ),
        ] {
            let failure = SafeFailure::from_provider_error(label, marker);
            assert_eq!(failure.phase, "text_translation");
            assert_eq!(failure.category, category);
            assert_eq!(failure.code, code);
            assert_eq!(SafeFailure::from_error(label), failure);
            assert_eq!(
                SafeFailure::from_error(&format!("{label}: {marker}")).code,
                "OTHER"
            );
            let mut journal = DiagnosticJournal::default();
            journal.record(
                DiagnosticEvent::Failure {
                    classification: failure,
                },
                20,
            );
            let report = render(DiagnosticFacts {
                journal: journal.snapshot(),
                ..Default::default()
            });
            for excluded in [
                "synthetic-secret",
                "/Users/private",
                "caption-words",
                "private.invalid",
            ] {
                assert!(!report.contains(excluded));
            }
        }
        assert_eq!(
            SafeFailure::from_provider_error(marker, marker).code,
            "OTHER"
        );
    }

    #[test]
    fn journal_is_bounded_monotonic_and_draft_counts_do_not_evict_state_changes() {
        let mut journal = DiagnosticJournal::default();
        journal.observe_status(DiagnosticStatus::Connecting, 8);
        journal.observe_status(DiagnosticStatus::Connecting, 9);
        for index in 1..100 {
            journal.record(
                DiagnosticEvent::Recovery {
                    action: RecoveryAction::Retrying,
                },
                if index % 2 == 0 { 2 } else { index },
            );
        }
        for _ in 0..2_000 {
            journal.observe_text(TextEventKind::SourceDraft);
        }
        journal.observe_text(TextEventKind::ConfirmedPair);
        journal.counts.translation_draft_events = u64::MAX;
        journal.observe_text(TextEventKind::TranslationDraft);
        let snapshot = journal.snapshot();
        assert_eq!(snapshot.events_observed, 100);
        assert_eq!(snapshot.events_omitted, 100 - RECENT_EVENT_LIMIT as u64);
        assert_eq!(snapshot.recent_events.len(), RECENT_EVENT_LIMIT);
        assert_eq!(snapshot.recent_events.last().unwrap().sequence, 100);
        assert!(snapshot
            .recent_events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence
                && pair[0].elapsed_since_app_start_ms <= pair[1].elapsed_since_app_start_ms));
        assert_eq!(snapshot.text_event_counts.source_draft_events, 2_000);
        assert_eq!(snapshot.text_event_counts.confirmed_pair_events, 1);
        assert_eq!(
            snapshot.text_event_counts.translation_draft_events,
            u64::MAX
        );
    }

    #[test]
    fn full_journal_report_fits_the_byte_bound_and_uses_the_fixed_paste_fallback() {
        let recovery = TranslationRecovery {
            reason: crate::core::diagnostics::TranslationRecoveryReason::TemporarilyUnavailable,
            retry_after_ms: u64::MAX,
            retry_scheduled: false,
        };
        let mut journal = DiagnosticJournal::default();
        for _ in 0..RECENT_EVENT_LIMIT {
            journal.record(DiagnosticEvent::TranslationBackoff { recovery }, u64::MAX);
        }
        let issue = issue_link(DiagnosticFacts {
            elapsed_ms: u64::MAX,
            api_latency_ms: Some(u64::MAX),
            translation_latency_ms: Some(u64::MAX),
            translation_latency_kind: Some(TranslationLatencyKind::Request),
            translation_recovery: Some(recovery),
            journal: journal.snapshot(),
            ..Default::default()
        });
        assert!(issue.report.len() <= REPORT_BYTE_LIMIT);
        let report: serde_json::Value = serde_json::from_str(&issue.report).unwrap();
        assert_eq!(
            report["journal"]["recent_events"].as_array().unwrap().len(),
            RECENT_EVENT_LIMIT
        );
        assert_eq!(
            report["service"]["translation_recovery"]["retryScheduled"],
            false
        );
        assert_eq!(report["service"]["translation_duration_kind"], "request");
        assert!(issue.requires_paste);
        assert!(issue.url.len() <= 6000);
        let url = url::Url::parse(&issue.url).unwrap();
        assert_eq!(url.host_str(), Some("github.com"));
        assert_eq!(url.path(), "/yuxino/mimi/issues/new");
        let body = url.query_pairs().find(|(key, _)| key == "body").unwrap().1;
        assert!(body.contains("Paste the diagnostic snapshot"));
        assert!(!body.contains("translation_backoff"));
    }
}
