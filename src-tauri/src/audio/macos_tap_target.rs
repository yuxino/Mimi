//! Bounded, transient application ownership for a private Core Audio tap.
//! Process metadata stays local and never enters diagnostics or persisted settings.
use super::macos_tap::property;
use super::SystemAudioCaptureError;
use crate::core::system_audio_target::{
    application_owns_audio_process, application_owns_responsible_audio_process,
};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use objc2_core_audio::*;
use objc2_core_foundation::{CFRetained, CFString};
use objc2_foundation::NSString;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::OnceLock;

const MAX_AUDIO_PROCESSES: usize = 4096;

pub(super) struct ApplicationTapTarget {
    id: String,
    apps: Vec<Retained<NSRunningApplication>>,
    paths: Vec<PathBuf>,
}

impl ApplicationTapTarget {
    pub(super) fn new(id: &str) -> Result<Self, SystemAudioCaptureError> {
        let own = NSRunningApplication::currentApplication().bundleIdentifier();
        if own.as_ref().is_some_and(|own| own.to_string() == id) {
            return Err(SystemAudioCaptureError::ApplicationUnavailable);
        }
        let apps: Vec<_> =
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(id))
                .into_iter()
                .filter(|app| !app.isTerminated())
                .collect();
        if apps.is_empty() {
            return Err(SystemAudioCaptureError::ApplicationUnavailable);
        }
        let paths = apps
            .iter()
            .filter_map(|app| {
                app.bundleURL()?
                    .path()
                    .map(|path| PathBuf::from(path.to_string()))
            })
            .collect();
        Ok(Self {
            id: id.into(),
            apps,
            paths,
        })
    }

    pub(super) fn is_running(&self) -> bool {
        self.apps.iter().any(|app| !app.isTerminated())
    }

    pub(super) fn processes(&self) -> Result<Vec<AudioObjectID>, SystemAudioCaptureError> {
        if !self.is_running() {
            return Err(SystemAudioCaptureError::ApplicationUnavailable);
        }
        let own_pid = std::process::id() as i32;
        // Use only still-live original app instances, never a stale PID.
        let pids: Vec<_> = self
            .apps
            .iter()
            .filter(|app| !app.isTerminated())
            .map(|app| app.processIdentifier())
            .collect();
        let mut selected = Vec::new();
        for object in process_list()? {
            let Ok(pid) = property::<i32>(
                object,
                kAudioProcessPropertyPID,
                kAudioObjectPropertyScopeGlobal,
                None,
            ) else {
                continue;
            };
            let bundle = process_bundle_id(object);
            let path = process_path(pid);
            if application_owns_audio_process(
                own_pid,
                pid,
                bundle.as_deref(),
                &self.id,
                &pids,
                &self.paths,
                path.as_deref(),
            ) || application_owns_responsible_audio_process(
                own_pid,
                pid,
                responsible_pid(pid),
                &pids,
            ) {
                selected.push(object);
            }
        }
        selected.sort_unstable();
        selected.dedup();
        // Empty means silence, never a global tap. A quiet app may create its
        // first audio process later; the worker refreshes this allowlist.
        Ok(selected)
    }
}

fn process_list() -> Result<Vec<AudioObjectID>, SystemAudioCaptureError> {
    let mut address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyProcessObjectList,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut size = 0;
    super::macos_tap::status(unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    })?;
    if size as usize > MAX_AUDIO_PROCESSES * std::mem::size_of::<AudioObjectID>()
        || !(size as usize).is_multiple_of(std::mem::size_of::<AudioObjectID>())
    {
        return Err(SystemAudioCaptureError::NativeStartFailed);
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut objects = vec![0; size as usize / std::mem::size_of::<AudioObjectID>()];
    let capacity = size;
    super::macos_tap::status(unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(objects.as_mut_ptr()).unwrap().cast(),
        )
    })?;
    if size > capacity || !(size as usize).is_multiple_of(std::mem::size_of::<AudioObjectID>()) {
        return Err(SystemAudioCaptureError::NativeStartFailed);
    }
    objects.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
    Ok(objects)
}

fn process_bundle_id(object: AudioObjectID) -> Option<String> {
    // This property returns a +1 CFString, not a borrowed Objective-C value.
    let raw: *mut CFString = property(
        object,
        kAudioProcessPropertyBundleID,
        kAudioObjectPropertyScopeGlobal,
        None,
    )
    .ok()?;
    let value = unsafe { CFRetained::from_raw(NonNull::new(raw)?) };
    Some(value.to_string())
}

fn process_path(pid: i32) -> Option<PathBuf> {
    let mut bytes = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let length = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if length <= 0 {
        return None;
    }
    let path = std::ffi::CStr::from_bytes_until_nul(&bytes)
        .ok()?
        .to_str()
        .ok()?;
    Some(PathBuf::from(path))
}

// WebKit XPC audio helpers live outside the host bundle and are parented to
// launchd. macOS responsibility supplies their actual host, not that parent.
// This is an undocumented libSystem SPI (also used by Chromium); resolve it
// optionally instead of introducing a hard load-time dependency on old macOS.
// Missing symbols, failed lookups, and unrelated hosts never widen the tap.
fn responsible_pid(pid: i32) -> Option<i32> {
    type ResponsiblePid = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
    static API: OnceLock<Option<ResponsiblePid>> = OnceLock::new();
    if pid <= 0 {
        return None;
    }
    let api = API.get_or_init(|| {
        // SAFETY: ABI matches libSystem's pid_t-to-pid_t entry point, as used
        // in Chromium base/process/process_info_mac.mm. No retained object
        // or process handle crosses this call.
        let symbol = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"responsibility_get_pid_responsible_for_pid".as_ptr(),
            )
        };
        if symbol.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut libc::c_void, ResponsiblePid>(symbol) })
        }
    });
    let responsible = unsafe { api.as_ref()?(pid) };
    (responsible > 0).then_some(responsible)
}
