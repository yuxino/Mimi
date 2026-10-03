//! Local app picker data. Names and identities must never enter diagnostics.
#[cfg(target_os = "windows")]
use super::SystemAudioCaptureError;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioApplication {
    pub id: String,
    pub name: String,
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
                    applications.push(AudioApplication { id, name });
                }
            }
        }
    }
    sort_applications(&mut applications);
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
                applications.push(AudioApplication { id, name });
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
        use windows::Win32::System::Threading::{
            GetProcessTimes, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        };
        let mut created = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let mut path = [0u16; 32768];
        let mut length = path.len() as u32;
        unsafe {
            GetProcessTimes(self.handle, &mut created, &mut exit, &mut kernel, &mut user)?;
            QueryFullProcessImageNameW(
                self.handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(path.as_mut_ptr()),
                &mut length,
            )?;
        }
        let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        let path = String::from_utf16_lossy(&path[..length as usize]);
        let name = path
            .rsplit('\\')
            .next()
            .unwrap_or("Application")
            .trim_end_matches(".exe")
            .to_string();
        Ok((format!("windows:{}:{}", self.pid, created), name))
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
    fn picker_sorting_is_deterministic_and_bounded() {
        let mut apps: Vec<_> = (0..600)
            .map(|i| AudioApplication {
                id: format!("app.{i}"),
                name: format!("App {i:03}"),
            })
            .collect();
        apps.reverse();
        sort_applications(&mut apps);
        assert_eq!(apps.len(), 512);
        assert_eq!(apps[0].name, "App 000");
    }
}
