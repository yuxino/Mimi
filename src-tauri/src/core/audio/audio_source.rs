//! Device-change decisions, independent of WASAPI and the session lifecycle.

/// Which of Windows' three default-output roles a "follow system" source binds
/// to. Media players and meeting apps disagree about which role they own: a
/// call headset is usually the *communications* default while media keeps
/// playing to the speakers, so following the wrong role captures silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultRole {
    Console,
    Multimedia,
    Communications,
}

/// Parses a persisted sound-source value. `None` means the value is an endpoint
/// id (a manual choice); `Some(role)` is the "follow system default" family.
pub fn parse_default_role(source: &str) -> Option<DefaultRole> {
    match source.strip_prefix("role:") {
        Some("console") => Some(DefaultRole::Console),
        Some("multimedia") => Some(DefaultRole::Multimedia),
        Some("communications") => Some(DefaultRole::Communications),
        _ => None,
    }
}

/// Levels above this count as "audible" when choosing an endpoint to follow.
/// About -34 dBFS: above idle noise and synthetic silence padding.
pub const AUDIBLE_HIGH_WATERMARK: f32 = 0.02;
/// Levels below this count as "stale" once an endpoint is already bound. The gap
/// between the two watermarks is deliberate hysteresis: an endpoint that keeps
/// hovering around the high mark cannot make the binding flap.
pub const AUDIBLE_LOW_WATERMARK: f32 = 0.008;
/// How many decisions a binding must survive before follow-audible may move it.
/// The monitor polls once per second, so this is a two-second minimum dwell.
pub const AUDIBLE_MIN_DWELL_TICKS: u32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub enum FollowDecision {
    /// The bound endpoint is still audible (or nothing else is): do not move.
    Keep,
    /// Nothing is audible at all; fall back to the caller's static default.
    NothingAudible,
    /// An audible endpoint has higher role priority, or the binding went quiet.
    SwitchTo(String),
}

/// Picks which render endpoint "follow audible" should capture next.
///
/// Stateful across polls because the decision needs the history: a binding is
/// kept while it stays above the low watermark unless a higher-priority role
/// becomes audible. Higher roles take over immediately; other moves respect
/// the minimum dwell. Pure, so every
/// platform and CI can test the same rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowAudible {
    bound: Option<String>,
    ticks_on_bound: u32,
    min_dwell_ticks: u32,
}

impl Default for FollowAudible {
    fn default() -> Self {
        Self::new(AUDIBLE_MIN_DWELL_TICKS)
    }
}

impl FollowAudible {
    pub fn new(min_dwell_ticks: u32) -> Self {
        Self {
            bound: None,
            ticks_on_bound: 0,
            min_dwell_ticks,
        }
    }

    pub fn bound(&self) -> Option<&str> {
        self.bound.as_deref()
    }

    /// No role preference: preserve a still-audible binding, otherwise choose
    /// the loudest audible endpoint. Used by callers without role information.
    #[cfg(test)]
    pub fn decide(&mut self, candidates: &[(String, f32)]) -> FollowDecision {
        self.decide_with_roles(candidates, &[])
    }

    /// `roles` are backend endpoint ids ordered communications, media, console.
    /// Unknown/non-role outputs remain valid candidates. Higher roles can take
    /// over background playback immediately; louder lower roles cannot steal.
    pub fn decide_with_roles(
        &mut self,
        candidates: &[(String, f32)],
        roles: &[String],
    ) -> FollowDecision {
        let rank = |id: &str| {
            roles
                .iter()
                .position(|role| role == id)
                .unwrap_or(usize::MAX)
        };
        let level_of = |id: &str| {
            candidates
                .iter()
                .find(|(candidate, _)| candidate == id)
                .map(|(_, level)| *level)
                .unwrap_or(0.0)
        };
        let preferred = candidates
            .iter()
            .filter(|(_, level)| level.is_finite() && *level >= AUDIBLE_HIGH_WATERMARK)
            .min_by(|(left_id, left_level), (right_id, right_level)| {
                rank(left_id)
                    .cmp(&rank(right_id))
                    .then_with(|| right_level.total_cmp(left_level))
            });
        // PR89's immediate role promotion bypasses dwell only for a strictly
        // higher role. Quiet/loudness changes still use the existing hysteresis.
        let higher_role = preferred.is_some_and(|(id, _)| {
            self.bound
                .as_deref()
                .is_some_and(|bound| rank(id) < rank(bound))
        });
        if higher_role {
            let id = preferred.unwrap().0.clone();
            self.bound = Some(id.clone());
            self.ticks_on_bound = 0;
            return FollowDecision::SwitchTo(id);
        }
        if let Some(bound) = self.bound.as_deref() {
            if level_of(bound).is_finite() && level_of(bound) >= AUDIBLE_LOW_WATERMARK {
                self.ticks_on_bound = self.ticks_on_bound.saturating_add(1);
                return FollowDecision::Keep;
            }
        }
        let Some((id, _)) = preferred else {
            self.bound = None;
            self.ticks_on_bound = 0;
            return FollowDecision::NothingAudible;
        };
        if self.bound.as_deref() != Some(id.as_str()) {
            if self.bound.is_some() && self.ticks_on_bound < self.min_dwell_ticks {
                self.ticks_on_bound = self.ticks_on_bound.saturating_add(1);
                return FollowDecision::Keep;
            }
            self.bound = Some(id.clone());
            self.ticks_on_bound = 0;
            return FollowDecision::SwitchTo(id.clone());
        }
        self.ticks_on_bound = self.ticks_on_bound.saturating_add(1);
        FollowDecision::Keep
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SourceAction {
    StopMonitor,
    Unavailable,
    Rebind,
    Keep,
}

pub fn source_action(
    generation_current: bool,
    bound: Option<&str>,
    resolved: Option<&str>,
) -> SourceAction {
    if !generation_current {
        SourceAction::StopMonitor
    } else if resolved.is_none() {
        SourceAction::Unavailable
    } else if resolved != bound {
        SourceAction::Rebind
    } else {
        SourceAction::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_role_sources_and_leaves_endpoint_ids_alone() {
        assert_eq!(
            parse_default_role("role:communications"),
            Some(DefaultRole::Communications)
        );
        assert_eq!(
            parse_default_role("role:multimedia"),
            Some(DefaultRole::Multimedia)
        );
        assert_eq!(
            parse_default_role("role:console"),
            Some(DefaultRole::Console)
        );
        // Everything else is a concrete endpoint id (manual choice).
        assert_eq!(
            parse_default_role("{0.0.0.00000000}.{823cf568-6294-403d-b7bf-c6db30f9ec3a}"),
            None
        );
        assert_eq!(parse_default_role(""), None);
        assert_eq!(parse_default_role("role:unknown"), None);
    }

    #[test]
    fn follow_audible_keeps_a_binding_that_is_still_audible() {
        let mut follow = FollowAudible::new(2);
        assert_eq!(
            follow.decide(&[("a".into(), 0.0)]),
            FollowDecision::NothingAudible
        );
        assert_eq!(
            follow.decide(&[("a".into(), 0.5)]),
            FollowDecision::SwitchTo("a".into())
        );
        // "b" is louder, but "a" is still above the low watermark: keep it.
        assert_eq!(
            follow.decide(&[("a".into(), 0.05), ("b".into(), 0.9)]),
            FollowDecision::Keep
        );
        assert_eq!(follow.bound(), Some("a"));
    }

    #[test]
    fn follow_audible_moves_only_after_the_dwell() {
        let mut follow = FollowAudible::new(2);
        let quiet_then_loud = [("a".to_string(), 0.0), ("b".to_string(), 0.5)];
        assert_eq!(
            follow.decide(&[("a".into(), 0.5)]),
            FollowDecision::SwitchTo("a".into())
        );
        // "a" goes quiet while "b" is loud. The dwell is counted from the last
        // move, so the first two decisions must stay put.
        assert_eq!(follow.decide(&quiet_then_loud), FollowDecision::Keep);
        assert_eq!(follow.decide(&quiet_then_loud), FollowDecision::Keep);
        assert_eq!(
            follow.decide(&quiet_then_loud),
            FollowDecision::SwitchTo("b".into())
        );
        // A fresh binding starts its own dwell window.
        assert_eq!(follow.decide(&quiet_then_loud), FollowDecision::Keep);
    }

    #[test]
    fn follow_audible_ignores_endpoints_below_the_high_watermark() {
        let mut follow = FollowAudible::new(0);
        assert_eq!(
            follow.decide(&[("a".into(), 0.01)]),
            FollowDecision::NothingAudible
        );
        assert_eq!(
            follow.decide(&[("a".into(), 0.02)]),
            FollowDecision::SwitchTo("a".into())
        );
        // Silence releases the binding so the next audible endpoint binds at once.
        assert_eq!(
            follow.decide(&[("a".into(), 0.0), ("b".into(), 0.03)]),
            FollowDecision::SwitchTo("b".into())
        );
    }

    #[test]
    fn communication_promotes_immediately_over_media_without_loudness_stealing() {
        let roles = vec!["call".into(), "media".into(), "console".into()];
        let mut follow = FollowAudible::new(2);
        assert_eq!(
            follow.decide_with_roles(&[("media".into(), 0.9)], &roles),
            FollowDecision::SwitchTo("media".into())
        );
        let both = [("media".into(), 0.9), ("call".into(), 0.03)];
        assert_eq!(
            follow.decide_with_roles(&both, &roles),
            FollowDecision::SwitchTo("call".into())
        );
        for _ in 0..4 {
            assert_eq!(
                follow.decide_with_roles(&both, &roles),
                FollowDecision::Keep
            );
        }
        assert_eq!(follow.bound(), Some("call"));
    }

    #[test]
    fn role_promotion_from_non_role_output_is_immediate_but_demotion_waits() {
        let roles = vec!["call".into(), "media".into()];
        let mut follow = FollowAudible::new(2);
        assert_eq!(
            follow.decide_with_roles(&[("other".into(), 0.8)], &roles),
            FollowDecision::SwitchTo("other".into())
        );
        assert_eq!(
            follow.decide_with_roles(&[("other".into(), 0.8), ("media".into(), 0.03)], &roles),
            FollowDecision::SwitchTo("media".into())
        );
        assert_eq!(
            follow.decide_with_roles(
                &[("media".into(), 0.03), ("call".into(), f32::INFINITY)],
                &roles
            ),
            FollowDecision::Keep
        );
        assert_eq!(
            follow.decide_with_roles(&[("call".into(), 0.02)], &roles),
            FollowDecision::SwitchTo("call".into())
        );
        let lower = [("media".into(), 0.8)];
        assert_eq!(
            follow.decide_with_roles(&lower, &roles),
            FollowDecision::Keep
        );
        assert_eq!(
            follow.decide_with_roles(&lower, &roles),
            FollowDecision::Keep
        );
        assert_eq!(
            follow.decide_with_roles(&lower, &roles),
            FollowDecision::SwitchTo("media".into())
        );
    }

    #[test]
    fn role_order_beats_loudness_and_non_role_outputs_still_work() {
        let roles = vec!["call".into(), "media".into(), "console".into()];
        let mut follow = FollowAudible::new(0);
        assert_eq!(
            follow.decide_with_roles(
                &[
                    ("console".into(), 0.9),
                    ("media".into(), 0.03),
                    ("call".into(), 0.0)
                ],
                &roles
            ),
            FollowDecision::SwitchTo("media".into())
        );
        assert_eq!(
            follow.decide_with_roles(&[("other".into(), 0.4)], &roles),
            FollowDecision::SwitchTo("other".into())
        );
        assert_eq!(
            follow.decide_with_roles(&[("other".into(), f32::NAN)], &roles),
            FollowDecision::NothingAudible
        );
        assert_eq!(
            follow.decide_with_roles(&[("call".into(), f32::INFINITY)], &roles),
            FollowDecision::NothingAudible
        );
    }

    #[test]
    fn silence_releases_and_higher_role_can_replace_removed_binding_immediately() {
        let roles = vec!["call".into(), "media".into()];
        let mut follow = FollowAudible::new(2);
        assert_eq!(
            follow.decide_with_roles(&[("call".into(), 0.4)], &roles),
            FollowDecision::SwitchTo("call".into())
        );
        assert_eq!(
            follow.decide_with_roles(&[("call".into(), 0.0)], &roles),
            FollowDecision::NothingAudible
        );
        assert_eq!(
            follow.decide_with_roles(&[("media".into(), 0.5)], &roles),
            FollowDecision::SwitchTo("media".into())
        );
        assert_eq!(
            follow.decide_with_roles(&[("call".into(), 0.3)], &roles),
            FollowDecision::SwitchTo("call".into())
        );
    }

    #[test]
    fn follows_changed_default_and_keeps_stable_endpoint() {
        assert_eq!(
            source_action(true, Some("speakers"), Some("headphones")),
            SourceAction::Rebind
        );
        assert_eq!(
            source_action(true, Some("headphones"), Some("headphones")),
            SourceAction::Keep
        );
    }

    #[test]
    fn disconnected_or_invalid_selection_does_not_fall_back_silently() {
        assert_eq!(
            source_action(true, Some("headphones"), None),
            SourceAction::Unavailable
        );
        assert_eq!(source_action(true, None, None), SourceAction::Unavailable);
    }

    #[test]
    fn stop_invalidates_monitor_even_when_another_endpoint_appears() {
        assert_eq!(
            source_action(false, Some("speakers"), Some("headphones")),
            SourceAction::StopMonitor
        );
        assert_eq!(source_action(false, None, None), SourceAction::StopMonitor);
    }
}

/// Only timestamps are retained; data presence does not imply sound or speech.
#[derive(Default)]
pub struct AudioActivity {
    last_data: Option<std::time::Instant>,
    last_sound: Option<std::time::Instant>,
}

impl AudioActivity {
    pub fn observe(&mut self, samples: &[f32], now: std::time::Instant) {
        if samples.is_empty() {
            return;
        }
        self.last_data = Some(now);
        if samples
            .iter()
            .any(|sample| sample.is_finite() && sample.abs() > 0.001)
        {
            self.last_sound = Some(now);
        }
    }

    pub fn state(&self, now: std::time::Instant) -> (bool, bool) {
        let recent = |at: Option<std::time::Instant>| {
            at.is_some_and(|at| {
                now.saturating_duration_since(at) < std::time::Duration::from_secs(2)
            })
        };
        (recent(self.last_data), recent(self.last_sound))
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn data_silence_sound_and_stale_data_are_distinct() {
        let now = Instant::now();
        let mut activity = AudioActivity::default();
        assert_eq!(activity.state(now), (false, false));
        activity.observe(&[], now);
        assert_eq!(activity.state(now), (false, false));
        activity.observe(&[0.0, 0.0001, f32::NAN, f32::INFINITY], now);
        assert_eq!(activity.state(now), (true, false));
        activity.observe(&[-0.02], now);
        assert_eq!(activity.state(now), (true, true));
        activity.observe(&[0.0], now + Duration::from_secs(3));
        assert_eq!(activity.state(now + Duration::from_secs(3)), (true, false));
        assert_eq!(activity.state(now + Duration::from_secs(5)), (false, false));
    }
}
