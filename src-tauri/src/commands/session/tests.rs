//! Session command behavior and capability regressions.

use super::super::tests::command_registered;
use super::*;

#[test]
fn overlay_can_restart_an_errored_session() {
    let permissions = include_str!("../../../permissions/app.toml");
    let overlay_permission = permissions
        .split("[[permission]]")
        .find(|entry| entry.contains("identifier = \"app-overlay\""))
        .unwrap();
    assert!(overlay_permission.contains("\"session_start\""));
    assert!(overlay_permission.contains("\"session_stop\""));
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../../../capabilities/overlay.json")).unwrap();
    assert_eq!(capability["windows"], serde_json::json!(["overlay"]));
    assert!(capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|permission| permission == "app-overlay"));
    assert!(command_registered("session_start"));
}

#[test]
fn control_panel_can_retry_without_pause_or_stop_access() {
    let permission = include_str!("../../../permissions/app.toml")
        .split("[[permission]]")
        .find(|entry| entry.contains("identifier = \"app-overlay-control\""))
        .unwrap();
    assert!(!permission.contains("\"session_toggle_paused\""));
    assert!(permission.contains("\"session_start\""));
    assert!(!permission.contains("\"session_stop\""));
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../../../capabilities/overlay-control.json")).unwrap();
    assert_eq!(
        capability["windows"],
        serde_json::json!(["overlay-control"])
    );
    assert!(capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|permission| permission == "app-overlay-control"));
    assert!(command_registered("session_toggle_paused"));
}

#[test]
fn control_panel_retry_ipc_is_authorized_by_its_native_permission() {
    // UI tests replace start with a mock. Check the real callback, store,
    // IPC command and native ACL together so a visible retry can reach Rust.
    let control_window =
        include_str!("../../../../src/windows/overlay-control/OverlayControlWindow.tsx");
    assert!(
        control_window.contains("onRetrySession={errorRequiresConfiguration ? undefined : start}")
    );
    assert!(include_str!("../../../../src/lib/store.ts")
        .split("start: async () => {")
        .nth(1)
        .unwrap()
        .split("return;")
        .next()
        .unwrap()
        .contains("await sessionStart();"));
    let ipc = include_str!("../../../../src/lib/ipc.ts");
    let command = ipc
        .split("export function sessionStart(): Promise<void> {")
        .nth(1)
        .unwrap()
        .split("return invoke(\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(command, "session_start");
    let permission = include_str!("../../../permissions/app.toml")
        .split("[[permission]]")
        .find(|entry| entry.contains("identifier = \"app-overlay-control\""))
        .unwrap();
    assert!(permission.contains(&format!("\"{command}\"")));
    assert!(command_registered(command));
}

#[test]
fn capture_status_is_readable_in_settings_and_control_panel_only() {
    let permissions = include_str!("../../../permissions/app.toml");
    let permitted: Vec<_> = permissions
        .split("[[permission]]")
        .filter(|entry| entry.contains("\"capture_status\""))
        .collect();
    assert_eq!(permitted.len(), 2);
    assert!(permitted
        .iter()
        .all(|entry| entry.contains("identifier = \"app-settings\"")
            || entry.contains("identifier = \"app-overlay-control\"")));
    assert!(command_registered("capture_status"));
}

#[test]
fn live_audio_capture_switches_are_scoped_to_settings_and_control_panel() {
    let permissions = include_str!("../../../permissions/app.toml");
    for command in [
        "session_switch_audio_input",
        "session_switch_system_audio_target",
    ] {
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains(&format!("\"{command}\"")))
            .collect();
        assert_eq!(permitted.len(), 2);
        assert!(permitted
            .iter()
            .all(|entry| entry.contains("identifier = \"app-settings\"")
                || entry.contains("identifier = \"app-overlay-control\"")));
        assert!(command_registered(command));
    }
}

#[test]
fn target_switch_is_scoped_to_settings_and_control_windows() {
    let permissions = include_str!("../../../permissions/app.toml");
    let permitted: Vec<_> = permissions
        .split("[[permission]]")
        .filter(|entry| entry.contains("\"session_switch_target_language\""))
        .collect();
    assert_eq!(permitted.len(), 2);
    for identifier in ["app-settings", "app-overlay-control"] {
        assert!(permitted
            .iter()
            .any(|entry| entry.contains(&format!("identifier = \"{identifier}\""))));
    }
    assert!(command_registered("session_switch_target_language"));
}

#[test]
fn capture_commands_accept_all_explicit_input_selections() {
    for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        assert!(ensure_audio_input_available(input).is_ok());
    }
}
