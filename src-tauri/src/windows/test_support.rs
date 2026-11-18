//! Shared window-geometry fixtures used by the owning feature tests.

use crate::settings_store::OverlayFrame;

use super::work_area::LogicalWorkArea;

pub(in crate::windows) fn frame(x: f64, y: f64, w: f64, h: f64) -> OverlayFrame {
    OverlayFrame {
        x,
        y,
        width: w,
        height: h,
    }
}

pub(in crate::windows) const WORK_AREA: LogicalWorkArea = LogicalWorkArea {
    x: 0.0,
    y: 24.0,
    width: 1512.0,
    height: 958.0,
    coordinate_scale: 1.0,
};
