//! Fixtures behavior regressions.

use super::*;

#[test]
fn ui_live_fixture_keeps_selected_sources_paired_without_confirming_history() {
    for audio_input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(audio_input);
        controller.archive_mut().begin(true, 1);
        controller.did_connect();
        seed_ui_test_live_subtitles(&mut controller, audio_input);

        let subtitles = &controller.state.subtitles;
        let sources: Vec<_> = subtitles
            .tracks
            .iter()
            .map(|track| track.audio_source)
            .collect();
        assert_eq!(sources, audio_input.sources());
        assert!(subtitles.history.is_empty());
        assert_eq!(controller.archive().page("", 0).total, 0);
        for track in &subtitles.tracks {
            let pair = track.preview_pair.as_ref().unwrap();
            assert!(!track.source.is_final);
            assert!(!track.translation.is_final);
            assert!(track.source.utterance_id.is_some());
            assert_eq!(track.source.utterance_id, track.translation.utterance_id);
            assert_eq!(track.source.utterance_id, pair.utterance_id);
            assert_eq!(track.source.text, pair.source);
            assert_eq!(track.translation.text, pair.translation);
            assert!(track.history.is_empty());
        }
        if subtitles.tracks.len() == 2 {
            assert_ne!(
                subtitles.tracks[0].source.utterance_id,
                subtitles.tracks[1].source.utterance_id
            );
        }
    }
}

#[test]
fn ui_export_fixture_survives_stop_and_repeated_finalization_but_resets_on_new_start() {
    let mut controller = TranslationSessionController::default();
    controller.archive_mut().begin(true, 1);
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Synthetic export fixture".into(),
        language: Some("en".into()),
        translation: "合成导出示例".into(),
    });
    let mut recording = crate::core::session_archive::AudioRecording::default();
    recording.begin(true);
    recording.append(16_000, &[0, 0, 1, 0]);
    controller.did_stop();
    let transcript = controller.archive().export().unwrap();
    let audio = recording.export().unwrap();
    for _ in 0..2 {
        finish_ui_test_archive(&mut controller, &mut recording, true);
        assert_eq!(controller.state.status, SessionStatus::Idle);
        assert_eq!(controller.archive().page("Synthetic", 0).total, 1);
        assert_eq!(controller.archive().export().unwrap(), transcript);
        assert_eq!(recording.export().unwrap(), audio);
    }
    // The next session begins fresh; opting out must not retain a fixture.
    controller.archive_mut().begin(false, 2);
    recording.begin(false);
    assert!(controller.archive().export().is_none());
    assert!(recording.export().is_none());
}

#[test]
fn ui_export_audio_fixture_is_eight_seconds_low_amplitude_and_source_specific() {
    for source in [AudioSource::System, AudioSource::Microphone] {
        for retain_for_export in [false, true] {
            let samples = ui_test_audio_samples(source, retain_for_export);
            assert_eq!(
                samples.len(),
                16_000 * 2 * if retain_for_export { 8 } else { 1 }
            );
            let pcm: Vec<_> = samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|sample| i16::from_le_bytes(*sample))
                .collect();
            assert!(pcm.iter().any(|sample| *sample != 0));
            assert!(pcm.iter().all(|sample| sample.abs() <= 1000));
            let mut recording = crate::core::session_archive::AudioRecording::default();
            recording.begin(true);
            recording.append(16_000, &samples);
            let wav = recording.export().unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[24..28], &16_000_u32.to_le_bytes());
            assert_eq!(wav.len(), samples.len() + 44);
            assert_eq!(&wav[44..], samples);
        }
    }
    assert_ne!(
        ui_test_audio_samples(AudioSource::System, true),
        ui_test_audio_samples(AudioSource::Microphone, true),
    );
}

#[test]
fn ordinary_ui_test_finalization_still_clears_synthetic_buffers() {
    let mut controller = TranslationSessionController::default();
    controller.archive_mut().begin(true, 1);
    controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Synthetic export fixture".into(),
        language: Some("en".into()),
        translation: "合成导出示例".into(),
    });
    let mut recording = crate::core::session_archive::AudioRecording::default();
    recording.begin(true);
    recording.append(16_000, &[0, 0]);
    finish_ui_test_archive(&mut controller, &mut recording, false);
    assert_eq!(controller.archive().page("", 0).total, 0);
    assert!(controller.archive().export().is_none());
    assert!(recording.export().is_none());
}
