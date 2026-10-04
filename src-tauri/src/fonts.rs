//! Installed font-family names for the settings picker. Font files and paths
//! never cross IPC, and enumeration does not prompt for local-font access.

use crate::core::subtitle_font::normalize_family_name;
use std::collections::BTreeMap;
use tauri::AppHandle;

const FONT_LIST_UNAVAILABLE: &str = "font_list_unavailable";
const MAX_FONT_FAMILIES: usize = 4096;

/// Keep one bounded, stable list even when a native API returns a family once
/// per style or character set. Dot-prefixed families and vertical aliases are
/// implementation details rather than useful horizontal subtitle choices.
#[derive(Default)]
struct FontFamilies(BTreeMap<String, String>);

impl FontFamilies {
    fn insert(&mut self, family: &str) {
        let Ok(family) = normalize_family_name(family) else {
            return;
        };
        if family.is_empty() || family.starts_with(['.', '@']) {
            return;
        }
        self.0
            .entry(family.to_lowercase())
            .or_insert_with(|| family.to_owned());
        if self.0.len() > MAX_FONT_FAMILIES {
            self.0.pop_last();
        }
    }

    fn into_names(self) -> Vec<String> {
        self.0.into_values().collect()
    }
}

#[tauri::command]
pub async fn installed_font_families(app: AppHandle) -> Result<Vec<String>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    // NSFontManager and the GTK context belong to the native UI thread. Keep
    // their objects inside this closure; only owned family names leave it.
    app.run_on_main_thread(move || {
        let result = platform_font_families(&handle).and_then(|families| {
            if families.is_empty() {
                Err(FONT_LIST_UNAVAILABLE)
            } else {
                Ok(families)
            }
        });
        let _ = sender.send(result);
    })
    .map_err(|_| FONT_LIST_UNAVAILABLE.to_owned())?;
    tokio::time::timeout(std::time::Duration::from_secs(10), receiver)
        .await
        .map_err(|_| FONT_LIST_UNAVAILABLE.to_owned())?
        .map_err(|_| FONT_LIST_UNAVAILABLE.to_owned())?
        .map_err(str::to_owned)
}

#[cfg(target_os = "macos")]
fn platform_font_families(_app: &AppHandle) -> Result<Vec<String>, &'static str> {
    let main_thread = objc2::MainThreadMarker::new().ok_or(FONT_LIST_UNAVAILABLE)?;
    objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
        let manager = objc2_app_kit::NSFontManager::sharedFontManager(main_thread);
        let mut families = FontFamilies::default();
        for family in manager.availableFontFamilies().iter() {
            families.insert(&family.to_string());
        }
        families.into_names()
    }))
    .map_err(|_| FONT_LIST_UNAVAILABLE)
}

#[cfg(target_os = "linux")]
fn platform_font_families(app: &AppHandle) -> Result<Vec<String>, &'static str> {
    use gtk::prelude::*;
    use tauri::Manager;

    let window = app
        .get_webview_window("settings")
        .ok_or(FONT_LIST_UNAVAILABLE)?;
    let native = window.gtk_window().map_err(|_| FONT_LIST_UNAVAILABLE)?;
    let mut families = FontFamilies::default();
    for family in native.pango_context().list_families() {
        families.insert(family.name().as_str());
    }
    Ok(families.into_names())
}

#[cfg(target_os = "windows")]
fn platform_font_families(_app: &AppHandle) -> Result<Vec<String>, &'static str> {
    use windows_sys::Win32::Graphics::Gdi::{
        EnumFontFamiliesExW, GetDC, ReleaseDC, DEFAULT_CHARSET, HDC, LOGFONTW,
    };

    struct ScreenContext(HDC);

    impl Drop for ScreenContext {
        fn drop(&mut self) {
            // The context is released on the same native UI thread that got it.
            unsafe { ReleaseDC(std::ptr::null_mut(), self.0) };
        }
    }

    // A screen DC exposes installed screen fonts without opening font files.
    let context = unsafe { GetDC(std::ptr::null_mut()) };
    if context.is_null() {
        return Err(FONT_LIST_UNAVAILABLE);
    }
    let context = ScreenContext(context);
    let query = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    let mut families = FontFamilies::default();
    // EnumFontFamiliesExW is synchronous: the collector remains alive for every
    // callback. An empty face name and DEFAULT_CHARSET enumerate all families.
    let result = unsafe {
        EnumFontFamiliesExW(
            context.0,
            &query,
            Some(enumerate_windows_family),
            &mut families as *mut FontFamilies as isize,
            0,
        )
    };
    if result == 0 {
        return Err(FONT_LIST_UNAVAILABLE);
    }
    Ok(families.into_names())
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enumerate_windows_family(
    font: *const windows_sys::Win32::Graphics::Gdi::LOGFONTW,
    _metrics: *const windows_sys::Win32::Graphics::Gdi::TEXTMETRICW,
    _font_type: u32,
    data: windows_sys::Win32::Foundation::LPARAM,
) -> i32 {
    if !font.is_null() {
        // GDI supplies a LOGFONTW for this callback; `data` is the borrowed
        // collector passed by the synchronous enumeration above.
        let font = unsafe { &*font };
        let families = unsafe { &mut *(data as *mut FontFamilies) };
        let length = font
            .lfFaceName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(font.lfFaceName.len());
        if let Ok(family) = String::from_utf16(&font.lfFaceName[..length]) {
            families.insert(&family);
        }
    }
    1
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_font_families(_app: &AppHandle) -> Result<Vec<String>, &'static str> {
    Err(FONT_LIST_UNAVAILABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sorted_deduplicated_and_preserve_unicode() {
        let mut families = FontFamilies::default();
        for name in ["  Noto Sans  ", "Arial", "arial", "苹方-简", "Noto Sans"] {
            families.insert(name);
        }
        assert_eq!(families.into_names(), ["Arial", "Noto Sans", "苹方-简"]);
    }

    #[test]
    fn hides_private_vertical_and_malformed_families() {
        let mut families = FontFamilies::default();
        for name in [
            "",
            " ",
            ".AppleSystemUIFont",
            "@SimSun",
            "Bad\0Font",
            "Bad\nFont",
        ] {
            families.insert(name);
        }
        families.insert(&"a".repeat(crate::core::subtitle_font::MAX_FAMILY_NAME_CHARS + 1));
        families.insert("Font, With \"Quotes\"");
        assert_eq!(families.into_names(), ["Font, With \"Quotes\""]);
    }

    #[test]
    fn list_is_bounded_independently_of_enumeration_order() {
        let mut ascending = FontFamilies::default();
        let mut descending = FontFamilies::default();
        for index in 0..MAX_FONT_FAMILIES + 10 {
            ascending.insert(&format!("Family {index:05}"));
        }
        for index in (0..MAX_FONT_FAMILIES + 10).rev() {
            descending.insert(&format!("Family {index:05}"));
        }
        let names = ascending.into_names();
        assert_eq!(names.len(), MAX_FONT_FAMILIES);
        assert_eq!(names, descending.into_names());
    }

    #[test]
    fn enumeration_permission_is_limited_to_settings() {
        let permissions = include_str!("../permissions/app.toml");
        let allowed_by: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|section| section.contains("\"installed_font_families\""))
            .collect();
        assert_eq!(allowed_by.len(), 1);
        assert!(allowed_by[0].contains("identifier = \"app-settings\""));
    }
}
