//! Overlay resize math: given a drag region, the starting mouse position and
//! window frame, and the current mouse position, compute the new frame. The
//! dragged edge/corner stays anchored, the size is clamped to min/max, the
//! origin recedes when the size clamp kicks in, and the frame is kept at
//! least partially on screen. Pure and unit-tested; the frontend only
//! forwards pointer coordinates.

use crate::settings_store::OverlayFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeRegion {
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl ResizeRegion {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "topLeft" => Some(Self::TopLeft),
            "top" => Some(Self::Top),
            "topRight" => Some(Self::TopRight),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "bottomLeft" => Some(Self::BottomLeft),
            "bottom" => Some(Self::Bottom),
            "bottomRight" => Some(Self::BottomRight),
            _ => None,
        }
    }

    fn moves_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::Left | Self::BottomLeft)
    }

    fn moves_right(self) -> bool {
        matches!(self, Self::TopRight | Self::Right | Self::BottomRight)
    }

    fn moves_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::Top | Self::TopRight)
    }

    fn moves_bottom(self) -> bool {
        matches!(self, Self::BottomLeft | Self::Bottom | Self::BottomRight)
    }
}

fn frame_is_finite(frame: &OverlayFrame) -> bool {
    frame.x.is_finite()
        && frame.y.is_finite()
        && frame.width.is_finite()
        && frame.height.is_finite()
}

/// Computes the new overlay frame for a resize drag.
///
/// All coordinates are logical (CSS) pixels. `screen` is the visible screen
/// size the frame must stay inside of (at least `min_visible` px on screen).
#[allow(clippy::too_many_arguments)]
pub fn apply_drag(
    region: ResizeRegion,
    start_mouse: (f64, f64),
    start_frame: &OverlayFrame,
    current_mouse: (f64, f64),
    min_size: (f64, f64),
    max_size: (f64, f64),
    screen: (f64, f64),
    min_visible: f64,
) -> OverlayFrame {
    if !start_mouse.0.is_finite()
        || !start_mouse.1.is_finite()
        || !current_mouse.0.is_finite()
        || !current_mouse.1.is_finite()
        || !frame_is_finite(start_frame)
        || !min_size.0.is_finite()
        || !min_size.1.is_finite()
        || !max_size.0.is_finite()
        || !max_size.1.is_finite()
        || min_size.0 > max_size.0
        || min_size.1 > max_size.1
        || !screen.0.is_finite()
        || !screen.1.is_finite()
        || !min_visible.is_finite()
    {
        return *start_frame;
    }

    let dx = current_mouse.0 - start_mouse.0;
    let dy = current_mouse.1 - start_mouse.1;
    if !dx.is_finite() || !dy.is_finite() {
        return *start_frame;
    }

    let mut width = start_frame.width;
    let mut height = start_frame.height;
    let mut x = start_frame.x;
    let mut y = start_frame.y;

    if region.moves_left() {
        width = start_frame.width - dx;
        x = start_frame.x + dx;
    } else if region.moves_right() {
        width = start_frame.width + dx;
    }
    if region.moves_top() {
        height = start_frame.height - dy;
        y = start_frame.y + dy;
    } else if region.moves_bottom() {
        height = start_frame.height + dy;
    }
    if !width.is_finite() || !height.is_finite() || !x.is_finite() || !y.is_finite() {
        return *start_frame;
    }

    let clamped_w = width.round().clamp(min_size.0, max_size.0);
    let clamped_h = height.round().clamp(min_size.1, max_size.1);

    // When the size clamp recedes the dragged edge, pull the origin back by
    // the same amount so the opposite edge stays put.
    if region.moves_left() {
        x += width - clamped_w;
    }
    if region.moves_top() {
        y += height - clamped_h;
    }
    if !x.is_finite() || !y.is_finite() {
        return *start_frame;
    }

    // Keep at least `min_visible` px of the window on screen.
    let x_min = -clamped_w + min_visible;
    let x_max = (screen.0 - min_visible).max(x_min);
    x = x.round().clamp(x_min, x_max);
    let y_max = (screen.1 - min_visible).max(0.0);
    y = y.round().clamp(0.0, y_max);

    let next = OverlayFrame {
        x,
        y,
        width: clamped_w,
        height: clamped_h,
    };
    if frame_is_finite(&next) {
        next
    } else {
        *start_frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: (f64, f64) = (360.0, 100.0);
    const MAX: (f64, f64) = (1200.0, 600.0);
    const SCREEN: (f64, f64) = (1440.0, 900.0);

    fn frame(x: f64, y: f64, w: f64, h: f64) -> OverlayFrame {
        OverlayFrame {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn bottom_right_grows_size_and_keeps_origin() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        let result = apply_drag(
            ResizeRegion::BottomRight,
            (0.0, 0.0),
            &start,
            (100.0, 100.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.x, 100.0);
        assert_eq!(result.y, 200.0);
        assert_eq!(result.width, 500.0);
        assert_eq!(result.height, 236.0);
    }

    #[test]
    fn top_left_moves_origin_and_grows_toward_top_left() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        let result = apply_drag(
            ResizeRegion::TopLeft,
            (0.0, 0.0),
            &start,
            (-50.0, -50.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.x, 50.0);
        assert_eq!(result.y, 150.0);
        assert_eq!(result.width, 450.0);
        assert_eq!(result.height, 186.0);
    }

    #[test]
    fn top_right_moves_top_edge_only() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        let result = apply_drag(
            ResizeRegion::TopRight,
            (0.0, 0.0),
            &start,
            (100.0, -30.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.x, 100.0);
        assert_eq!(result.y, 170.0);
        assert_eq!(result.width, 500.0);
        assert_eq!(result.height, 166.0);
    }

    #[test]
    fn top_edge_recedes_when_clamped_to_min_height() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        // Drag the top edge down far past the minimum height (dy > 0 shrinks
        // the window because the top edge moves down).
        let result = apply_drag(
            ResizeRegion::Top,
            (0.0, 0.0),
            &start,
            (0.0, 500.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.height, 100.0);
        // The bottom edge must stay at 200 + 136 = 336; the origin recedes.
        assert_eq!(result.y, 336.0 - 100.0);
    }

    #[test]
    fn top_edge_grows_upward_without_clamp() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        let result = apply_drag(
            ResizeRegion::Top,
            (0.0, 0.0),
            &start,
            (0.0, -50.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.height, 186.0);
        assert_eq!(result.y, 150.0);
    }

    #[test]
    fn left_edge_recedes_when_clamped_to_min_width() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        // Drag the left edge right far past the minimum width (dx > 0 shrinks
        // the window because the left edge moves right).
        let result = apply_drag(
            ResizeRegion::Left,
            (0.0, 0.0),
            &start,
            (800.0, 0.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.width, 360.0);
        // Right edge stays at 100 + 400 = 500.
        assert_eq!(result.x, 500.0 - 360.0);
    }

    #[test]
    fn right_edge_clamps_to_max_width() {
        let start = frame(100.0, 200.0, 1000.0, 136.0);
        let result = apply_drag(
            ResizeRegion::Right,
            (0.0, 0.0),
            &start,
            (500.0, 0.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.width, 1200.0);
        assert_eq!(result.x, 100.0);
    }

    #[test]
    fn frame_stays_partially_visible_when_dragged_off_screen() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        // Drag the left edge far right (shrinks to minimum) and the bottom
        // edge far down, pushing the origin well off-screen.
        let result = apply_drag(
            ResizeRegion::BottomLeft,
            (0.0, 0.0),
            &start,
            (5000.0, 5000.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.width, 360.0);
        // x must keep at least 48px on screen: right edge may leave the right
        // side but x cannot exceed screen - 48.
        assert!(result.x >= -result.width + 48.0);
        assert!(result.x <= SCREEN.0 - 48.0);
        assert!(result.y <= SCREEN.1 - 48.0);
        assert!(result.y >= 0.0);
    }

    #[test]
    fn bottom_edge_grows_without_moving_origin() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        let result = apply_drag(
            ResizeRegion::Bottom,
            (0.0, 0.0),
            &start,
            (0.0, 50.0),
            MIN,
            MAX,
            SCREEN,
            48.0,
        );
        assert_eq!(result.x, 100.0);
        assert_eq!(result.y, 200.0);
        assert_eq!(result.height, 186.0);
    }

    #[test]
    fn all_region_names_parse() {
        assert_eq!(
            ResizeRegion::from_name("topLeft"),
            Some(ResizeRegion::TopLeft)
        );
        assert_eq!(
            ResizeRegion::from_name("bottomRight"),
            Some(ResizeRegion::BottomRight)
        );
        assert_eq!(ResizeRegion::from_name("nope"), None);
    }

    #[test]
    fn non_finite_or_overflowing_coordinates_preserve_start_frame() {
        let start = frame(100.0, 200.0, 400.0, 136.0);
        for (start_mouse, current_mouse) in [
            ((0.0, 0.0), (f64::NAN, 10.0)),
            ((0.0, 0.0), (10.0, f64::INFINITY)),
            ((f64::NEG_INFINITY, 0.0), (10.0, 10.0)),
            ((-f64::MAX, 0.0), (f64::MAX, 0.0)),
        ] {
            let result = apply_drag(
                ResizeRegion::TopLeft,
                start_mouse,
                &start,
                current_mouse,
                MIN,
                MAX,
                SCREEN,
                48.0,
            );
            assert_eq!(result, start);
            assert!(result.x.is_finite());
            assert!(result.y.is_finite());
            assert!(result.width.is_finite());
            assert!(result.height.is_finite());
        }
    }
}
