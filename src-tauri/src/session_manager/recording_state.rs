//! Content-free evidence for the desktop's local recording indicator.
//! This describes successful local writes, not recognition or microphone access.
use crate::core::audio::audio_input::{AudioInput, AudioSource};
use crate::core::models::SessionStatus;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioRecordingStatus {
    #[default]
    Off,
    Starting,
    Recording,
    Paused,
    Limited,
    Error,
}

#[derive(Default)]
pub(super) struct RecordingEvidence {
    input: AudioInput,
    owner: Option<(u64, u64)>,
    written: u8,
    failed: u8,
}

impl RecordingEvidence {
    pub(super) fn new(input: AudioInput) -> Self {
        Self {
            input,
            ..Default::default()
        }
    }

    pub(super) fn begin_attempt(&mut self, generation: u64, attempt: u64) {
        self.owner = Some((generation, attempt));
        self.written = 0;
        // A reconnect cannot repair samples lost to a previous write failure.
        // Keep that warning until this archive is cleared or a new one begins.
    }

    pub(super) fn owns(&self, generation: u64, attempt: u64, source: AudioSource) -> bool {
        self.owner == Some((generation, attempt)) && self.input.sources().contains(&source)
    }

    pub(super) fn owner_for_generation(&self, generation: u64) -> Option<(u64, u64)> {
        self.owner
            .filter(|(owner, _)| generation != 0 && *owner == generation)
    }

    pub(super) fn record(
        &mut self,
        generation: u64,
        attempt: u64,
        source: AudioSource,
        success: bool,
    ) {
        if !self.owns(generation, attempt, source) {
            return;
        }
        let bit = source_bit(source);
        if success {
            self.written |= bit;
        } else {
            self.failed |= bit;
        }
    }

    pub(super) fn write_status(&self, limited: bool) -> AudioRecordingStatus {
        if self.failed != 0 {
            AudioRecordingStatus::Error
        } else if limited {
            AudioRecordingStatus::Limited
        } else if self.owner.is_some()
            && self
                .input
                .sources()
                .iter()
                .all(|&source| self.written & source_bit(source) != 0)
        {
            AudioRecordingStatus::Recording
        } else {
            AudioRecordingStatus::Starting
        }
    }

    pub(super) fn status(
        &self,
        enabled: bool,
        session: &SessionStatus,
        paused: bool,
        owner: Option<(u64, u64)>,
        limited: bool,
    ) -> AudioRecordingStatus {
        if !enabled || matches!(session, SessionStatus::Idle) {
            return AudioRecordingStatus::Off;
        }
        let writes = self.write_status(limited);
        if matches!(session, SessionStatus::Error(_)) || writes == AudioRecordingStatus::Error {
            return AudioRecordingStatus::Error;
        }
        if writes == AudioRecordingStatus::Limited {
            return AudioRecordingStatus::Limited;
        }
        if matches!(session, SessionStatus::Stopping) && owner.is_none() {
            // Stopping an already paused session has no capture queue to drain.
            return AudioRecordingStatus::Off;
        }
        if paused {
            return AudioRecordingStatus::Paused;
        }
        if !matches!(session, SessionStatus::Listening | SessionStatus::Stopping)
            || owner.is_none()
            || owner != self.owner
        {
            return AudioRecordingStatus::Starting;
        }
        writes
    }
}

fn source_bit(source: AudioSource) -> u8 {
    match source {
        AudioSource::System => 1,
        AudioSource::Microphone => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listening(evidence: &RecordingEvidence, owner: Option<(u64, u64)>) -> AudioRecordingStatus {
        evidence.status(true, &SessionStatus::Listening, false, owner, false)
    }

    #[test]
    fn recording_requires_successful_writes_from_every_selected_source() {
        for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            let mut evidence = RecordingEvidence::new(input);
            evidence.begin_attempt(7, 11);
            assert_eq!(
                listening(&evidence, Some((7, 11))),
                AudioRecordingStatus::Starting
            );
            for (index, &source) in input.sources().iter().enumerate() {
                evidence.record(7, 11, source, true);
                assert_eq!(
                    listening(&evidence, Some((7, 11))),
                    if index + 1 == input.sources().len() {
                        AudioRecordingStatus::Recording
                    } else {
                        AudioRecordingStatus::Starting
                    },
                );
            }
            // Repeated PCM writes cause no further state transitions.
            for &source in input.sources() {
                let before = evidence.write_status(false);
                evidence.record(7, 11, source, true);
                assert_eq!(evidence.write_status(false), before);
            }
        }
    }

    #[test]
    fn reconnect_and_same_generation_rebuild_require_fresh_source_evidence() {
        let mut evidence = RecordingEvidence::new(AudioInput::Both);
        evidence.begin_attempt(7, 11);
        for source in [AudioSource::System, AudioSource::Microphone] {
            evidence.record(7, 11, source, true);
        }
        assert_eq!(listening(&evidence, None), AudioRecordingStatus::Starting);
        assert_eq!(
            listening(&evidence, Some((8, 12))),
            AudioRecordingStatus::Starting
        );
        evidence.begin_attempt(7, 12);
        evidence.record(7, 11, AudioSource::System, false);
        evidence.record(7, 11, AudioSource::Microphone, true);
        evidence.record(7, 12, AudioSource::System, true);
        assert_eq!(
            listening(&evidence, Some((7, 12))),
            AudioRecordingStatus::Starting
        );
        evidence.record(7, 12, AudioSource::Microphone, true);
        assert_eq!(
            listening(&evidence, Some((7, 12))),
            AudioRecordingStatus::Recording
        );
        evidence.begin_attempt(8, 13);
        evidence.record(7, 12, AudioSource::Microphone, false);
        assert_eq!(
            listening(&evidence, Some((8, 13))),
            AudioRecordingStatus::Starting
        );
    }

    #[test]
    fn an_unselected_source_cannot_create_success_or_failure_evidence() {
        let mut evidence = RecordingEvidence::new(AudioInput::Microphone);
        evidence.begin_attempt(7, 11);
        evidence.record(7, 11, AudioSource::System, true);
        evidence.record(7, 11, AudioSource::System, false);
        assert_eq!(
            listening(&evidence, Some((7, 11))),
            AudioRecordingStatus::Starting
        );
    }

    #[test]
    fn one_healthy_source_never_hides_a_failed_recording_even_after_reconnect() {
        let mut evidence = RecordingEvidence::new(AudioInput::Both);
        evidence.begin_attempt(7, 11);
        evidence.record(7, 11, AudioSource::System, false);
        evidence.record(7, 11, AudioSource::Microphone, true);
        assert_eq!(
            listening(&evidence, Some((7, 11))),
            AudioRecordingStatus::Error
        );
        evidence.begin_attempt(8, 12);
        for source in [AudioSource::System, AudioSource::Microphone] {
            evidence.record(8, 12, source, true);
        }
        assert_eq!(
            listening(&evidence, Some((8, 12))),
            AudioRecordingStatus::Error
        );
        assert_eq!(
            evidence.status(true, &SessionStatus::Listening, true, None, true),
            AudioRecordingStatus::Error,
        );
    }

    #[test]
    fn off_pause_limit_and_failure_have_explicit_lifecycle_priority() {
        let mut evidence = RecordingEvidence::new(AudioInput::System);
        evidence.begin_attempt(7, 11);
        evidence.record(7, 11, AudioSource::System, true);
        assert_eq!(
            evidence.status(true, &SessionStatus::Listening, true, None, false),
            AudioRecordingStatus::Paused,
        );
        assert_eq!(
            evidence.status(true, &SessionStatus::Listening, true, None, true),
            AudioRecordingStatus::Limited,
        );
        assert_eq!(
            evidence.status(
                true,
                &SessionStatus::Connecting,
                false,
                Some((7, 11)),
                false
            ),
            AudioRecordingStatus::Starting,
        );
        assert_eq!(
            evidence.status(
                true,
                &SessionStatus::Error("synthetic".into()),
                false,
                None,
                false
            ),
            AudioRecordingStatus::Error,
        );
        assert_eq!(
            evidence.status(true, &SessionStatus::Idle, false, None, true),
            AudioRecordingStatus::Off
        );
        assert_eq!(
            evidence.status(
                true,
                &SessionStatus::Stopping,
                false,
                evidence.owner_for_generation(7),
                false
            ),
            AudioRecordingStatus::Recording,
        );
        assert_eq!(
            evidence.status(true, &SessionStatus::Stopping, false, None, false),
            AudioRecordingStatus::Off,
        );
        assert_eq!(evidence.owner_for_generation(8), None);
        assert_eq!(evidence.owner_for_generation(0), None);
        assert_eq!(
            evidence.status(false, &SessionStatus::Listening, false, Some((7, 11)), true),
            AudioRecordingStatus::Off,
        );
        evidence = RecordingEvidence::new(AudioInput::System);
        assert_eq!(listening(&evidence, None), AudioRecordingStatus::Starting);
    }

    #[test]
    fn recording_status_serializes_only_its_fixed_content_free_label() {
        for (status, expected) in [
            (AudioRecordingStatus::Off, "off"),
            (AudioRecordingStatus::Starting, "starting"),
            (AudioRecordingStatus::Recording, "recording"),
            (AudioRecordingStatus::Paused, "paused"),
            (AudioRecordingStatus::Limited, "limited"),
            (AudioRecordingStatus::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), expected);
        }
    }
}
