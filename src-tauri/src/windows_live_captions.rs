//! Read-only bridge to Windows' on-device Live Captions window.
//!
//! This adapter neither captures audio nor controls the system caption window.
//! Opening Live Captions is a separate, explicit user action. Microsoft does not
//! expose a supported API to verify its optional microphone preference; callers
//! must explain that it must remain off before accepting these snapshots.

use serde::Serialize;

#[cfg(any(target_os = "windows", test))]
const MAX_SNAPSHOT_UNITS: usize = 8192;

/// Use an exact system executable match, rather than trusting a filename or
/// Windows directory prefix. QueryFullProcessImageName uses Win32 paths.
#[cfg(any(target_os = "windows", test))]
fn is_system_image(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

#[cfg(any(target_os = "windows", test))]
fn snapshot_text(text: &[u16]) -> Result<String, String> {
    if text.len() > MAX_SNAPSHOT_UNITS {
        return Err("windows_live_captions_unreadable".into());
    }
    String::from_utf16(text).map_err(|_| "windows_live_captions_unreadable".into())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub available: bool,
    pub status: String,
    pub build_number: Option<u32>,
}

#[cfg(target_os = "windows")]
#[path = "windows_live_captions/native.rs"]
mod native;
#[cfg(target_os = "windows")]
pub use native::CaptionSession;

pub async fn support() -> Support {
    #[cfg(target_os = "windows")]
    return native::support().await;
    #[cfg(not(target_os = "windows"))]
    Support {
        available: false,
        status: "unsupported".into(),
        build_number: None,
    }
}

pub fn open() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return native::open();
    #[cfg(not(target_os = "windows"))]
    Err("windows_live_captions_unsupported".into())
}

pub async fn start() -> Result<CaptionSession, String> {
    #[cfg(target_os = "windows")]
    return native::start().await;
    #[cfg(not(target_os = "windows"))]
    Err("windows_live_captions_unsupported".into())
}

#[cfg(not(target_os = "windows"))]
pub struct CaptionSession;

#[cfg(not(target_os = "windows"))]
impl CaptionSession {
    pub async fn recv(&mut self) -> Result<Option<String>, String> {
        Err("windows_live_captions_unsupported".into())
    }

    pub fn cancel(&mut self) {}
}

#[cfg(not(target_os = "windows"))]
impl Drop for CaptionSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_identity_rejects_lookalike_locations_and_names() {
        let expected = r"C:\Windows\System32\LiveCaptions.exe";
        assert!(is_system_image(
            r"c:\WINDOWS\system32\livecaptions.EXE",
            expected
        ));
        for impostor in [
            r"C:\Users\Public\LiveCaptions.exe",
            r"C:\Windows\System32\LiveCaptions.exe.old",
            r"C:\Windows\System32\LiveCaptions\LiveCaptions.exe",
            r"C:\Windows-old\System32\LiveCaptions.exe",
        ] {
            assert!(!is_system_image(impostor, expected));
        }
    }

    #[test]
    fn snapshots_accept_unicode_but_reject_excessive_or_invalid_utf16() {
        let text: Vec<_> = "字幕 🐱".encode_utf16().collect();
        assert_eq!(snapshot_text(&text).unwrap(), "字幕 🐱");
        assert_eq!(snapshot_text(&[]).unwrap(), "");
        assert!(snapshot_text(&vec![b'a' as u16; MAX_SNAPSHOT_UNITS]).is_ok());
        assert!(snapshot_text(&vec![b'a' as u16; MAX_SNAPSHOT_UNITS + 1]).is_err());
        assert!(snapshot_text(&[0xd800]).is_err());
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn other_platforms_never_offer_or_start_a_caption_reader() {
        let support = support().await;
        assert!(!support.available);
        assert_eq!(support.status, "unsupported");
        assert_eq!(support.build_number, None);
        assert!(matches!(open(), Err(error) if error == "windows_live_captions_unsupported"));
        assert!(
            matches!(start().await, Err(error) if error == "windows_live_captions_unsupported")
        );
    }
}
