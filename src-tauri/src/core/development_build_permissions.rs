//! Select the checked-in application permission template at build time.
//! Development commands never enter a production ACL manifest.
pub fn permissions_for_build(template: &str, development: bool) -> String {
    if development {
        return template.to_owned();
    }
    template
        .lines()
        .filter(|line| !line.trim_start().starts_with("\"development_debug_"))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// AppManifest accepts a glob, so literal checkout paths must escape its syntax.
pub fn escape_glob_path(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            '?' => escaped.push_str("[?]"),
            '*' => escaped.push_str("[*]"),
            '[' => escaped.push_str("[[]"),
            ']' => escaped.push_str("[]]"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = include_str!("../../permissions/app.toml");

    #[test]
    fn production_excludes_debug_permissions_but_retains_product_controls() {
        let permissions = permissions_for_build(TEMPLATE, false);
        assert!(!permissions.contains("development_debug_"));
        for command in [
            "settings_get",
            "session_get_state",
            "session_start",
            "session_stop",
            "settings_save",
            "app_quit",
            "session_export",
            "profile_save_credentials",
            "local_program_pick_path",
            "overlay_move_start",
        ] {
            assert!(permissions.contains(&format!("\"{command}\"")), "{command}");
        }
        for permission in [
            "app-bootstrap",
            "app-settings",
            "app-overlay",
            "app-tray-panel",
        ] {
            assert!(permissions.contains(&format!("identifier = \"{permission}\"")));
        }
    }

    #[test]
    fn explicitly_enabled_dev_keeps_the_complete_template() {
        assert_eq!(permissions_for_build(TEMPLATE, true), TEMPLATE);
        assert!(TEMPLATE.contains("development_debug_start"));
        assert!(TEMPLATE.contains("development_debug_observe"));
    }

    #[test]
    fn windows_and_unix_paths_are_literal_globs() {
        assert_eq!(escape_glob_path("/tmp/app.toml"), "/tmp/app.toml");
        assert_eq!(
            escape_glob_path("C:/Users/A[dev]/permissions?*.toml"),
            "C:/Users/A[[]dev[]]/permissions[?][*].toml"
        );
    }
}
