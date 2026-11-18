//! Validation for a single installed subtitle font family, never a CSS list.

pub const MAX_FAMILY_NAME_CHARS: usize = 256;
pub const INVALID_FAMILY_NAME: &str = "subtitle_font_family_invalid";

/// Empty selects the platform's default font stack. Preserve valid names even
/// if the font has been removed; rendering can fall back without losing a choice.
pub fn normalize_family_name(value: &str) -> Result<&str, &'static str> {
    if value.chars().any(char::is_control) {
        return Err(INVALID_FAMILY_NAME);
    }
    let value = value.trim();
    if value.chars().count() > MAX_FAMILY_NAME_CHARS {
        return Err(INVALID_FAMILY_NAME);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_preserve_unicode_and_punctuation_and_trim_outer_spaces() {
        for (value, expected) in [
            ("", ""),
            ("　 ", ""),
            ("　 Noto Sans CJK SC  ", "Noto Sans CJK SC"),
            ("思源黑体", "思源黑体"),
            (
                r#"Example "Font", Family\Name"#,
                r#"Example "Font", Family\Name"#,
            ),
        ] {
            assert_eq!(normalize_family_name(value), Ok(expected));
        }
    }

    #[test]
    fn family_names_reject_controls_even_at_the_edges() {
        for value in ["Font\0Name", "\nFont", "Font\r", "Font\tName", "Font\u{7f}"] {
            assert_eq!(normalize_family_name(value), Err(INVALID_FAMILY_NAME));
        }
    }

    #[test]
    fn family_names_are_bounded_by_unicode_characters_without_truncation() {
        let maximum = "字".repeat(MAX_FAMILY_NAME_CHARS);
        assert_eq!(normalize_family_name(&maximum), Ok(maximum.as_str()));
        assert_eq!(
            normalize_family_name(&format!("{maximum}字")),
            Err(INVALID_FAMILY_NAME)
        );
    }
}
