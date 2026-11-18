//! Native UI language selection shared by shell menus and OS adapters.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeUiLanguage {
    Chinese,
    TraditionalChinese,
    English,
    Japanese,
    German,
    French,
    Korean,
    Thai,
    Russian,
}

pub(crate) fn effective_native_ui_language(
    override_language: Option<&str>,
    system_language: Option<&str>,
) -> NativeUiLanguage {
    let language = match override_language {
        Some("zh") | Some("zh-TW") | Some("en") | Some("ja") | Some("de") | Some("fr")
        | Some("ko") | Some("th") | Some("ru") => override_language,
        _ => system_language,
    }
    .unwrap_or("en")
    .to_ascii_lowercase()
    .replace('_', "-");
    if language.starts_with("zh") {
        let subtags: Vec<_> = language.split('-').skip(1).collect();
        let traditional = if subtags.contains(&"hant") {
            true
        } else if subtags.contains(&"hans") {
            false
        } else {
            subtags
                .iter()
                .any(|subtag| matches!(*subtag, "tw" | "hk" | "mo"))
        };
        if traditional {
            NativeUiLanguage::TraditionalChinese
        } else {
            NativeUiLanguage::Chinese
        }
    } else if language.starts_with("ja") {
        NativeUiLanguage::Japanese
    } else if language.starts_with("de") {
        NativeUiLanguage::German
    } else if language.starts_with("fr") {
        NativeUiLanguage::French
    } else if language.starts_with("ko") {
        NativeUiLanguage::Korean
    } else if language.starts_with("th") {
        NativeUiLanguage::Thai
    } else if language.starts_with("ru") {
        NativeUiLanguage::Russian
    } else {
        NativeUiLanguage::English
    }
}

/// Reads the operating-system language code when the preference follows the system.
#[cfg(target_os = "macos")]
pub(crate) fn system_language_code() -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    use objc2_foundation::{NSArray, NSString};
    unsafe {
        let locale_class = AnyClass::get(c"NSLocale")?;
        // The formatting locale's base language drops scripts such as Hant
        // and can differ from the user's preferred interface language.
        let languages: *mut NSArray<NSString> = msg_send![locale_class, preferredLanguages];
        languages
            .as_ref()?
            .firstObject()
            .map(|language| language.to_string())
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn system_language_code() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

    // Windows defines LOCALE_NAME_MAX_LENGTH as 85 UTF-16 code units,
    // including the trailing null terminator.
    let mut buffer = [0_u16; 85];
    let length =
        unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), i32::try_from(buffer.len()).ok()?) };
    if length <= 1 {
        return None;
    }
    String::from_utf16(&buffer[..usize::try_from(length - 1).ok()?]).ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn system_language_code() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) fn system_language_code() -> Option<String> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn native_system_language_preserves_the_full_preferred_ui_language() {
        use objc2::{msg_send, runtime::AnyClass};
        use objc2_foundation::{NSArray, NSString};

        // Read the OS UI preference rather than its region-formatting locale.
        // The language code alone loses Chinese scripts and regional variants.
        let class = AnyClass::get(c"NSLocale").unwrap();
        let languages: *mut NSArray<NSString> = unsafe { msg_send![class, preferredLanguages] };
        let expected = unsafe { languages.as_ref() }
            .and_then(|languages| languages.firstObject())
            .map(|language| language.to_string());
        assert_eq!(system_language_code(), expected);
    }
}
