//! Local app picker data. Names and identities must never enter diagnostics.
#[cfg(target_os = "windows")]
use super::SystemAudioCaptureError;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioApplication {
    pub id: String,
    pub name: String,
    /// Transient local presentation only; never persist icons with capture targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_data_url: Option<String>,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(super) const MAX_APPLICATION_ICON_PNG_BYTES: usize = 8 * 1024;
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const MAX_APPLICATION_ICON_SNAPSHOT_BYTES: usize = 1024 * 1024;

/// A fresh budget for one already sorted/bounded application snapshot. There is
/// no persistent icon cache; the frontend replaces its previous snapshot.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(super) struct ApplicationIconBudget {
    remaining_bytes: usize,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl Default for ApplicationIconBudget {
    fn default() -> Self {
        Self {
            remaining_bytes: MAX_APPLICATION_ICON_SNAPSHOT_BYTES,
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl ApplicationIconBudget {
    pub fn encode_png(&mut self, png: &[u8]) -> Option<String> {
        use base64::Engine as _;
        const PREFIX: &str = "data:image/png;base64,";
        if png.len() > MAX_APPLICATION_ICON_PNG_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return None;
        }
        let encoded_len = PREFIX.len() + png.len().div_ceil(3) * 4;
        if encoded_len > self.remaining_bytes {
            return None;
        }
        let data_url = format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        self.remaining_bytes -= data_url.len();
        Some(data_url)
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationSnapshot {
    pub supported: bool,
    pub applications: Vec<AudioApplication>,
}

pub fn supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        super::windows_application::supported()
    }
    #[cfg(not(target_os = "windows"))]
    {
        cfg!(target_os = "macos")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub fn sort_applications(applications: &mut Vec<AudioApplication>) {
    applications.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen = std::collections::HashSet::new();
    applications.retain(|app| seen.insert(app.id.clone()));
    applications.truncate(512);
}

#[cfg(target_os = "windows")]
pub fn windows_applications() -> Result<ApplicationSnapshot, SystemAudioCaptureError> {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    if !supported() {
        return Ok(ApplicationSnapshot {
            supported: false,
            applications: vec![],
        });
    }
    let mut applications = Vec::<AudioApplication>::new();
    // EnumWindows is synchronous; the borrowed vector lives through every callback.
    unsafe {
        EnumWindows(
            Some(enumerate_window),
            windows::Win32::Foundation::LPARAM(&mut applications as *mut _ as isize),
        )
    }
    .map_err(|_| SystemAudioCaptureError::ApplicationListFailed)?;
    // A voice/music app may keep playing after its window closes to the tray.
    // Include its existing audio session as well as visible application windows.
    for session in super::census::census().sessions {
        if session.pid == std::process::id() || applications.len() >= 512 {
            continue;
        }
        if let Ok(process) = Process::open(session.pid) {
            if let Ok((id, name)) = process.identity() {
                if !applications.iter().any(|app| app.id == id) {
                    applications.push(AudioApplication {
                        id,
                        name,
                        icon_data_url: None,
                    });
                }
            }
        }
    }
    sort_applications(&mut applications);
    super::windows_application_icons::populate(&mut applications);
    Ok(ApplicationSnapshot {
        supported: true,
        applications,
    })
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enumerate_window(
    window: windows::Win32::Foundation::HWND,
    data: windows::Win32::Foundation::LPARAM,
) -> windows::core::BOOL {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    };
    if !unsafe { IsWindowVisible(window) }.as_bool() || unsafe { GetWindowTextLengthW(window) } == 0
    {
        return true.into();
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    if pid == std::process::id() {
        return true.into();
    }
    if let Ok(process) = Process::open(pid) {
        if let Ok((id, name)) = process.identity() {
            let applications = unsafe { &mut *(data.0 as *mut Vec<AudioApplication>) };
            if !applications.iter().any(|app| app.id == id) && applications.len() < 512 {
                applications.push(AudioApplication {
                    id,
                    name,
                    icon_data_url: None,
                });
            }
        }
    }
    true.into()
}

#[cfg(target_os = "windows")]
pub(super) struct Process {
    pub handle: windows::Win32::Foundation::HANDLE,
    pub pid: u32,
}

#[cfg(target_os = "windows")]
impl Process {
    pub fn open(pid: u32) -> windows::core::Result<Self> {
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )?
        };
        Ok(Self { handle, pid })
    }
    pub fn identity(&self) -> windows::core::Result<(String, String)> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetProcessTimes;
        let mut created = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(self.handle, &mut created, &mut exit, &mut kernel, &mut user)?;
        }
        let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let path = self.image_path()?;
        let path = String::from_utf16_lossy(&path);
        let name = path
            .rsplit('\\')
            .next()
            .unwrap_or("Application")
            .trim_end_matches(".exe")
            .to_string();
        Ok((format!("windows:{}:{}", self.pid, created), name))
    }

    pub(super) fn image_path(&self) -> windows::core::Result<Vec<u16>> {
        use windows::Win32::System::Threading::{QueryFullProcessImageNameW, PROCESS_NAME_WIN32};
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                self.handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(path.as_mut_ptr()),
                &mut length,
            )?;
        }
        path.truncate(length as usize);
        Ok(path)
    }
    pub fn selected(id: &str) -> Result<Self, SystemAudioCaptureError> {
        let pid = id
            .strip_prefix("windows:")
            .and_then(|text| text.split(':').next())
            .and_then(|pid| pid.parse::<u32>().ok())
            .filter(|pid| *pid != std::process::id())
            .ok_or(SystemAudioCaptureError::ApplicationUnavailable)?;
        let process =
            Self::open(pid).map_err(|_| SystemAudioCaptureError::ApplicationUnavailable)?;
        if process
            .identity()
            .map_err(|_| SystemAudioCaptureError::ApplicationUnavailable)?
            .0
            != id
            || !process.running()
        {
            return Err(SystemAudioCaptureError::ApplicationUnavailable);
        }
        Ok(process)
    }
    pub fn running(&self) -> bool {
        unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(self.handle, 0)
                == windows::Win32::Foundation::WAIT_TIMEOUT
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for Process {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_icons_are_optional_camel_case_presentation_metadata() {
        let mut app = AudioApplication {
            id: "test.player".into(),
            name: "Player".into(),
            icon_data_url: None,
        };
        assert_eq!(
            serde_json::to_value(&app).unwrap(),
            serde_json::json!({
                "id": "test.player", "name": "Player"
            })
        );
        app.icon_data_url = Some("data:image/png;base64,test".into());
        let value = serde_json::to_value(&app).unwrap();
        assert_eq!(value["iconDataUrl"], "data:image/png;base64,test");
        assert!(value.get("icon_data_url").is_none());
    }

    #[test]
    fn application_icon_encoding_rejects_invalid_or_oversized_png_without_spending_budget() {
        use base64::Engine as _;
        let mut budget = ApplicationIconBudget::default();
        let initial = budget.remaining_bytes;
        assert!(budget.encode_png(b"").is_none());
        assert!(budget.encode_png(b"<svg></svg>").is_none());
        let mut oversized = vec![0; MAX_APPLICATION_ICON_PNG_BYTES + 1];
        oversized[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert!(budget.encode_png(&oversized).is_none());
        assert_eq!(budget.remaining_bytes, initial);
        let png = include_bytes!("../../icons/32x32.png");
        let url = budget.encode_png(png).unwrap();
        let encoded = url.strip_prefix("data:image/png;base64,").unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
            png
        );
        assert_eq!(budget.remaining_bytes, initial - url.len());
    }

    #[test]
    fn application_icon_snapshot_has_a_total_budget_and_allows_smaller_icons_after_a_skip() {
        let mut png = vec![0; MAX_APPLICATION_ICON_PNG_BYTES];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut budget = ApplicationIconBudget::default();
        let mut total = 0;
        while let Some(url) = budget.encode_png(&png) {
            total += url.len();
        }
        assert!(total <= MAX_APPLICATION_ICON_SNAPSHOT_BYTES);
        assert!(total + 4 * png.len().div_ceil(3) > MAX_APPLICATION_ICON_SNAPSHOT_BYTES);
        let remaining = budget.remaining_bytes;
        assert!(budget.encode_png(&png).is_none());
        assert_eq!(budget.remaining_bytes, remaining);
        assert!(budget.encode_png(b"\x89PNG\r\n\x1a\n").is_some());
    }

    #[test]
    fn picker_sorting_is_deterministic_and_bounded() {
        let mut apps: Vec<_> = (0..600)
            .map(|i| AudioApplication {
                id: format!("app.{i}"),
                name: format!("App {i:03}"),
                icon_data_url: None,
            })
            .collect();
        apps.reverse();
        sort_applications(&mut apps);
        assert_eq!(apps.len(), 512);
        assert_eq!(apps[0].name, "App 000");
    }
}
