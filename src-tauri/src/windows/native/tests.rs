use super::{
    overlay_collection_behavior, overlay_window_level, RawSubtitleControlPanel,
    RawSubtitleOverlayPanel,
};
use objc2_app_kit::{NSScreenSaverWindowLevel, NSStatusWindowLevel, NSWindowCollectionBehavior};

#[cfg(target_os = "macos")]
#[test]
fn control_panel_declares_key_eligibility_without_main_window_eligibility() {
    use objc2::{msg_send, ClassType};

    // This checks declared policy, not runtime keyboard delivery. Native
    // typing still requires the explicit makeKeyWindow path and real UI
    // verification; class selectors do not prove a live first responder.
    let control = RawSubtitleControlPanel::class();
    let can_become_key: bool = unsafe { msg_send![control, canBecomeKeyWindow] };
    let can_become_main: bool = unsafe { msg_send![control, canBecomeMainWindow] };
    let key_only_if_needed: bool = unsafe { msg_send![control, becomesKeyOnlyIfNeeded] };
    assert!(can_become_key);
    assert!(!can_become_main);
    assert!(!key_only_if_needed);

    // Reading subtitles keeps the more restrictive existing key policy.
    let subtitles = RawSubtitleOverlayPanel::class();
    let key_only_if_needed: bool = unsafe { msg_send![subtitles, becomesKeyOnlyIfNeeded] };
    assert!(key_only_if_needed);
    let can_become_main: bool = unsafe { msg_send![subtitles, canBecomeMainWindow] };
    assert!(!can_become_main);
}

#[test]
fn overlay_level_covers_true_full_screen_media() {
    assert_eq!(overlay_window_level(), NSScreenSaverWindowLevel);
    assert!(overlay_window_level() > NSStatusWindowLevel);
}

#[test]
fn overlay_can_join_other_apps_full_screen_spaces() {
    let current = NSWindowCollectionBehavior::MoveToActiveSpace
        | NSWindowCollectionBehavior::Managed
        | NSWindowCollectionBehavior::ParticipatesInCycle
        | NSWindowCollectionBehavior::Primary
        | NSWindowCollectionBehavior::FullScreenPrimary;

    let behavior = overlay_collection_behavior(current);

    assert!(behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
    assert!(behavior.contains(NSWindowCollectionBehavior::Stationary));
    assert!(behavior.contains(NSWindowCollectionBehavior::IgnoresCycle));
    assert!(behavior.contains(NSWindowCollectionBehavior::CanJoinAllApplications));
    assert!(behavior.contains(NSWindowCollectionBehavior::FullScreenAuxiliary));
    assert!(!behavior.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
    assert!(!behavior.contains(NSWindowCollectionBehavior::Managed));
    assert!(!behavior.contains(NSWindowCollectionBehavior::Transient));
    assert!(!behavior.contains(NSWindowCollectionBehavior::ParticipatesInCycle));
    assert!(!behavior.contains(NSWindowCollectionBehavior::Primary));
    assert!(!behavior.contains(NSWindowCollectionBehavior::Auxiliary));
    assert!(!behavior.contains(NSWindowCollectionBehavior::FullScreenPrimary));
    assert!(!behavior.contains(NSWindowCollectionBehavior::FullScreenNone));
}
