//! Overlay drag and edge-resize gesture lifecycle and cross-DPI pointer math.

use super::apply::{lock_overlay_after_native_geometry_read, persist_user_frame_if_current};
use super::geometry::OverlayApplySnapshot;
use super::{OverlayMode, OverlayState, OverlayWindowManager, SubtitleOverlayMetrics};
use crate::settings_store::{OverlayFrame, SettingsStore};
use crate::windows::resize::{apply_drag, ResizeRegion};
use crate::windows::work_area::{
    available_work_areas, current_work_area_logical, fit_user_frame_to_work_areas, LogicalWorkArea,
};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

const MAX_POINTER_COORDINATE: f64 = 1_000_000.0;

pub(super) fn is_valid_pointer(x: f64, y: f64) -> bool {
    x.is_finite()
        && y.is_finite()
        && x.abs() <= MAX_POINTER_COORDINATE
        && y.abs() <= MAX_POINTER_COORDINATE
}

fn frame_is_finite(frame: &OverlayFrame) -> bool {
    frame.x.is_finite()
        && frame.y.is_finite()
        && frame.width.is_finite()
        && frame.height.is_finite()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ResizeGesture {
    pub(super) pointer: (f64, f64),
    pub(super) frame: OverlayFrame,
    pub(super) work_area: LogicalWorkArea,
}

impl OverlayWindowManager {
    /// Marks the beginning of Tauri's native window drag. AppKit owns the
    /// gesture, so this origin is the only reliable way to distinguish a
    /// user-authored position from a simultaneous programmatic size animation.
    pub fn move_start(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
    ) -> bool {
        let (mut state, observed, _, _) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        let Some(observed) = observed else {
            return false;
        };
        state.native_drag_start = Some((observed.x, observed.y));
        true
    }

    pub fn move_cancel(state: &Arc<std::sync::Mutex<OverlayState>>) {
        state.lock().unwrap().native_drag_start = None;
    }

    /// Begins an overlay resize drag. `region` is one of topLeft/top/topRight/
    /// left/right/bottomLeft/bottom/bottomRight; `x`/`y` are the pointer
    /// position in the WebView's screen coordinates. Windows replaces those
    /// values with the native physical cursor position.
    pub fn resize_start(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        region_name: &str,
        x: f64,
        y: f64,
    ) -> Result<(), String> {
        if !is_valid_pointer(x, y) {
            return Err("invalid resize pointer coordinates".to_string());
        }
        let region = ResizeRegion::from_name(region_name)
            .ok_or_else(|| format!("unknown resize region: {region_name}"))?;
        let Some(window) = app.get_webview_window("overlay") else {
            return Ok(());
        };
        let work_area = current_work_area_logical(app, &window);
        let pointer = resize_pointer_position((x, y));
        if !is_valid_pointer(pointer.0, pointer.1) {
            return Err("invalid resize pointer coordinates".to_string());
        }
        let (mut state, observed_frame, _, _) =
            lock_overlay_after_native_geometry_read(app, overlay_state);
        // Resizing is meaningless (and corrupting) in any transient mode.
        if state.mode != OverlayMode::Expanded || state.session_error {
            return Ok(());
        }
        state.native_drag_start = None;
        // A resize gesture is explicit user intent. Adopt the currently
        // presented frame before calculating resize anchors, then let the
        // existing resize path persist its final result.
        state.presentation_frame = None;
        if let Some(observed_frame) = observed_frame {
            state.user_frame = observed_frame;
            state.user_frame.height = state.user_frame.height.max(state.minimum_height);
        }
        state.resize_drag = Some(region);
        state.resize_start = Some(ResizeGesture {
            pointer,
            frame: state.user_frame,
            work_area,
        });
        tracing::info!(
            "resize start region={region:?} frame={:?}",
            (
                state.user_frame.x,
                state.user_frame.y,
                state.user_frame.width,
                state.user_frame.height
            )
        );
        Ok(())
    }

    /// Continues a resize drag with the current pointer position. The dragged
    /// edge/corner stays anchored, sizes clamp to min/max, and the frame keeps
    /// at least 48 logical px on screen.
    pub fn resize_move(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        x: f64,
        y: f64,
    ) {
        if !is_valid_pointer(x, y) {
            return;
        }
        if app.get_webview_window("overlay").is_none() {
            return;
        }
        let pointer = resize_pointer_position((x, y));
        if !is_valid_pointer(pointer.0, pointer.1) {
            return;
        }
        let mut state = overlay_state.lock().unwrap();
        if state.mode != OverlayMode::Expanded || state.session_error {
            return;
        }
        let Some(region) = state.resize_drag else {
            return;
        };
        let Some(start) = state.resize_start else {
            return;
        };
        let frame = resized_frame_for_pointer(region, start, pointer, state.minimum_height);
        if !frame_is_finite(&frame) {
            return;
        }
        // Skip sub-pixel jitter: unchanged frames do not need window ops.
        if (frame.x - state.user_frame.x).abs() < 0.5
            && (frame.y - state.user_frame.y).abs() < 0.5
            && (frame.width - state.user_frame.width).abs() < 0.5
            && (frame.height - state.user_frame.height).abs() < 0.5
        {
            return;
        }
        state.user_frame = frame;
        let log_now = std::time::Instant::now();
        let due = state
            .resize_log_at
            .is_none_or(|at| log_now.duration_since(at).as_millis() >= 250);
        if due {
            state.resize_log_at = Some(log_now);
            tracing::info!(
                "resize move frame={:?}",
                (frame.x, frame.y, frame.width, frame.height)
            );
        }
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        Self::apply_resize_if_geometry_current(app, overlay_state, expected_state);
    }

    /// Ends a resize drag and persists the final frame.
    pub fn resize_end(
        app: &AppHandle,
        overlay_state: &Arc<std::sync::Mutex<OverlayState>>,
        settings: &SettingsStore,
    ) {
        let (work_areas, primary_work_area) = available_work_areas(app);
        let mut state = overlay_state.lock().unwrap();
        let Some(frame_to_persist) = finish_resize_drag(&mut state, &work_areas, primary_work_area)
        else {
            return;
        };
        let expected_state = OverlayApplySnapshot::from(&*state);
        drop(state);
        persist_user_frame_if_current(overlay_state, settings, frame_to_persist);
        Self::apply_if_geometry_current(app, overlay_state, expected_state, false);
        tracing::info!("resize end");
    }
}

pub(super) fn finish_resize_drag(
    state: &mut OverlayState,
    work_areas: &[LogicalWorkArea],
    primary_work_area: Option<LogicalWorkArea>,
) -> Option<OverlayFrame> {
    // Error presentation already cancels its resize owner. A later WebView
    // cleanup must not replace the recovered Space frame or a native move.
    state.resize_drag?;
    state.resize_drag = None;
    state.resize_start = None;
    if state.mode != OverlayMode::Expanded || state.session_error {
        return None;
    }
    state.presentation_frame = None;
    state.native_drag_start = None;
    fit_user_frame_to_work_areas(
        &mut state.user_frame,
        work_areas,
        primary_work_area,
        state.minimum_height,
    );
    Some(state.user_frame)
}

/// WebView `screenX`/`screenY` are not a stable global coordinate space on a
/// mixed-DPI Windows desktop. Read the native cursor position there; retain
/// the existing logical event coordinates on other platforms.
fn resize_pointer_position(fallback: (f64, f64)) -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_ok() {
            return (point.x as f64, point.y as f64);
        }
    }
    fallback
}

/// Applies a resize entirely in the work area's desktop coordinate space,
/// then converts only the resulting dimensions back to logical pixels.
pub(super) fn resized_frame_for_pointer(
    region: ResizeRegion,
    start: ResizeGesture,
    pointer: (f64, f64),
    minimum_height: f64,
) -> OverlayFrame {
    let work_area = start.work_area;
    let scale = work_area.coordinate_scale;
    if !is_valid_pointer(pointer.0, pointer.1)
        || !is_valid_pointer(start.pointer.0, start.pointer.1)
        || !scale.is_finite()
        || scale <= 0.0
    {
        return start.frame;
    }
    let local_start_frame = OverlayFrame {
        x: start.frame.x - work_area.x,
        y: start.frame.y - work_area.y,
        width: start.frame.width * scale,
        height: start.frame.height * scale,
    };
    let mut frame = apply_drag(
        region,
        (start.pointer.0 - work_area.x, start.pointer.1 - work_area.y),
        &local_start_frame,
        (pointer.0 - work_area.x, pointer.1 - work_area.y),
        (
            SubtitleOverlayMetrics::MINIMUM_WIDTH * scale,
            minimum_height * scale,
        ),
        (
            SubtitleOverlayMetrics::MAXIMUM_WIDTH * scale,
            SubtitleOverlayMetrics::MAXIMUM_HEIGHT * scale,
        ),
        (work_area.coordinate_width(), work_area.coordinate_height()),
        48.0 * scale,
    );
    frame.x += work_area.x;
    frame.y += work_area.y;
    frame.width /= scale;
    frame.height /= scale;
    if frame_is_finite(&frame) {
        frame
    } else {
        start.frame
    }
}
