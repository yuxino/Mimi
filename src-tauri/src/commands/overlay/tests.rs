//! Overlay command behavior and capability regressions.

use super::super::settings::SettingsSnapshotPayload;
use super::super::tests::{command_registered, PartiallyUnavailableSecretStore};
use super::*;

#[test]
fn background_blend_forces_expanded_overlay() {
    assert!(!normalize_overlay_collapsed(true, true));
    assert!(!normalize_overlay_collapsed(false, true));
    assert!(normalize_overlay_collapsed(true, false));
}

#[test]
fn overlay_lock_toggle_persists_without_changing_immersive_mode() {
    for immersive in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at_path(
            directory.path().into(),
            Box::new(PartiallyUnavailableSecretStore),
        );
        store
            .save_preferences(|prefs| prefs.subtitle_blends_with_background = immersive)
            .unwrap();
        for (requested, expected) in [(None, true), (None, false), (Some(true), true)] {
            persist_overlay_lock(&store, requested).unwrap();
            let restarted = SettingsStore::at_path(
                directory.path().into(),
                Box::new(PartiallyUnavailableSecretStore),
            );
            assert_eq!(store.preferences().overlay_locked, expected);
            assert_eq!(restarted.preferences(), store.preferences());
            assert_eq!(
                restarted.preferences().subtitle_blends_with_background,
                immersive
            );
        }
    }
}

#[test]
fn overlay_lock_failed_write_preserves_the_published_preference() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(PartiallyUnavailableSecretStore),
    );
    persist_overlay_lock(&store, Some(true)).unwrap();
    let before = store.preferences();
    let preferences = directory.path().join("preferences.json");
    std::fs::remove_file(&preferences).unwrap();
    std::fs::create_dir(&preferences).unwrap();
    assert!(persist_overlay_lock(&store, None).is_err());
    assert_eq!(store.preferences(), before);
}

#[test]
fn overlay_lock_recovery_does_not_require_a_readable_service_catalog() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("service-profiles.json"), "{invalid").unwrap();
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(PartiallyUnavailableSecretStore),
    );
    for expected in [true, false] {
        persist_overlay_lock(&store, None).unwrap();
        assert_eq!(store.preferences().overlay_locked, expected);
        assert!(SettingsSnapshotPayload::try_from_store(&store).is_err());
        let restarted = SettingsStore::at_path(
            directory.path().into(),
            Box::new(PartiallyUnavailableSecretStore),
        );
        assert_eq!(restarted.preferences(), store.preferences());
    }
}

#[test]
fn native_pointer_cursor_intents_are_registered_and_overlay_scoped() {
    let permissions = include_str!("../../../permissions/app.toml");
    let permitted: Vec<_> = permissions
        .split("[[permission]]")
        .filter(|entry| entry.contains("\"overlay_set_pointer_cursor\""))
        .collect();
    assert_eq!(permitted.len(), 1);
    assert!(permitted[0].contains("identifier = \"app-overlay\""));
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../../../capabilities/overlay.json")).unwrap();
    assert_eq!(capability["windows"], serde_json::json!(["overlay"]));
    assert!(capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|permission| permission == "app-overlay"));
    assert!(command_registered("overlay_set_pointer_cursor"));
    assert_eq!(ensure_overlay_pointer_cursor_window("overlay"), Ok(()));
    for label in ["settings", "overlay-control", "tray-panel", "main", ""] {
        assert_eq!(
            ensure_overlay_pointer_cursor_window(label),
            Err("overlay_pointer_cursor_not_allowed".into())
        );
    }
}

#[test]
fn panel_drag_targets_only_the_subtitle_root_while_open_and_unlocked() {
    assert_eq!(
        panel_drag_target(OverlayControlMode::Panel, false, false),
        Some("overlay")
    );
    assert_eq!(
        panel_drag_target(OverlayControlMode::Island, false, false),
        None
    );
    assert_eq!(
        panel_drag_target(OverlayControlMode::Hidden, false, false),
        None
    );
    assert_eq!(
        panel_drag_target(OverlayControlMode::Panel, true, false),
        None
    );
    assert_eq!(
        panel_drag_target(OverlayControlMode::Panel, false, true),
        None
    );
}

#[test]
fn native_overlay_drag_is_authorized_only_for_overlay_surfaces() {
    let permitted: Vec<_> = include_str!("../../../permissions/app.toml")
        .split("[[permission]]")
        .filter(|entry| entry.contains("\"overlay_move_start\""))
        .collect();
    assert_eq!(permitted.len(), 2);
    for (identifier, label, capability) in [
        (
            "app-overlay",
            "overlay",
            include_str!("../../../capabilities/overlay.json"),
        ),
        (
            "app-overlay-control",
            "overlay-control",
            include_str!("../../../capabilities/overlay-control.json"),
        ),
    ] {
        assert!(permitted
            .iter()
            .any(|entry| entry.contains(&format!("identifier = \"{identifier}\""))));
        let capability: serde_json::Value = serde_json::from_str(capability).unwrap();
        assert_eq!(capability["windows"], serde_json::json!([label]));
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission == identifier));
    }
    assert!(command_registered("overlay_move_start"));
}

#[test]
fn island_width_measurement_is_control_window_scoped() {
    let permissions = include_str!("../../../permissions/app.toml");
    let permitted: Vec<_> = permissions
        .split("[[permission]]")
        .filter(|entry| entry.contains("\"overlay_control_set_island_width\""))
        .collect();
    assert_eq!(permitted.len(), 1);
    assert!(permitted[0].contains("identifier = \"app-overlay-control\""));
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../../../capabilities/overlay-control.json")).unwrap();
    assert_eq!(
        capability["windows"],
        serde_json::json!(["overlay-control"])
    );
    assert!(command_registered("overlay_control_set_island_width"));
}

#[test]
fn embedded_control_capability_is_linux_only_and_does_not_grant_settings_access() {
    let capability: serde_json::Value = serde_json::from_str(include_str!(
        "../../../capabilities/overlay-linux-controls.json"
    ))
    .unwrap();
    assert_eq!(capability["platforms"], serde_json::json!(["linux"]));
    assert_eq!(capability["windows"], serde_json::json!(["overlay"]));
    assert_eq!(
        capability["permissions"],
        serde_json::json!(["app-overlay-control"])
    );
}
