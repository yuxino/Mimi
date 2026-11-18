//! Monitor coordinate spaces, saved-frame selection, fitting and screen mapping.

use crate::settings_store::OverlayFrame;
use tauri::AppHandle;

use super::overlay::SubtitleOverlayMetrics;

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

/// Logical work area of the monitor the overlay currently sits on. The
/// desktop-space origin is preserved, including negative coordinates for
/// monitors positioned to the left of or above the primary display.
pub(in crate::windows) fn current_work_area_logical(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
) -> LogicalWorkArea {
    window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
        .map(|monitor| logical_work_area(&monitor))
        .unwrap_or(LogicalWorkArea {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
            coordinate_scale: 1.0,
        })
}

/// Fits the complete expanded frame inside the work area that contains most of
/// it. This is intentionally called only on restore or after native movement
/// settles; live dragging stays entirely in the window server.
pub(in crate::windows) fn fit_user_frame_to_screen(
    app: &AppHandle,
    frame: &mut OverlayFrame,
    minimum_height: f64,
) -> bool {
    let (areas, primary) = available_work_areas(app);
    fit_user_frame_to_work_areas(frame, &areas, primary, minimum_height)
}

pub(in crate::windows) fn fit_user_frame_to_work_areas(
    frame: &mut OverlayFrame,
    areas: &[LogicalWorkArea],
    primary: Option<LogicalWorkArea>,
    minimum_height: f64,
) -> bool {
    let Some(work_area) = choose_work_area_for_frame(frame, areas.iter().copied(), primary) else {
        return false;
    };
    fit_frame_to_work_area(frame, work_area, minimum_height)
}

pub(in crate::windows) fn fit_collapsed_position_to_work_areas(
    frame: &mut OverlayFrame,
    areas: &[LogicalWorkArea],
    primary: Option<LogicalWorkArea>,
) -> bool {
    let mut collapsed = OverlayFrame {
        x: frame.x,
        y: frame.y,
        width: SubtitleOverlayMetrics::COLLAPSED_WIDTH,
        height: SubtitleOverlayMetrics::COLLAPSED_HEIGHT,
    };
    let Some(work_area) = choose_work_area_for_frame(&collapsed, areas.iter().copied(), primary)
    else {
        return false;
    };
    clamp_frame_origin_to_work_area(&mut collapsed, work_area);
    let changed = collapsed.x != frame.x || collapsed.y != frame.y;
    frame.x = collapsed.x;
    frame.y = collapsed.y;
    changed
}

/// Origin for a saved frame (clamped into the visible screen) or the default
/// bottom-center placement 72 logical px above the bottom edge.
pub(in crate::windows) fn default_overlay_origin(
    app: &AppHandle,
    width: f64,
    height: f64,
    saved: &Option<OverlayFrame>,
) -> (f64, f64) {
    if let Some(saved) = saved {
        // Prefer the work area that contains most of the saved frame. This
        // preserves deliberate placement on a left/upper secondary monitor;
        // if that display disappeared, fall back to the primary work area.
        if let Some(work_area) = work_area_for_saved_frame(app, saved) {
            let mut restored = *saved;
            clamp_frame_origin_to_work_area(&mut restored, work_area);
            return (restored.x, restored.y);
        }
        return (saved.x, saved.y);
    }
    // Default to the bottom-center of the primary monitor.
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let work_area = logical_work_area(&monitor);
        return default_overlay_origin_in_work_area(width, height, work_area);
    }
    (0.0, 0.0)
}

/// A monitor work area with logical dimensions and a desktop-space origin.
/// Windows desktop origins are physical pixels and `coordinate_scale` maps
/// logical distances into that coordinate space. Other platforms keep the
/// former all-logical representation with a scale of 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::windows) struct LogicalWorkArea {
    pub(in crate::windows) x: f64,
    pub(in crate::windows) y: f64,
    pub(in crate::windows) width: f64,
    pub(in crate::windows) height: f64,
    pub(in crate::windows) coordinate_scale: f64,
}

impl LogicalWorkArea {
    pub(in crate::windows) fn coordinate_width(self) -> f64 {
        self.width * self.coordinate_scale
    }

    pub(in crate::windows) fn coordinate_height(self) -> f64 {
        self.height * self.coordinate_scale
    }

    pub(in crate::windows) fn coordinate_distance(self, logical: f64) -> f64 {
        logical * self.coordinate_scale
    }

    /// CSS pixels also include WebView zoom (including Windows text scaling).
    /// Preserve the physical work area while expressing its bounds in CSS units.
    #[cfg(any(target_os = "windows", test))]
    pub(in crate::windows) fn with_webview_zoom(self, zoom: f64) -> Self {
        Self {
            width: self.width / zoom,
            height: self.height / zoom,
            coordinate_scale: self.coordinate_scale * zoom,
            ..self
        }
    }
}

pub(in crate::windows) fn logical_work_area(monitor: &tauri::Monitor) -> LogicalWorkArea {
    let scale = monitor.scale_factor().max(1.0);
    let area = monitor.work_area();
    #[cfg(target_os = "windows")]
    let (x, y, coordinate_scale) = (area.position.x as f64, area.position.y as f64, scale);
    #[cfg(not(target_os = "windows"))]
    let (x, y, coordinate_scale) = (
        area.position.x as f64 / scale,
        area.position.y as f64 / scale,
        1.0,
    );
    LogicalWorkArea {
        x,
        y,
        width: area.size.width as f64 / scale,
        height: area.size.height as f64 / scale,
        coordinate_scale,
    }
}

pub(in crate::windows) fn available_work_areas(
    app: &AppHandle,
) -> (Vec<LogicalWorkArea>, Option<LogicalWorkArea>) {
    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| logical_work_area(&monitor));
    let areas = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| logical_work_area(&monitor))
        .collect();
    (areas, primary)
}

pub(in crate::windows) fn work_area_for_saved_frame(
    app: &AppHandle,
    frame: &OverlayFrame,
) -> Option<LogicalWorkArea> {
    let (areas, primary) = available_work_areas(app);
    choose_work_area_for_frame(frame, areas, primary)
}

pub(in crate::windows) fn choose_work_area_for_frame(
    frame: &OverlayFrame,
    areas: impl IntoIterator<Item = LogicalWorkArea>,
    fallback: Option<LogicalWorkArea>,
) -> Option<LogicalWorkArea> {
    let areas = areas.into_iter().collect::<Vec<_>>();
    // A saved frame's origin is always written in its owning monitor's
    // desktop coordinate space. Prefer that unambiguous signal before
    // estimating intersection: the same logical width has different physical
    // extents on neighbouring mixed-DPI displays.
    if let Some(area) = areas
        .iter()
        .copied()
        .find(|area| frame_origin_is_inside_work_area(frame, *area))
    {
        return Some(area);
    }
    areas
        .into_iter()
        .map(|area| (frame_intersection_area(frame, area), area))
        .filter(|(intersection, _)| *intersection > 0.0)
        .max_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, area)| area)
        .or(fallback)
}

fn frame_origin_is_inside_work_area(frame: &OverlayFrame, area: LogicalWorkArea) -> bool {
    frame.x >= area.x
        && frame.x < area.x + area.coordinate_width()
        && frame.y >= area.y
        && frame.y < area.y + area.coordinate_height()
}

/// Maps a saved frame into another display's work area without changing its
/// size or relative placement. Vertical placement preserves the bottom inset,
/// which keeps subtitles near the video controls across differently sized
/// displays; the final fit handles smaller destinations safely.
#[cfg(any(target_os = "macos", test))]
pub(in crate::windows) fn map_frame_between_work_areas(
    frame: OverlayFrame,
    source: LogicalWorkArea,
    target: LogicalWorkArea,
    minimum_height: f64,
) -> OverlayFrame {
    let mut mapped = if work_areas_approximately_equal(source, target) {
        frame
    } else {
        let horizontal_range = (source.width - frame.width).max(0.0);
        let horizontal_fraction = if horizontal_range > 0.0 {
            ((frame.x - source.x) / source.coordinate_scale / horizontal_range).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let bottom_inset = ((source.y + source.coordinate_height() - frame.y)
            / source.coordinate_scale
            - frame.height)
            .max(0.0);
        let target_horizontal_range = (target.width - frame.width).max(0.0);
        OverlayFrame {
            x: target.x + target.coordinate_distance(horizontal_fraction * target_horizontal_range),
            y: target.y + target.coordinate_distance(target.height - frame.height - bottom_inset),
            ..frame
        }
    };
    fit_frame_to_work_area(&mut mapped, target, minimum_height);
    mapped
}

#[cfg(any(target_os = "macos", test))]
fn work_areas_approximately_equal(left: LogicalWorkArea, right: LogicalWorkArea) -> bool {
    (left.x - right.x).abs() < 1.0
        && (left.y - right.y).abs() < 1.0
        && (left.width - right.width).abs() < 1.0
        && (left.height - right.height).abs() < 1.0
        && (left.coordinate_scale - right.coordinate_scale).abs() < 0.001
}

fn frame_intersection_area(frame: &OverlayFrame, area: LogicalWorkArea) -> f64 {
    let width = (frame.x + area.coordinate_distance(frame.width))
        .min(area.x + area.coordinate_width())
        - frame.x.max(area.x);
    let height = (frame.y + area.coordinate_distance(frame.height))
        .min(area.y + area.coordinate_height())
        - frame.y.max(area.y);
    width.max(0.0) * height.max(0.0)
}

fn clamp_frame_origin_to_work_area(frame: &mut OverlayFrame, area: LogicalWorkArea) {
    let max_x = (area.x + area.coordinate_distance(area.width - frame.width)).max(area.x);
    let max_y = (area.y + area.coordinate_distance(area.height - frame.height)).max(area.y);
    frame.x = frame.x.clamp(area.x, max_x);
    frame.y = frame.y.clamp(area.y, max_y);
}

/// Normalizes dimensions first, then origin. Origin-only clamping cannot make
/// an oversized frame fully visible after a display or resolution change.
pub(in crate::windows) fn fit_frame_to_work_area(
    frame: &mut OverlayFrame,
    area: LogicalWorkArea,
    minimum_height: f64,
) -> bool {
    let previous = *frame;
    let maximum_width = area.width.clamp(1.0, SubtitleOverlayMetrics::MAXIMUM_WIDTH);
    let maximum_height = area
        .height
        .clamp(1.0, SubtitleOverlayMetrics::MAXIMUM_HEIGHT);
    let minimum_width = SubtitleOverlayMetrics::MINIMUM_WIDTH.min(maximum_width);
    let minimum_height = minimum_height.min(maximum_height);
    frame.width = frame.width.clamp(minimum_width, maximum_width);
    frame.height = frame.height.clamp(minimum_height, maximum_height);
    clamp_frame_origin_to_work_area(frame, area);
    *frame != previous
}

fn default_overlay_origin_in_work_area(
    width: f64,
    height: f64,
    area: LogicalWorkArea,
) -> (f64, f64) {
    let mut frame = OverlayFrame {
        x: area.x + area.coordinate_distance((area.width - width) / 2.0),
        y: area.y + area.coordinate_distance(area.height - height - 72.0),
        width,
        height,
    };
    clamp_frame_origin_to_work_area(&mut frame, area);
    (frame.x, frame.y)
}

#[cfg(test)]
mod tests;
