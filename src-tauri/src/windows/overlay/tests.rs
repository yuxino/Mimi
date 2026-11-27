use super::geometry::{
    active_space_follow_allowed, adopt_observed_frame, apply_snapshot_is_current,
    fit_promoted_user_frame, frame_layout_is_current, geometry_for, geometry_snapshot_is_current,
    minimum_height_for_preferences, native_drag_changed_position,
    presentation_frame_matches_observed, promote_observed_user_frame, reconcile_native_drag,
    transition_overlay_mode, update_presentation_frame, OverlayApplySnapshot,
    OverlayGeometrySnapshot,
};
use super::resize_gesture::{
    finish_resize_drag, is_valid_pointer, resized_frame_for_pointer, ResizeGesture,
};
use super::{OverlayMode, OverlayState, SubtitleOverlayMetrics, OVERLAY_FRAME_LAYOUT_VERSION};
use crate::settings_store::OverlayFrame;
use crate::settings_store::Preferences;
use crate::windows::resize::ResizeRegion;
use crate::windows::test_support::{frame, WORK_AREA};
use crate::windows::work_area::{fit_frame_to_work_area, LogicalWorkArea};

fn state_with_frame(user_frame: OverlayFrame) -> OverlayState {
    OverlayState {
        mode: OverlayMode::Expanded,
        mode_revision: 0,
        minimum_height: SubtitleOverlayMetrics::MINIMUM_HEIGHT,
        user_frame,
        presentation_frame: None,
        session_error: false,
        native_drag_start: None,
        resize_drag: None,
        resize_start: None,
        last_geometry_event: None,
        geometry_task_pending: false,
        resize_log_at: None,
    }
}

#[test]
fn session_error_expands_temporarily_without_saving_its_geometry() {
    let saved = frame(400.0, 300.0, 360.0, 136.0);
    let mut state = state_with_frame(saved);
    state.mode = OverlayMode::Collapsed;
    let old = OverlayApplySnapshot::from(&state);
    assert!(state.set_session_error(true));
    assert!(!apply_snapshot_is_current(&state, old));
    let shown = state.effective_frame();
    assert_eq!(state.effective_mode(), OverlayMode::Expanded);
    assert_eq!(shown.height, 280.0);
    assert!(!adopt_observed_frame(&mut state, shown));
    promote_observed_user_frame(&mut state, frame(500.0, 300.0, 400.0, 400.0));
    assert_eq!(state.user_frame, saved);
    assert_eq!(state.mode, OverlayMode::Collapsed);
    assert!(state.set_session_error(false));
    assert_eq!(state.effective_mode(), OverlayMode::Collapsed);
    assert_eq!(state.effective_frame(), saved);
}

#[test]
fn session_error_preserves_followed_frame_and_cancels_old_resize() {
    let saved = frame(400.0, 300.0, 640.0, 400.0);
    let followed = frame(-1200.0, 200.0, 640.0, 400.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(followed);
    state.resize_drag = Some(ResizeRegion::Bottom);
    state.native_drag_start = Some((1.0, 2.0));
    assert!(state.set_session_error(true));
    assert!(state.resize_drag.is_none());
    assert!(state.native_drag_start.is_none());
    assert!(!active_space_follow_allowed(&state));
    assert_eq!(state.effective_frame(), followed);
    assert!(state.set_session_error(false));
    assert_eq!(state.presentation_frame, Some(followed));
    assert_eq!(state.user_frame, saved);
}

#[test]
fn error_drag_keeps_normal_dimensions_and_survives_recovery() {
    for mode in [OverlayMode::Expanded, OverlayMode::Collapsed] {
        let saved = frame(400.0, 300.0, 663.0, 257.0);
        let mut state = state_with_frame(saved);
        state.mode = mode;
        state.set_session_error(true);
        state.native_drag_start = Some((saved.x, saved.y));
        let moved = frame(500.0, 400.0, 663.0, 280.0);
        assert!(adopt_observed_frame(&mut state, moved));
        assert!(!fit_promoted_user_frame(
            &mut state,
            &moved,
            &[WORK_AREA],
            Some(WORK_AREA)
        ));
        let chosen = frame(500.0, 400.0, saved.width, saved.height);
        assert_eq!(state.user_frame, chosen);
        assert_eq!(state.effective_frame(), moved);
        assert!(!state.set_session_error(true));
        assert_eq!(state.effective_frame(), moved);
        state.set_session_error(false);
        assert_eq!(state.user_frame, chosen);
        assert_eq!(state.effective_frame(), chosen);
        assert_eq!(state.effective_mode(), mode);
    }
}

#[test]
fn error_drag_reconciles_before_debounce_and_discards_old_space_origin() {
    let saved = frame(400.0, 300.0, 663.0, 257.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(frame(100.0, 200.0, 500.0, 257.0));
    state.set_session_error(true);
    state.native_drag_start = Some((100.0, 200.0));
    let moved = frame(210.0, 330.0, 500.0, 280.0);
    let result = reconcile_native_drag(&mut state, Some(moved), &[WORK_AREA], Some(WORK_AREA));
    assert!(result.promoted);
    assert!(result.fit_changed); // Restore the chosen width, never adopt a screen-fit width.
    assert_eq!(state.presentation_frame, None);
    assert_eq!(state.user_frame, frame(210.0, 330.0, 663.0, 257.0));
    assert!(!active_space_follow_allowed(&state));
    state.set_session_error(false);
    assert_eq!(state.effective_frame(), frame(210.0, 330.0, 663.0, 257.0));
}

#[test]
fn error_drag_fit_updates_only_origin_and_handles_a_paused_gesture() {
    let saved = frame(400.0, 300.0, 663.0, 257.0);
    let mut state = state_with_frame(saved);
    state.set_session_error(true);
    state.native_drag_start = Some((saved.x, saved.y));
    let moved = frame(500.0, 900.0, 663.0, 280.0);
    assert!(adopt_observed_frame(&mut state, moved));
    assert!(fit_promoted_user_frame(
        &mut state,
        &moved,
        &[WORK_AREA],
        Some(WORK_AREA)
    ));
    assert_eq!(state.user_frame, frame(500.0, 702.0, 663.0, 257.0));
    let fitted = frame(500.0, 702.0, 663.0, 280.0);
    assert!(!adopt_observed_frame(&mut state, fitted));
    // Another event after the 350 ms debounce belongs to the same explicit drag.
    let continued = frame(550.0, 600.0, 663.0, 280.0);
    assert!(adopt_observed_frame(&mut state, continued));
    assert_eq!(state.user_frame, frame(550.0, 600.0, 663.0, 257.0));
}

#[test]
fn automatic_error_fits_without_drag_intent_never_replace_the_saved_frame() {
    let saved = frame(900.0, 800.0, 663.0, 257.0);
    let mut state = state_with_frame(saved);
    state.set_session_error(true);
    let fitted = frame(849.0, 702.0, 663.0, 280.0);
    assert!(!adopt_observed_frame(&mut state, fitted));
    assert!(
        !reconcile_native_drag(&mut state, Some(fitted), &[WORK_AREA], Some(WORK_AREA)).promoted
    );
    assert_eq!(state.user_frame, saved);
    state.native_drag_start = Some((fitted.x, fitted.y));
    // A click with no movement must not adopt the automatic fit either.
    assert!(!adopt_observed_frame(&mut state, fitted));
    assert_eq!(state.user_frame, saved);
    // A new layout fit cannot inherit an earlier drag's authorization.
    state.update_minimum_height(200.0);
    assert!(state.native_drag_start.is_none());
    assert!(!adopt_observed_frame(
        &mut state,
        frame(700.0, 500.0, 663.0, 280.0)
    ));
    assert_eq!(state.user_frame, saved);
}

#[test]
fn ordinary_native_drag_reconciles_position_without_adopting_observed_dimensions() {
    let mut state = state_with_frame(frame(400.0, 300.0, 640.0, 136.0));
    state.native_drag_start = Some((400.0, 300.0));
    let moved = frame(1200.0, 400.0, 700.0, 200.0);
    let result = reconcile_native_drag(&mut state, Some(moved), &[WORK_AREA], Some(WORK_AREA));
    assert!(result.promoted);
    assert!(result.fit_changed);
    assert_eq!(state.user_frame, frame(872.0, 400.0, 640.0, 136.0));
    assert!(state.native_drag_start.is_none());
}

#[test]
fn resize_observation_without_a_native_move_still_updates_the_complete_frame() {
    let mut state = state_with_frame(frame(400.0, 300.0, 640.0, 136.0));
    let resized = frame(400.0, 300.0, 700.0, 200.0);

    assert!(adopt_observed_frame(&mut state, resized));
    assert_eq!(state.user_frame, resized);
}

#[test]
fn expanded_passes_user_frame_through() {
    let geometry = geometry_for(
        OverlayMode::Expanded,
        &frame(400.0, 300.0, 640.0, 136.0),
        136.0,
    );
    assert_eq!(
        (geometry.x, geometry.y, geometry.width, geometry.height),
        (400.0, 300.0, 640.0, 136.0)
    );
    assert_eq!(geometry.min, (360.0, 136.0));
    assert_eq!(geometry.max, (1200.0, 600.0));
}

#[test]
fn source_display_and_font_changes_raise_only_the_needed_height() {
    let mut state = state_with_frame(frame(410.0, 300.0, 730.0, 136.0));
    let mut preferences = Preferences {
        subtitle_display_mode: crate::core::models::SubtitleDisplayMode::Bilingual,
        font_size: 18.0,
        ..Preferences::default()
    };
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 160.0));
    preferences.audio_input = crate::core::audio::audio_input::AudioInput::Both;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 272.0));
    preferences.font_size = 20.0;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 280.0));
    preferences.subtitle_display_mode = crate::core::models::SubtitleDisplayMode::Original;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 228.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 280.0));
    preferences.audio_input = crate::core::audio::audio_input::AudioInput::System;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 156.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 280.0));
    preferences.show_subtitle_timestamps = true;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 180.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 280.0));
    preferences.audio_input = crate::core::audio::audio_input::AudioInput::Microphone;
    assert!(!state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 180.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 280.0));
    preferences.show_subtitle_timestamps = false;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 160.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 280.0));
}

#[test]
fn maximum_font_expands_for_bilingual_tracks_without_moving_the_user_frame() {
    let mut state = state_with_frame(frame(410.0, 300.0, 730.0, 156.0));
    let mut preferences = Preferences {
        subtitle_display_mode: crate::core::models::SubtitleDisplayMode::Bilingual,
        font_size: 48.0,
        ..Preferences::default()
    };
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 236.0));
    preferences.show_subtitle_timestamps = true;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 256.0));
    preferences.audio_input = crate::core::audio::audio_input::AudioInput::Both;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 424.0));
    preferences.font_size = 16.0;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 264.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 424.0));
}

#[test]
fn system_timestamp_toggle_expands_a_compact_frame_without_losing_user_geometry() {
    let mut state = state_with_frame(frame(410.0, 300.0, 730.0, 136.0));
    let mut preferences = Preferences {
        show_subtitle_timestamps: true,
        ..Preferences::default()
    };
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 180.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 180.0));

    preferences.show_subtitle_timestamps = false;
    assert!(state.update_minimum_height(minimum_height_for_preferences(&preferences)));
    assert_eq!(state.minimum_height, 156.0);
    assert_eq!(state.user_frame, frame(410.0, 300.0, 730.0, 180.0));
}

#[test]
fn requirement_changes_invalidate_native_reads_and_queued_writes_without_frame_changes() {
    let mut state = state_with_frame(frame(400.0, 300.0, 640.0, 482.0));
    let pending_read = OverlayGeometrySnapshot::from(&state);
    let pending_resize_or_animation = OverlayApplySnapshot::from(&state);
    let before = state.user_frame;
    assert!(state.update_minimum_height(200.0));
    assert_eq!(state.user_frame, before);
    assert!(!geometry_snapshot_is_current(&state, pending_read));
    assert!(!apply_snapshot_is_current(
        &state,
        pending_resize_or_animation
    ));
}

#[test]
fn collapsed_requirement_changes_keep_the_fixed_bar_and_remember_expanded_geometry() {
    let mut state = state_with_frame(frame(400.0, 300.0, 730.0, 136.0));
    state.mode = OverlayMode::Collapsed;
    state.update_minimum_height(200.0);
    let collapsed = geometry_for(state.mode, &state.effective_frame(), state.minimum_height);
    assert_eq!((collapsed.width, collapsed.height), (280.0, 54.0));
    assert_eq!(collapsed.min, collapsed.max);
    assert_eq!((collapsed.x, collapsed.y), (400.0, 300.0));
    state.mode = OverlayMode::Expanded;
    let expanded = geometry_for(state.mode, &state.effective_frame(), state.minimum_height);
    assert_eq!((expanded.width, expanded.height), (730.0, 200.0));
    assert_eq!(expanded.min, (360.0, 200.0));
}

#[test]
fn followed_and_saved_frames_grow_independently_without_changing_their_origins() {
    let mut state = state_with_frame(frame(400.0, 300.0, 730.0, 136.0));
    state.presentation_frame = Some(frame(-1200.0, 200.0, 600.0, 144.0));
    state.update_minimum_height(200.0);
    assert_eq!(state.user_frame, frame(400.0, 300.0, 730.0, 200.0));
    assert_eq!(state.effective_frame(), frame(-1200.0, 200.0, 600.0, 200.0));
    assert!(presentation_frame_matches_observed(
        &state,
        &state.effective_frame()
    ));
    assert!(!adopt_observed_frame(
        &mut state,
        frame(-1200.0, 200.0, 600.0, 200.0)
    ));
    assert_eq!(state.user_frame, frame(400.0, 300.0, 730.0, 200.0));
}

#[test]
fn native_resize_uses_the_current_minimum_and_keeps_the_opposite_edge_anchored() {
    let start = ResizeGesture {
        pointer: (400.0, 300.0),
        frame: frame(400.0, 300.0, 640.0, 482.0),
        work_area: WORK_AREA,
    };
    let resized = resized_frame_for_pointer(ResizeRegion::Top, start, (400.0, 900.0), 200.0);
    assert_eq!(resized.height, 200.0);
    assert_eq!(
        resized.y + resized.height,
        start.frame.y + start.frame.height
    );
    assert_eq!((resized.x, resized.width), (400.0, 640.0));
}

#[test]
fn fitting_and_native_observation_cannot_restore_a_height_below_the_current_requirement() {
    let mut state = state_with_frame(frame(400.0, 300.0, 640.0, 136.0));
    state.update_minimum_height(200.0);
    promote_observed_user_frame(&mut state, frame(400.0, 300.0, 640.0, 136.0));
    assert_eq!(state.user_frame.height, 200.0);
    let mut fitted = frame(400.0, 300.0, 640.0, 136.0);
    assert!(fit_frame_to_work_area(
        &mut fitted,
        WORK_AREA,
        state.minimum_height
    ));
    assert_eq!(fitted, state.user_frame);
}

#[test]
fn collapsed_is_fixed_size_at_user_origin() {
    let geometry = geometry_for(
        OverlayMode::Collapsed,
        &frame(400.0, 300.0, 640.0, 136.0),
        136.0,
    );
    assert_eq!(
        (geometry.x, geometry.y, geometry.width, geometry.height),
        (400.0, 300.0, 280.0, 54.0)
    );
    // min == max: the OS cannot resize the collapsed bar.
    assert_eq!(geometry.min, (280.0, 54.0));
    assert_eq!(geometry.max, (280.0, 54.0));
}

#[test]
fn persisted_layout_requires_an_exact_coordinate_version() {
    assert!(frame_layout_is_current(OVERLAY_FRAME_LAYOUT_VERSION));
    assert!(!frame_layout_is_current(
        OVERLAY_FRAME_LAYOUT_VERSION.saturating_sub(1)
    ));
    assert!(!frame_layout_is_current(OVERLAY_FRAME_LAYOUT_VERSION + 1));
}

#[test]
fn mixed_dpi_resize_converts_physical_delta_back_to_logical_size() {
    let work_area = LogicalWorkArea {
        x: 1920.0,
        y: 0.0,
        width: 2560.0 / 1.5,
        height: 1400.0 / 1.5,
        coordinate_scale: 1.5,
    };
    let start = ResizeGesture {
        pointer: (3030.0, 354.0),
        frame: frame(2070.0, 150.0, 640.0, 136.0),
        work_area,
    };

    let resized =
        resized_frame_for_pointer(ResizeRegion::BottomRight, start, (3180.0, 429.0), 136.0);

    assert_eq!(resized, frame(2070.0, 150.0, 740.0, 186.0));
}

#[test]
fn matching_presentation_geometry_is_not_a_user_move() {
    let saved = frame(400.0, 700.0, 640.0, 136.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

    assert!(!adopt_observed_frame(
        &mut state,
        frame(-1200.0, 700.0, 640.0, 136.0)
    ));
    assert_eq!(state.user_frame, saved);
    assert!(state.presentation_frame.is_some());
}

#[test]
fn return_to_saved_screen_clears_the_runtime_override_after_settle() {
    let saved = frame(400.0, 700.0, 640.0, 136.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(saved);

    assert!(!adopt_observed_frame(&mut state, saved));
    assert_eq!(state.user_frame, saved);
    assert_eq!(state.presentation_frame, None);
}

#[test]
fn single_screen_follow_does_not_create_a_runtime_override() {
    let saved = frame(400.0, 700.0, 640.0, 136.0);
    let mut state = state_with_frame(saved);

    assert!(!update_presentation_frame(&mut state, saved));
    assert_eq!(state.presentation_frame, None);
}

#[test]
fn returning_from_another_screen_waits_for_the_matching_native_event() {
    let saved = frame(400.0, 700.0, 640.0, 136.0);
    let followed = frame(-1200.0, 700.0, 640.0, 136.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(followed);

    assert!(update_presentation_frame(&mut state, saved));
    assert_eq!(state.presentation_frame, Some(saved));

    assert!(!adopt_observed_frame(&mut state, saved));
    assert_eq!(state.presentation_frame, None);
}

#[test]
fn manual_move_promotes_a_followed_frame_to_user_state() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));
    let moved = frame(-1160.0, 680.0, 640.0, 136.0);

    assert!(adopt_observed_frame(&mut state, moved));
    assert_eq!(state.user_frame, moved);
    assert_eq!(state.presentation_frame, None);
}

#[test]
fn collapsed_presentation_compares_against_the_visual_bar() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    state.mode = OverlayMode::Collapsed;
    state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

    assert!(presentation_frame_matches_observed(
        &state,
        &frame(-1200.0, 700.0, 280.0, 54.0)
    ));
}

#[test]
fn collapsed_manual_move_preserves_the_expanded_size() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    state.mode = OverlayMode::Collapsed;
    state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

    assert!(adopt_observed_frame(
        &mut state,
        frame(-1100.0, 680.0, 280.0, 54.0)
    ));
    assert_eq!(state.user_frame, frame(-1100.0, 680.0, 640.0, 136.0));
    assert_eq!(state.presentation_frame, None);
}

#[test]
fn active_space_follow_never_overwrites_a_live_resize() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    assert!(active_space_follow_allowed(&state));

    state.resize_drag = Some(ResizeRegion::BottomRight);

    assert!(!active_space_follow_allowed(&state));
}

#[test]
fn newer_space_follow_cancels_an_older_resize_animation() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    let animation = OverlayApplySnapshot::from(&state);

    state.presentation_frame = Some(frame(-1200.0, 700.0, 640.0, 136.0));

    assert!(!apply_snapshot_is_current(&state, animation));
}

#[test]
fn interrupted_expansion_keeps_the_saved_frame_and_cancels_the_earlier_animation() {
    let saved = frame(400.0, 300.0, 659.0, 328.0);
    let collapsed = frame(400.0, 300.0, 280.0, 54.0);
    let halfway = frame(400.0, 300.0, 469.5, 191.0);
    let mut state = state_with_frame(saved);
    transition_overlay_mode(&mut state, Some(saved), &[WORK_AREA], Some(WORK_AREA), true);
    transition_overlay_mode(
        &mut state,
        Some(collapsed),
        &[WORK_AREA],
        Some(WORK_AREA),
        false,
    );
    let earlier_expansion = OverlayApplySnapshot::from(&state);

    let recollapse = transition_overlay_mode(
        &mut state,
        Some(halfway),
        &[WORK_AREA],
        Some(WORK_AREA),
        true,
    );
    assert!(recollapse.mode_changed);
    assert!(recollapse.geometry_changed);
    assert!(recollapse.frame_to_persist.is_none());
    assert_eq!(state.user_frame, saved);
    assert_eq!(state.mode, OverlayMode::Collapsed);

    transition_overlay_mode(
        &mut state,
        Some(collapsed),
        &[WORK_AREA],
        Some(WORK_AREA),
        false,
    );
    assert_eq!(state.effective_frame(), saved);
    assert_eq!(state.mode, earlier_expansion.mode);
    // The mode and complete frame now match the earlier expansion, but its
    // queued steps must stay cancelled instead of interleaving with this one.
    assert!(!apply_snapshot_is_current(&state, earlier_expansion));
}

#[test]
fn repeated_mode_requests_do_not_cancel_the_current_animation() {
    let saved = frame(400.0, 300.0, 659.0, 328.0);
    let mut state = state_with_frame(saved);
    transition_overlay_mode(&mut state, Some(saved), &[WORK_AREA], Some(WORK_AREA), true);
    let animation = OverlayApplySnapshot::from(&state);

    let repeated = transition_overlay_mode(
        &mut state,
        Some(frame(400.0, 300.0, 469.5, 191.0)),
        &[WORK_AREA],
        Some(WORK_AREA),
        true,
    );
    assert!(!repeated.geometry_changed);
    assert!(apply_snapshot_is_current(&state, animation));
    assert_eq!(state.user_frame, saved);
}

#[test]
fn native_move_during_expansion_preserves_the_saved_size_when_recollapsed() {
    let saved = frame(400.0, 300.0, 659.0, 328.0);
    let mut state = state_with_frame(saved);
    transition_overlay_mode(&mut state, Some(saved), &[WORK_AREA], Some(WORK_AREA), true);
    transition_overlay_mode(
        &mut state,
        Some(frame(400.0, 300.0, 280.0, 54.0)),
        &[WORK_AREA],
        Some(WORK_AREA),
        false,
    );
    state.native_drag_start = Some((saved.x, saved.y));

    let recollapse = transition_overlay_mode(
        &mut state,
        Some(frame(420.0, 300.0, 469.5, 191.0)),
        &[WORK_AREA],
        Some(WORK_AREA),
        true,
    );
    let moved = frame(420.0, 300.0, saved.width, saved.height);
    assert_eq!(state.user_frame, moved);
    assert_eq!(recollapse.frame_to_persist, Some(moved));

    transition_overlay_mode(
        &mut state,
        Some(frame(420.0, 300.0, 280.0, 54.0)),
        &[WORK_AREA],
        Some(WORK_AREA),
        false,
    );
    assert_eq!(state.effective_frame(), moved);
}

#[test]
fn native_drag_marker_does_not_strand_an_animation_mid_size() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    state.mode = OverlayMode::Collapsed;
    let animation = OverlayApplySnapshot::from(&state);

    state.native_drag_start = Some((400.0, 700.0));

    assert!(apply_snapshot_is_current(&state, animation));
}

#[test]
fn resize_end_cancels_a_queued_move_from_the_previous_gesture() {
    let mut state = state_with_frame(frame(400.0, 700.0, 640.0, 136.0));
    state.resize_drag = Some(ResizeRegion::BottomRight);
    state.resize_start = Some(ResizeGesture {
        pointer: (400.0, 700.0),
        frame: state.user_frame,
        work_area: WORK_AREA,
    });
    let queued_move = OverlayApplySnapshot::from(&state);

    assert_eq!(
        finish_resize_drag(&mut state, &[WORK_AREA], Some(WORK_AREA)),
        Some(frame(400.0, 700.0, 640.0, 136.0))
    );

    assert!(!apply_snapshot_is_current(&state, queued_move));
}

#[test]
fn resize_end_without_an_active_gesture_preserves_new_presentation_and_move() {
    let saved = frame(400.0, 300.0, 640.0, 400.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(frame(-1200.0, 200.0, 640.0, 400.0));
    state.native_drag_start = Some((-1200.0, 200.0));
    let before = OverlayGeometrySnapshot::from(&state);

    assert_eq!(finish_resize_drag(&mut state, &[], None), None);

    assert_eq!(OverlayGeometrySnapshot::from(&state), before);
}

#[test]
fn resize_end_after_error_recovery_cannot_restore_an_old_saved_frame() {
    let saved = frame(400.0, 300.0, 640.0, 400.0);
    let followed = frame(-1200.0, 200.0, 640.0, 400.0);
    let mut state = state_with_frame(saved);
    state.presentation_frame = Some(followed);
    state.resize_drag = Some(ResizeRegion::BottomRight);
    state.resize_start = Some(ResizeGesture {
        pointer: (-1200.0, 200.0),
        frame: followed,
        work_area: WORK_AREA,
    });
    state.set_session_error(true);
    state.set_session_error(false);
    let before = OverlayGeometrySnapshot::from(&state);

    assert_eq!(
        finish_resize_drag(&mut state, &[WORK_AREA], Some(WORK_AREA)),
        None
    );

    assert_eq!(OverlayGeometrySnapshot::from(&state), before);
    assert_eq!(state.effective_frame(), followed);
    assert_eq!(state.user_frame, saved);
}

#[test]
fn resize_end_in_a_transient_mode_only_clears_the_resize_owner() {
    for (mode, session_error) in [
        (OverlayMode::Collapsed, false),
        (OverlayMode::Expanded, true),
    ] {
        let saved = frame(400.0, 300.0, 640.0, 400.0);
        let followed = frame(-1200.0, 200.0, 640.0, 400.0);
        let mut state = state_with_frame(saved);
        state.mode = mode;
        state.session_error = session_error;
        state.presentation_frame = Some(followed);
        state.native_drag_start = Some((-1200.0, 200.0));
        state.resize_drag = Some(ResizeRegion::BottomRight);
        state.resize_start = Some(ResizeGesture {
            pointer: (-1200.0, 200.0),
            frame: followed,
            work_area: WORK_AREA,
        });

        assert_eq!(
            finish_resize_drag(&mut state, &[WORK_AREA], Some(WORK_AREA)),
            None
        );

        assert_eq!(state.resize_drag, None);
        assert_eq!(state.resize_start, None);
        assert_eq!(state.mode, mode);
        assert_eq!(state.session_error, session_error);
        assert_eq!(state.presentation_frame, Some(followed));
        assert_eq!(state.native_drag_start, Some((-1200.0, 200.0)));
        assert_eq!(state.user_frame, saved);
    }
}

#[test]
fn resize_end_fits_the_active_frame_and_a_second_end_is_a_noop() {
    let mut state = state_with_frame(frame(1400.0, 900.0, 640.0, 400.0));
    state.resize_drag = Some(ResizeRegion::BottomRight);
    state.resize_start = Some(ResizeGesture {
        pointer: (1400.0, 900.0),
        frame: state.user_frame,
        work_area: WORK_AREA,
    });
    let fitted = frame(872.0, 582.0, 640.0, 400.0);

    assert_eq!(
        finish_resize_drag(&mut state, &[WORK_AREA], Some(WORK_AREA)),
        Some(fitted)
    );
    assert_eq!(state.user_frame, fitted);
    assert_eq!(state.resize_drag, None);
    assert_eq!(state.resize_start, None);
    assert!(active_space_follow_allowed(&state));
    assert_eq!(
        finish_resize_drag(&mut state, &[WORK_AREA], Some(WORK_AREA)),
        None
    );
}

#[test]
fn native_drag_intent_ignores_intermediate_collapse_size() {
    let start = (-1200.0, 700.0);
    let intermediate_animation_frame = frame(-1200.0, 700.0, 430.0, 82.0);

    assert!(!native_drag_changed_position(
        start,
        &intermediate_animation_frame
    ));

    let actually_moved = frame(-1160.0, 680.0, 430.0, 82.0);
    assert!(native_drag_changed_position(start, &actually_moved));
}

#[test]
fn collapse_reconciles_a_manual_drag_without_overwriting_its_fitted_frame() {
    let saved = frame(400.0, 300.0, 640.0, 136.0);
    let mut state = state_with_frame(saved);
    state.native_drag_start = Some((saved.x, saved.y));
    let transition = transition_overlay_mode(
        &mut state,
        Some(frame(1200.0, 400.0, 700.0, 200.0)),
        &[WORK_AREA],
        Some(WORK_AREA),
        true,
    );
    let fitted = frame(872.0, 400.0, saved.width, saved.height);

    assert_eq!(state.user_frame, fitted);
    assert_eq!(state.mode, OverlayMode::Collapsed);
    assert_eq!(transition.frame_to_persist, Some(fitted));
}

#[test]
fn resize_pointer_validation_and_math_reject_non_finite_or_out_of_bounds_coordinates() {
    assert!(is_valid_pointer(0.0, -500.0));
    assert!(!is_valid_pointer(f64::NAN, 100.0));
    assert!(!is_valid_pointer(100.0, f64::INFINITY));
    assert!(!is_valid_pointer(f64::NEG_INFINITY, 100.0));
    assert!(!is_valid_pointer(1_000_001.0, 100.0));

    let start = ResizeGesture {
        pointer: (400.0, 300.0),
        frame: frame(400.0, 300.0, 640.0, 482.0),
        work_area: WORK_AREA,
    };
    for bad_pointer in [
        (f64::NAN, 300.0),
        (400.0, f64::INFINITY),
        (400.0, f64::NEG_INFINITY),
        (2_000_000.0, 300.0),
    ] {
        assert_eq!(
            resized_frame_for_pointer(ResizeRegion::BottomRight, start, bad_pointer, 136.0),
            start.frame
        );
    }
}

#[test]
fn overlay_snapshots_remain_reflexive_even_with_nan_fields() {
    let mut state = state_with_frame(frame(f64::NAN, 300.0, 640.0, 136.0));
    state.presentation_frame = Some(frame(100.0, f64::NAN, 640.0, 136.0));
    state.minimum_height = f64::NAN;
    state.native_drag_start = Some((f64::NAN, 200.0));
    state.resize_drag = Some(ResizeRegion::BottomRight);
    state.resize_start = Some(ResizeGesture {
        pointer: (f64::NAN, 100.0),
        frame: frame(400.0, 300.0, f64::NAN, 136.0),
        work_area: WORK_AREA,
    });

    let apply_snapshot = OverlayApplySnapshot::from(&state);
    let geometry_snapshot = OverlayGeometrySnapshot::from(&state);
    assert!(apply_snapshot_is_current(&state, apply_snapshot));
    assert!(geometry_snapshot_is_current(&state, geometry_snapshot));
}
