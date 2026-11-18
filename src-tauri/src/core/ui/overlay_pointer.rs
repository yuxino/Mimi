//! Coordinate conversion and bounded delivery for native overlay hover.

use serde::Serialize;
use std::time::{Duration, Instant};

const MOTION_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct OverlayPointerPosition {
    pub x: f64,
    pub y: f64,
}

/// AppKit view coordinates are logical points, like CSS pixels at the
/// overlay's default zoom. Retina's backing scale must not be applied again.
pub fn css_position(
    point: (f64, f64),
    origin: (f64, f64),
    size: (f64, f64),
    flipped: bool,
) -> Option<OverlayPointerPosition> {
    if ![point.0, point.1, origin.0, origin.1, size.0, size.1]
        .iter()
        .all(|value| value.is_finite())
        || size.0 <= 0.0
        || size.1 <= 0.0
    {
        return None;
    }
    let x = point.0 - origin.0;
    let local_y = point.1 - origin.1;
    let y = if flipped { local_y } else { size.1 - local_y };
    (x >= 0.0 && x < size.0 && y >= 0.0 && y < size.1).then_some(OverlayPointerPosition { x, y })
}

#[derive(Default)]
pub struct PointerMotionGate {
    last_point: Option<OverlayPointerPosition>,
    last_emitted_at: Option<Instant>,
    pending_point: Option<OverlayPointerPosition>,
}

#[derive(Debug, PartialEq)]
pub enum PointerDelivery {
    Position(OverlayPointerPosition),
    Delay(Duration),
    None,
}

impl PointerMotionGate {
    /// Renderer cursor intents belong only to the latest delivered position.
    /// A leave/hide clear invalidates every outstanding reply immediately.
    pub fn accepts_cursor_reply(&self, point: OverlayPointerPosition) -> bool {
        point.x.is_finite() && point.y.is_finite() && self.last_point == Some(point)
    }

    /// Keep only the newest movement until one bounded flush is due. There
    /// is no repeat timer: a flush is scheduled only by real mouse movement.
    pub fn motion(&mut self, point: OverlayPointerPosition, now: Instant) -> PointerDelivery {
        if self.last_point == Some(point) {
            self.pending_point = None;
            return PointerDelivery::None;
        }
        if let Some(last) = self.last_emitted_at {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < MOTION_INTERVAL {
                self.pending_point = Some(point);
                return PointerDelivery::Delay(MOTION_INTERVAL - elapsed);
            }
        }
        self.pending_point = None;
        self.last_point = Some(point);
        self.last_emitted_at = Some(now);
        PointerDelivery::Position(point)
    }

    pub fn flush(&mut self, now: Instant) -> PointerDelivery {
        self.pending_point
            .take()
            .map_or(PointerDelivery::None, |point| self.motion(point, now))
    }

    /// Leaving/hiding never waits for the movement rate limit; a subsequent
    /// entry is delivered immediately even at the same coordinates.
    pub fn clear(&mut self) -> bool {
        let was_inside = self.last_point.take().is_some();
        self.last_emitted_at = None;
        self.pending_point = None;
        was_inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_bounds_and_orientation_produce_css_points_without_retina_scaling() {
        let point = OverlayPointerPosition { x: 20.0, y: 30.0 };
        assert_eq!(
            css_position((25.0, 37.0), (5.0, 7.0), (640.0, 136.0), true),
            Some(point)
        );
        assert_eq!(
            css_position((25.0, 113.0), (5.0, 7.0), (640.0, 136.0), false),
            Some(point)
        );
        // Re-read bounds on each event: resize changes a nonflipped y origin.
        assert_eq!(
            css_position((25.0, 113.0), (5.0, 7.0), (640.0, 236.0), false),
            Some(OverlayPointerPosition { x: 20.0, y: 130.0 })
        );
        assert!(css_position((645.0, 37.0), (5.0, 7.0), (640.0, 136.0), true).is_none());
        assert!(css_position((-1.0, 1.0), (0.0, 0.0), (640.0, 136.0), true).is_none());
        assert!(css_position((1.0, f64::NAN), (0.0, 0.0), (640.0, 136.0), true).is_none());
        assert!(css_position((0.0, 0.0), (0.0, 0.0), (0.0, 0.0), true).is_none());
    }

    #[test]
    fn motion_is_bounded_and_exit_allows_immediate_reentry() {
        let now = Instant::now();
        let a = OverlayPointerPosition { x: 20.0, y: 30.0 };
        let b = OverlayPointerPosition { x: 25.0, y: 30.0 };
        let mut gate = PointerMotionGate::default();
        assert_eq!(gate.motion(a, now), PointerDelivery::Position(a));
        assert_eq!(
            gate.motion(a, now + Duration::from_secs(1)),
            PointerDelivery::None
        );
        assert_eq!(
            gate.motion(b, now + Duration::from_millis(15)),
            PointerDelivery::Delay(Duration::from_millis(1))
        );
        assert_eq!(
            gate.flush(now + Duration::from_millis(16)),
            PointerDelivery::Position(b)
        );
        assert!(gate.clear());
        assert!(!gate.clear());
        assert_eq!(
            gate.motion(b, now + Duration::from_millis(17)),
            PointerDelivery::Position(b)
        );
    }

    #[test]
    fn cursor_replies_require_the_latest_delivered_position_and_are_invalid_after_leave() {
        let now = Instant::now();
        let a = OverlayPointerPosition { x: 20.0, y: 30.0 };
        let b = OverlayPointerPosition { x: 25.0, y: 30.0 };
        let mut gate = PointerMotionGate::default();
        assert!(!gate.accepts_cursor_reply(a));
        assert_eq!(gate.motion(a, now), PointerDelivery::Position(a));
        assert!(gate.accepts_cursor_reply(a));
        assert!(!gate.accepts_cursor_reply(b));
        assert!(!gate.accepts_cursor_reply(OverlayPointerPosition {
            x: f64::NAN,
            y: 30.0,
        }));
        assert!(matches!(
            gate.motion(b, now + Duration::from_millis(1)),
            PointerDelivery::Delay(_)
        ));
        assert!(gate.accepts_cursor_reply(a));
        assert!(!gate.accepts_cursor_reply(b));
        assert_eq!(
            gate.flush(now + Duration::from_millis(16)),
            PointerDelivery::Position(b)
        );
        assert!(!gate.accepts_cursor_reply(a));
        assert!(gate.accepts_cursor_reply(b));
        assert!(gate.clear());
        assert!(!gate.accepts_cursor_reply(a));
        assert!(!gate.accepts_cursor_reply(b));
    }

    #[test]
    fn coalescing_keeps_the_latest_point_and_exit_cancels_the_pending_flush() {
        let now = Instant::now();
        let a = OverlayPointerPosition { x: 20.0, y: 30.0 };
        let b = OverlayPointerPosition { x: 25.0, y: 30.0 };
        let c = OverlayPointerPosition { x: 30.0, y: 30.0 };
        let mut gate = PointerMotionGate::default();
        assert_eq!(gate.motion(a, now), PointerDelivery::Position(a));
        assert!(matches!(
            gate.motion(b, now + Duration::from_millis(1)),
            PointerDelivery::Delay(_)
        ));
        assert!(matches!(
            gate.motion(c, now + Duration::from_millis(2)),
            PointerDelivery::Delay(_)
        ));
        assert_eq!(
            gate.flush(now + Duration::from_millis(16)),
            PointerDelivery::Position(c)
        );
        assert_eq!(
            gate.flush(now + Duration::from_millis(32)),
            PointerDelivery::None
        );
        assert!(matches!(
            gate.motion(b, now + Duration::from_millis(17)),
            PointerDelivery::Delay(_)
        ));
        assert!(gate.clear());
        assert_eq!(
            gate.flush(now + Duration::from_millis(32)),
            PointerDelivery::None
        );
    }
}
