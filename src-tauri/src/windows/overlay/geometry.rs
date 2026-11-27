//! Pure overlay geometry derivation, snapshots, and reconciliation state machine.

use super::resize_gesture::ResizeGesture;
use super::{OverlayMode, OverlayState, SubtitleOverlayMetrics, OVERLAY_FRAME_LAYOUT_VERSION};
use crate::core::ui::overlay_layout::minimum_overlay_height;
use crate::settings_store::{OverlayFrame, Preferences};
use crate::windows::resize::ResizeRegion;
use crate::windows::work_area::{
    fit_collapsed_position_to_work_areas, fit_user_frame_to_work_areas, LogicalWorkArea,
};

pub(super) fn frame_layout_is_current(version: u64) -> bool {
    version == OVERLAY_FRAME_LAYOUT_VERSION
}

fn f64_bits_eq(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

fn point_bits_eq(left: (f64, f64), right: (f64, f64)) -> bool {
    f64_bits_eq(left.0, right.0) && f64_bits_eq(left.1, right.1)
}

fn frame_bits_eq(left: OverlayFrame, right: OverlayFrame) -> bool {
    f64_bits_eq(left.x, right.x)
        && f64_bits_eq(left.y, right.y)
        && f64_bits_eq(left.width, right.width)
        && f64_bits_eq(left.height, right.height)
}

fn work_area_bits_eq(left: LogicalWorkArea, right: LogicalWorkArea) -> bool {
    f64_bits_eq(left.x, right.x)
        && f64_bits_eq(left.y, right.y)
        && f64_bits_eq(left.width, right.width)
        && f64_bits_eq(left.height, right.height)
        && f64_bits_eq(left.coordinate_scale, right.coordinate_scale)
}

fn resize_gesture_bits_eq(left: ResizeGesture, right: ResizeGesture) -> bool {
    point_bits_eq(left.pointer, right.pointer)
        && frame_bits_eq(left.frame, right.frame)
        && work_area_bits_eq(left.work_area, right.work_area)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OverlayApplySnapshot {
    pub(super) mode: OverlayMode,
    pub(super) mode_revision: u64,
    pub(super) session_error: bool,
    pub(super) minimum_height: f64,
    pub(super) user_frame: OverlayFrame,
    pub(super) presentation_frame: Option<OverlayFrame>,
    pub(super) resize_drag: Option<ResizeRegion>,
    pub(super) resize_start: Option<ResizeGesture>,
}

impl PartialEq for OverlayApplySnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.mode_revision == other.mode_revision
            && self.session_error == other.session_error
            && f64_bits_eq(self.minimum_height, other.minimum_height)
            && frame_bits_eq(self.user_frame, other.user_frame)
            && match (self.presentation_frame, other.presentation_frame) {
                (Some(left), Some(right)) => frame_bits_eq(left, right),
                (None, None) => true,
                _ => false,
            }
            && self.resize_drag == other.resize_drag
            && match (self.resize_start, other.resize_start) {
                (Some(left), Some(right)) => resize_gesture_bits_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for OverlayApplySnapshot {}

impl From<&OverlayState> for OverlayApplySnapshot {
    fn from(state: &OverlayState) -> Self {
        Self {
            mode: state.mode,
            mode_revision: state.mode_revision,
            session_error: state.session_error,
            minimum_height: state.minimum_height,
            user_frame: state.user_frame,
            presentation_frame: state.presentation_frame,
            resize_drag: state.resize_drag,
            resize_start: state.resize_start,
        }
    }
}

/// Native reads also include the drag marker so an origin observed before a
/// programmatic transition can never be attached to the state after it.
#[derive(Debug, Clone, Copy)]
pub(super) struct OverlayGeometrySnapshot {
    pub(super) apply: OverlayApplySnapshot,
    pub(super) native_drag_start: Option<(f64, f64)>,
}

impl PartialEq for OverlayGeometrySnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.apply == other.apply
            && match (self.native_drag_start, other.native_drag_start) {
                (Some(left), Some(right)) => point_bits_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for OverlayGeometrySnapshot {}

impl From<&OverlayState> for OverlayGeometrySnapshot {
    fn from(state: &OverlayState) -> Self {
        Self {
            apply: OverlayApplySnapshot::from(state),
            native_drag_start: state.native_drag_start,
        }
    }
}

pub(super) fn geometry_snapshot_is_current(
    state: &OverlayState,
    expected: OverlayGeometrySnapshot,
) -> bool {
    OverlayGeometrySnapshot::from(state) == expected
}

pub(super) fn apply_snapshot_is_current(
    state: &OverlayState,
    expected: OverlayApplySnapshot,
) -> bool {
    OverlayApplySnapshot::from(state) == expected
}

impl OverlayState {
    pub(super) fn effective_mode(&self) -> OverlayMode {
        if self.session_error {
            OverlayMode::Expanded
        } else {
            self.mode
        }
    }

    pub(super) fn effective_minimum_height(&self) -> f64 {
        if self.session_error {
            self.minimum_height.max(280.0)
        } else {
            self.minimum_height
        }
    }

    pub(in crate::windows) fn effective_frame(&self) -> OverlayFrame {
        let mut frame = self.presentation_frame.unwrap_or(self.user_frame);
        frame.height = frame.height.max(self.effective_minimum_height());
        frame
    }

    pub(super) fn set_session_error(&mut self, enabled: bool) -> bool {
        if self.session_error == enabled {
            return false;
        }
        self.session_error = enabled;
        self.native_drag_start = None;
        self.resize_drag = None;
        self.resize_start = None;
        true
    }

    pub(super) fn update_minimum_height(&mut self, minimum_height: f64) -> bool {
        if self.minimum_height == minimum_height {
            return false;
        }
        self.minimum_height = minimum_height;
        if self.session_error {
            // The resulting native resize/fit is programmatic, even if an
            // earlier manual error drag had already settled.
            self.native_drag_start = None;
        }
        // A smaller requirement lets the user resize later; it never shrinks
        // their chosen frame or resets its position/width automatically.
        self.user_frame.height = self.user_frame.height.max(minimum_height);
        if let Some(frame) = self.presentation_frame.as_mut() {
            frame.height = frame.height.max(minimum_height);
        }
        true
    }
}

pub(super) fn minimum_height_for_preferences(preferences: &Preferences) -> f64 {
    minimum_overlay_height(
        preferences.audio_input,
        preferences.subtitle_display_mode,
        preferences.target_language,
        preferences.font_size,
        preferences.show_subtitle_timestamps,
    )
}

/// OS window geometry derived from `(mode, user_frame)`; all logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WindowGeometry {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) min: (f64, f64),
    pub(super) max: (f64, f64),
}

/// Pure derivation of the window geometry from the overlay state. `screen` is
/// the logical size of the screen the overlay currently sits on.
pub(super) fn geometry_for(
    mode: OverlayMode,
    user_frame: &OverlayFrame,
    minimum_height: f64,
) -> WindowGeometry {
    let min = (SubtitleOverlayMetrics::MINIMUM_WIDTH, minimum_height);
    let max = (
        SubtitleOverlayMetrics::MAXIMUM_WIDTH,
        SubtitleOverlayMetrics::MAXIMUM_HEIGHT,
    );
    match mode {
        OverlayMode::Expanded => WindowGeometry {
            x: user_frame.x,
            y: user_frame.y,
            width: user_frame.width,
            height: user_frame.height.max(minimum_height),
            min,
            max,
        },
        OverlayMode::Collapsed => WindowGeometry {
            x: user_frame.x,
            y: user_frame.y,
            width: SubtitleOverlayMetrics::COLLAPSED_WIDTH,
            height: SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
            // Fixed size: min == max so the OS cannot resize the collapsed bar.
            min: (
                SubtitleOverlayMetrics::COLLAPSED_WIDTH,
                SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
            ),
            max: (
                SubtitleOverlayMetrics::COLLAPSED_WIDTH,
                SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
            ),
        },
    }
}

pub(super) fn presentation_frame_matches_observed(
    state: &OverlayState,
    observed: &OverlayFrame,
) -> bool {
    let Some(presentation) = state.presentation_frame else {
        return false;
    };
    let geometry = geometry_for(state.mode, &presentation, state.minimum_height);
    let expected = OverlayFrame {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
    };
    frames_approximately_equal(&expected, observed)
}

/// Returns `true` when `observed` is user-authored geometry and promotes it to
/// the canonical frame. A frame that exactly matches the active-Space
/// presentation remains a runtime override and must not reach preferences.
pub(super) fn adopt_observed_frame(state: &mut OverlayState, observed: OverlayFrame) -> bool {
    if state.session_error {
        if !state
            .native_drag_start
            .is_some_and(|start| native_drag_changed_position(start, &observed))
        {
            return false;
        }
        promote_observed_user_frame(state, observed);
        return true;
    }
    if presentation_frame_matches_observed(state, &observed) {
        if state
            .presentation_frame
            .is_some_and(|frame| frames_approximately_equal(&frame, &state.user_frame))
        {
            // A return-to-saved-display move needs one matching event to
            // suppress persistence, then no longer needs an override.
            state.presentation_frame = None;
        }
        return false;
    }
    promote_observed_user_frame(state, observed);
    true
}

pub(super) fn promote_observed_user_frame(state: &mut OverlayState, observed: OverlayFrame) {
    if state.session_error {
        if state
            .native_drag_start
            .is_some_and(|start| native_drag_changed_position(start, &observed))
        {
            // Only explicit native movement can replace this temporary
            // presentation. Retain the user's normal expanded dimensions.
            state.user_frame.x = observed.x;
            state.user_frame.y = observed.y;
            state.presentation_frame = None;
            // Native dragging can pause longer than the debounce and then
            // continue. Retain its intent until a programmatic transition;
            // matching events never count as another placement.
            state.native_drag_start = Some((observed.x, observed.y));
        }
        return;
    }
    // start_dragging only moves the window; custom resize gestures own size.
    // Its observed dimensions may still belong to an expansion animation.
    let native_move = state.native_drag_start.take().is_some();
    state.presentation_frame = None;
    match state.mode {
        OverlayMode::Expanded if !native_move => {
            state.user_frame = observed;
            state.user_frame.height = state.user_frame.height.max(state.minimum_height);
        }
        OverlayMode::Expanded | OverlayMode::Collapsed => {
            state.user_frame.x = observed.x;
            state.user_frame.y = observed.y;
        }
    }
}

/// Reconciles an outstanding native drag before another state transition can
/// overwrite its frame. Returns whether fitting the promoted frame requires a
/// corrective OS geometry write; promotion itself is persisted immediately.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeDragReconcile {
    pub(super) promoted: bool,
    pub(super) fit_changed: bool,
}

#[derive(Debug)]
pub(super) struct OverlayModeTransition {
    pub(super) mode_changed: bool,
    pub(super) geometry_changed: bool,
    pub(super) frame_to_persist: Option<OverlayFrame>,
}

/// Native observations reconcile explicit movement only. An unmarked size can
/// be an intermediate animation frame and must never replace the expanded
/// target when the user reverses a transition.
pub(super) fn transition_overlay_mode(
    state: &mut OverlayState,
    observed: Option<OverlayFrame>,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
    collapsed: bool,
) -> OverlayModeTransition {
    let manual_drag = reconcile_native_drag(state, observed, work_areas, primary_work_area);
    let frame_to_persist = manual_drag.promoted.then_some(state.user_frame);
    let new_mode = if collapsed {
        OverlayMode::Collapsed
    } else {
        OverlayMode::Expanded
    };
    let mode_changed = state.mode != new_mode;
    if mode_changed {
        if !collapsed && !state.session_error {
            // The screen configuration may have changed while collapsed;
            // keep the expanded frame on the visible screen.
            let minimum_height = state.minimum_height;
            if let Some(frame) = state.presentation_frame.as_mut() {
                fit_user_frame_to_work_areas(frame, work_areas, primary_work_area, minimum_height);
            } else {
                fit_user_frame_to_work_areas(
                    &mut state.user_frame,
                    work_areas,
                    primary_work_area,
                    minimum_height,
                );
            }
        }
        state.mode = new_mode;
        state.mode_revision = state.mode_revision.wrapping_add(1);
        if state.session_error {
            state.native_drag_start = None;
        }
    }
    OverlayModeTransition {
        mode_changed,
        geometry_changed: mode_changed || manual_drag.fit_changed,
        frame_to_persist,
    }
}

pub(super) fn reconcile_native_drag(
    state: &mut OverlayState,
    observed: Option<OverlayFrame>,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
) -> NativeDragReconcile {
    let Some(start) = state.native_drag_start else {
        return NativeDragReconcile::default();
    };
    let Some(observed) = observed else {
        return NativeDragReconcile::default();
    };
    if !native_drag_changed_position(start, &observed) {
        if !state.session_error {
            state.native_drag_start = None;
        }
        return NativeDragReconcile::default();
    }
    promote_observed_user_frame(state, observed);
    let fit_changed = fit_promoted_user_frame(state, &observed, work_areas, primary_work_area);
    NativeDragReconcile {
        promoted: true,
        fit_changed,
    }
}

/// Fit after confirmed user movement. Error presentation may need a larger
/// temporary height or a smaller screen fit, neither of which is a user resize.
pub(super) fn fit_promoted_user_frame(
    state: &mut OverlayState,
    observed: &OverlayFrame,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
) -> bool {
    if state.session_error {
        let mut shown = state.effective_frame();
        fit_user_frame_to_work_areas(
            &mut shown,
            work_areas,
            primary_work_area,
            state.effective_minimum_height(),
        );
        state.user_frame.x = shown.x;
        state.user_frame.y = shown.y;
        state.native_drag_start = Some((shown.x, shown.y));
        return !frames_approximately_equal(&shown, observed);
    }
    let minimum_height = state.minimum_height;
    match state.mode {
        OverlayMode::Expanded => {
            fit_user_frame_to_work_areas(
                &mut state.user_frame,
                work_areas,
                primary_work_area,
                minimum_height,
            ) || !frames_approximately_equal(&state.user_frame, observed)
        }
        OverlayMode::Collapsed => fit_collapsed_position_to_work_areas(
            &mut state.user_frame,
            work_areas,
            primary_work_area,
        ),
    }
}

fn positions_approximately_equal(left: (f64, f64), right: (f64, f64)) -> bool {
    (left.0 - right.0).abs() < 1.0 && (left.1 - right.1).abs() < 1.0
}

pub(super) fn native_drag_changed_position(start: (f64, f64), observed: &OverlayFrame) -> bool {
    !positions_approximately_equal(start, (observed.x, observed.y))
}

fn frames_approximately_equal(left: &OverlayFrame, right: &OverlayFrame) -> bool {
    (left.x - right.x).abs() < 1.0
        && (left.y - right.y).abs() < 1.0
        && (left.width - right.width).abs() < 1.0
        && (left.height - right.height).abs() < 1.0
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn active_space_follow_allowed(state: &OverlayState) -> bool {
    state.resize_drag.is_none() && !state.session_error
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn update_presentation_frame(state: &mut OverlayState, next: OverlayFrame) -> bool {
    let changed = !frames_approximately_equal(&state.effective_frame(), &next);
    if !changed && frames_approximately_equal(&next, &state.user_frame) {
        state.presentation_frame = None;
    } else {
        state.presentation_frame = Some(next);
    }
    changed
}
