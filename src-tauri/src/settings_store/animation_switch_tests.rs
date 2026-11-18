//! Preference serialization regressions.

use super::Preferences;

/// An explicit "off" is a choice, not an absent value: it must survive a
/// round trip even though the field defaults to `None` (follow the system).
#[test]
fn animation_switches_round_trip_explicit_values() {
    let mut prefs = Preferences::default();
    assert_eq!(prefs.pulse_animation, None);
    assert_eq!(prefs.subtitle_animation, None);

    prefs.pulse_animation = Some(false);
    prefs.subtitle_animation = Some(true);
    let json = serde_json::to_string(&prefs).unwrap();
    let restored: Preferences = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.pulse_animation, Some(false));
    assert_eq!(restored.subtitle_animation, Some(true));
}
