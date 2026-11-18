//! Apple resources command behavior and capability regressions.

use super::super::tests::command_registered;

#[test]
fn apple_asset_management_is_settings_only() {
    for command in ["get_apple_speech_support", "prepare_apple_speech_language"] {
        let permissions = include_str!("../../../permissions/app.toml");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains(&format!("\"{command}\"")))
            .collect();
        assert_eq!(permitted.len(), 1);
        assert!(permitted[0].contains("identifier = \"app-settings\""));
        assert!(command_registered(command));
    }
}
