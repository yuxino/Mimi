//! Windows-only startup serialization and content-free single-instance activation.
//!
//! The outer mutex closes the cold-start gap before the Tauri plugin is set up.
//! The local plugin intentionally does not use `WM_COPYDATA`: a secondary sends
//! one registered, payload-free message only after it has verified that exactly
//! one listener exists and that the listener process is running the same file.

use std::ffi::OsStr;
use std::iter::once;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, Manager, RunEvent, Wry};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HINSTANCE, HWND, INVALID_HANDLE_VALUE,
    LPARAM, LRESULT, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileIdInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_NORMAL, FILE_ID_INFO,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    ReleaseMutex, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW,
    GetWindowThreadProcessId, RegisterClassExW, RegisterWindowMessageW, SendMessageTimeoutW,
    UnregisterClassW, SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT, WNDCLASSEXW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

const STARTUP_GATE_WAIT_MS: u32 = 15_000;
const ACTIVATION_TIMEOUT_MS: u32 = 3_000;
const ACTIVATION_ACK: usize = 1;
const MAX_UI_TEST_DELAY_MS: u64 = 5_000;
const MAX_PROCESS_IMAGE_PATH_U16: usize = 32_768;

static ACTIVATION_MESSAGE: AtomicU32 = AtomicU32::new(0);
static LISTENER_APP: OnceLock<Mutex<Option<AppHandle<Wry>>>> = OnceLock::new();

/// Owns the startup mutex until the full Tauri setup has returned to the event
/// loop. Win32 mutex ownership is thread-affine, so this guard must be dropped
/// on the thread that acquired it.
pub struct StartupGate {
    // Store the opaque value rather than the raw-pointer HANDLE so this guard
    // can be captured by Tauri's `Send` setup closure.
    handle: isize,
    owner_thread_id: u32,
}

impl StartupGate {
    /// Exit a verified secondary before creating Tauri's runtime. Contenders
    /// are serialized by this guard, so initializing and cleaning up a whole
    /// runtime for each one can exhaust the cold-launch handoff deadline.
    pub fn handoff_if_running(&self, identifier: &str) -> Result<bool, &'static str> {
        let (mutex, already_exists) = open_instance_mutex(identifier)?;
        if !already_exists {
            // This is only a probe. The primary plugin will create and retain
            // its real mutex/listener while we still hold the startup gate.
            unsafe {
                ReleaseMutex(mutex);
                CloseHandle(mutex);
            }
            return Ok(false);
        }
        let result = register_activation_message(identifier).and_then(|message| {
            handoff_to_verified_listener(
                &wide_name(&format!("{identifier}-sic")),
                &wide_name(&format!("{identifier}-siw")),
                message,
                current_process_id(),
            )
        });
        // An existing mutex is not owned by this thread.
        unsafe { CloseHandle(mutex) };
        result.map(|()| true)
    }

    pub fn acquire(identifier: &str) -> Result<Self, &'static str> {
        let name = wide_name(&format!("{identifier}-startup-gate-v1"));

        // SAFETY: `name` is live and NUL-terminated. The null security
        // descriptor requests the caller's default object ACL.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Err("create_failed");
        }

        // SAFETY: read immediately after CreateMutexW, as required by Win32.
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already_exists {
            // SAFETY: `handle` is valid. Existing named mutexes ignore the
            // initial-owner argument, so wait until this thread owns it.
            match unsafe { WaitForSingleObject(handle, STARTUP_GATE_WAIT_MS) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => {}
                WAIT_TIMEOUT => {
                    // SAFETY: valid handle not owned by this thread.
                    unsafe { CloseHandle(handle) };
                    return Err("wait_timeout");
                }
                _ => {
                    // SAFETY: valid handle not known to be owned.
                    unsafe { CloseHandle(handle) };
                    return Err("wait_failed");
                }
            }
        } else if let Err(label) = prepare_primary_for_ui_test() {
            // SAFETY: a newly created mutex is owned by this thread.
            unsafe {
                ReleaseMutex(handle);
                CloseHandle(handle);
            }
            return Err(label);
        }

        Ok(Self {
            handle: handle as isize,
            // SAFETY: this function has no preconditions.
            owner_thread_id: unsafe { GetCurrentThreadId() },
        })
    }
}

impl Drop for StartupGate {
    fn drop(&mut self) {
        // Closing the only handle without releasing a thread-owned mutex would
        // strand waiters. Abort and let the kernel abandon it instead.
        if unsafe { GetCurrentThreadId() } != self.owner_thread_id {
            std::process::abort();
        }
        // SAFETY: every successful constructor path owns this valid handle.
        unsafe {
            let handle = self.handle as HANDLE;
            ReleaseMutex(handle);
            CloseHandle(handle);
        }
    }
}

struct LocalSingleInstanceState {
    mutex: isize,
    window: isize,
    class_name: Vec<u16>,
    module: isize,
    owner_thread_id: u32,
    cleaned: AtomicBool,
}

impl LocalSingleInstanceState {
    fn cleanup(&self) {
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }
        if unsafe { GetCurrentThreadId() } != self.owner_thread_id {
            std::process::abort();
        }

        // Keep the mutex owned until the listener is gone. A launch racing
        // shutdown therefore fails closed instead of becoming a second full
        // primary while the old activation target still exists.
        unsafe {
            DestroyWindow(self.window as HWND);
        }
        clear_listener_app();
        unsafe {
            UnregisterClassW(self.class_name.as_ptr(), self.module as HINSTANCE);
            ReleaseMutex(self.mutex as HANDLE);
            CloseHandle(self.mutex as HANDLE);
        }
    }
}

/// Local Windows implementation of Tauri's single-instance plugin contract.
/// It must remain the first registered plugin.
pub fn single_instance_plugin() -> TauriPlugin<Wry> {
    PluginBuilder::new("mimi-single-instance")
        .setup(|app, _api| {
            initialize_or_handoff(app, app.config().identifier.as_str()).map_err(|label| {
                std::io::Error::other(format!("mimi single-instance setup failed: {label}")).into()
            })
        })
        .on_event(|app, event| {
            if matches!(event, RunEvent::Exit) {
                if let Some(state) = app.try_state::<Arc<LocalSingleInstanceState>>() {
                    state.cleanup();
                }
            }
        })
        .build()
}

fn initialize_or_handoff(app: &AppHandle<Wry>, identifier: &str) -> Result<(), &'static str> {
    let activation_message = register_activation_message(identifier)?;
    let (mutex, already_exists) = open_instance_mutex(identifier)?;

    let class_name = wide_name(&format!("{identifier}-sic"));
    let window_name = wide_name(&format!("{identifier}-siw"));
    if already_exists {
        let handoff = handoff_to_verified_listener(
            &class_name,
            &window_name,
            activation_message,
            current_process_id(),
        );
        // SAFETY: CreateMutexW returned a valid handle. This process does not
        // own an already-existing mutex, so it must only close the handle.
        unsafe { CloseHandle(mutex) };
        handoff?;
        app.cleanup_before_exit();
        std::process::exit(0);
    }

    create_primary_listener(app, mutex, class_name, window_name)
}

fn register_activation_message(identifier: &str) -> Result<u32, &'static str> {
    let activation_name = wide_name(&format!("{identifier}-activation-v1"));
    // SAFETY: `activation_name` is live and NUL-terminated.
    let activation_message = unsafe { RegisterWindowMessageW(activation_name.as_ptr()) };
    if activation_message == 0 {
        return Err("activation_message_registration_failed");
    }
    ACTIVATION_MESSAGE.store(activation_message, Ordering::SeqCst);
    Ok(activation_message)
}

fn open_instance_mutex(identifier: &str) -> Result<(HANDLE, bool), &'static str> {
    let mutex_name = wide_name(&format!("{identifier}-sim"));
    // SAFETY: `mutex_name` is live and NUL-terminated.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) };
    if mutex.is_null() {
        // Wrong-type objects and access-denied existing mutexes both land here.
        return Err("single_instance_mutex_invalid");
    }
    // SAFETY: read immediately after CreateMutexW.
    Ok((mutex, unsafe { GetLastError() } == ERROR_ALREADY_EXISTS))
}

fn create_primary_listener(
    app: &AppHandle<Wry>,
    mutex: HANDLE,
    class_name: Vec<u16>,
    window_name: Vec<u16>,
) -> Result<(), &'static str> {
    if set_listener_app(app.clone()).is_err() {
        release_owned_mutex(mutex);
        return Err("listener_app_state_invalid");
    }

    // SAFETY: null requests the current executable module.
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    if module.is_null() {
        clear_listener_app();
        release_owned_mutex(mutex);
        return Err("listener_module_missing");
    }

    let window_class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(listener_window_proc),
        hInstance: module,
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    // SAFETY: all pointers in the class descriptor remain live for this call.
    if unsafe { RegisterClassExW(&window_class) } == 0 {
        clear_listener_app();
        release_owned_mutex(mutex);
        return Err("listener_class_registration_failed");
    }

    // SAFETY: registered class and both UTF-16 strings are live. The hidden
    // zero-sized top-level tool window has no payload or external pointers.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            module,
            std::ptr::null(),
        )
    };
    if window.is_null() {
        unsafe { UnregisterClassW(class_name.as_ptr(), module) };
        clear_listener_app();
        release_owned_mutex(mutex);
        return Err("listener_window_creation_failed");
    }

    let state = Arc::new(LocalSingleInstanceState {
        mutex: mutex as isize,
        window: window as isize,
        class_name,
        module: module as isize,
        owner_thread_id: unsafe { GetCurrentThreadId() },
        cleaned: AtomicBool::new(false),
    });

    if verify_primary_listener(&window_name, &state.class_name, window).is_err() {
        state.cleanup();
        return Err("listener_identity_invalid");
    }
    if !app.manage(Arc::clone(&state)) {
        state.cleanup();
        return Err("listener_state_registration_failed");
    }
    Ok(())
}

fn verify_primary_listener(
    window_name: &[u16],
    class_name: &[u16],
    expected_window: HWND,
) -> Result<(), &'static str> {
    let listener = find_unique_listener(class_name, window_name)?;
    if listener != expected_window {
        return Err("listener_handle_mismatch");
    }
    if window_process_id(listener)? != current_process_id() {
        return Err("listener_process_mismatch");
    }
    Ok(())
}

fn handoff_to_verified_listener(
    class_name: &[u16],
    window_name: &[u16],
    activation_message: u32,
    current_process_id: u32,
) -> Result<(), &'static str> {
    let listener = find_unique_listener(class_name, window_name)?;
    let owner_process_id = window_process_id(listener)?;
    let current_identity = process_image_identity(current_process_id)?;
    let owner_identity = process_image_identity(owner_process_id)?;
    if current_identity != owner_identity {
        return Err("listener_image_mismatch");
    }

    // A launch initiated by the user is allowed to transfer its foreground
    // activation right to the verified resident process. Without this grant,
    // Windows may restore Settings but reject its later SetForegroundWindow
    // call, especially when the resident process has been idle for a while.
    // This is best-effort because the handoff itself remains valid when the
    // caller was not foreground-eligible (for example, a background script).
    unsafe {
        AllowSetForegroundWindow(owner_process_id);
    }

    let mut result = 0usize;
    // SAFETY: `listener` was returned by FindWindowExW. No pointers or content
    // cross the process boundary; the bounded registered message has zero
    // parameters and the receiver returns a fixed acknowledgement.
    let sent = unsafe {
        SendMessageTimeoutW(
            listener,
            activation_message,
            0,
            0,
            SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
            ACTIVATION_TIMEOUT_MS,
            &mut result,
        )
    };
    if sent == 0 || result != ACTIVATION_ACK {
        return Err("listener_handoff_failed");
    }
    if window_process_id(listener)? != owner_process_id {
        return Err("listener_changed_during_handoff");
    }
    Ok(())
}

fn find_unique_listener(class_name: &[u16], window_name: &[u16]) -> Result<HWND, &'static str> {
    // SAFETY: both UTF-16 strings are live and NUL-terminated.
    let first = unsafe {
        FindWindowExW(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            class_name.as_ptr(),
            window_name.as_ptr(),
        )
    };
    if first.is_null() {
        return Err("single_instance_listener_missing");
    }
    // Reject any extra matching target. Sending directly to the validated HWND
    // then avoids a second name lookup and its spoofing race.
    let second = unsafe {
        FindWindowExW(
            std::ptr::null_mut(),
            first,
            class_name.as_ptr(),
            window_name.as_ptr(),
        )
    };
    if !second.is_null() {
        return Err("single_instance_listener_ambiguous");
    }
    Ok(first)
}

fn window_process_id(window: HWND) -> Result<u32, &'static str> {
    let mut process_id = 0u32;
    // SAFETY: callers pass a candidate HWND returned by Win32.
    if unsafe { GetWindowThreadProcessId(window, &mut process_id) } == 0 || process_id == 0 {
        return Err("listener_process_missing");
    }
    Ok(process_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

fn process_image_identity(process_id: u32) -> Result<FileIdentity, &'static str> {
    // SAFETY: process ID is obtained from Win32. No inheritable handle needed.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err("listener_process_open_failed");
    }

    let mut path = vec![0u16; MAX_PROCESS_IMAGE_PATH_U16];
    let mut path_len = path.len() as u32;
    // SAFETY: `path` has `path_len` writable UTF-16 elements.
    let queried =
        unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut path_len) };
    // SAFETY: `process` is a valid handle returned above.
    unsafe { CloseHandle(process) };
    if queried == 0 || path_len == 0 || path_len as usize >= path.len() {
        return Err("listener_process_image_query_failed");
    }
    path.truncate(path_len as usize);
    path.push(0);
    file_identity(&path)
}

fn file_identity(path: &[u16]) -> Result<FileIdentity, &'static str> {
    // SAFETY: `path` is live and NUL-terminated. Share delete/write so an
    // updater does not turn an otherwise queryable running image into a lock.
    let file = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if file == INVALID_HANDLE_VALUE {
        return Err("listener_image_open_failed");
    }

    let mut info = MaybeUninit::<FILE_ID_INFO>::uninit();
    // SAFETY: `info` has exactly the size requested for FileIdInfo.
    let queried = unsafe {
        GetFileInformationByHandleEx(
            file,
            FileIdInfo,
            info.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    // SAFETY: `file` is a valid handle returned above.
    unsafe { CloseHandle(file) };
    if queried == 0 {
        return Err("listener_image_identity_query_failed");
    }
    // SAFETY: GetFileInformationByHandleEx initialized the full structure.
    let info = unsafe { info.assume_init() };
    Ok(FileIdentity {
        volume_serial_number: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

unsafe extern "system" fn listener_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let activation_message = ACTIVATION_MESSAGE.load(Ordering::SeqCst);
    if activation_message != 0 && message == activation_message {
        let app = listener_app()
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned());
        if let Some(app) = app {
            crate::windows::ensure_settings_window(&app);
            return ACTIVATION_ACK as LRESULT;
        }
        return 0;
    }
    // SAFETY: forward every unrelated message to the system default handler.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn listener_app() -> &'static Mutex<Option<AppHandle<Wry>>> {
    LISTENER_APP.get_or_init(|| Mutex::new(None))
}

fn set_listener_app(app: AppHandle<Wry>) -> Result<(), ()> {
    let mut slot = listener_app().lock().map_err(|_| ())?;
    if slot.is_some() {
        return Err(());
    }
    *slot = Some(app);
    Ok(())
}

fn clear_listener_app() {
    if let Ok(mut slot) = listener_app().lock() {
        *slot = None;
    }
}

fn release_owned_mutex(mutex: HANDLE) {
    // SAFETY: only called on the thread that created and owns this mutex.
    unsafe {
        ReleaseMutex(mutex);
        CloseHandle(mutex);
    }
}

fn current_process_id() -> u32 {
    // SAFETY: this function has no preconditions.
    unsafe { GetCurrentProcessId() }
}

fn wide_name(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(once(0)).collect()
}

fn prepare_primary_for_ui_test() -> Result<(), &'static str> {
    if std::env::var("MIMI_UI_TEST").as_deref() != Ok("1") {
        return Ok(());
    }
    if let Some(path) = std::env::var_os("MIMI_UI_TEST_STARTUP_GATE_READY_FILE") {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "ui_test_startup_signal_failed")?;
    }
    let delay_ms = std::env::var("MIMI_UI_TEST_STARTUP_GATE_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default()
        .min(MAX_UI_TEST_DELAY_MS);
    if delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(delay_ms));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_primary_probe_does_not_leave_an_instance_mutex_behind() {
        let identifier = format!("mimi-test-{}", uuid::Uuid::new_v4());
        let gate = StartupGate::acquire(&identifier).unwrap();
        assert!(!gate.handoff_if_running(&identifier).unwrap());
        let (mutex, already_exists) = open_instance_mutex(&identifier).unwrap();
        unsafe {
            ReleaseMutex(mutex);
            CloseHandle(mutex);
        }
        assert!(!already_exists);
    }

    #[test]
    fn early_handoff_rejects_an_existing_mutex_without_a_listener() {
        let identifier = format!("mimi-test-{}", uuid::Uuid::new_v4());
        let gate = StartupGate::acquire(&identifier).unwrap();
        let (mutex, _) = open_instance_mutex(&identifier).unwrap();
        let result = gate.handoff_if_running(&identifier);
        unsafe {
            ReleaseMutex(mutex);
            CloseHandle(mutex);
        }
        assert_eq!(result, Err("single_instance_listener_missing"));
    }

    #[test]
    fn early_handoff_can_acknowledge_a_verified_listener_without_tauri() {
        unsafe extern "system" fn acknowledge(
            window: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if message >= 0xc000 {
                ACTIVATION_ACK as LRESULT
            } else {
                DefWindowProcW(window, message, wparam, lparam)
            }
        }
        let identifier = format!("mimi-test-{}", uuid::Uuid::new_v4());
        let gate = StartupGate::acquire(&identifier).unwrap();
        let class_name = wide_name(&format!("{identifier}-sic"));
        let window_name = wide_name(&format!("{identifier}-siw"));
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(acknowledge),
            hInstance: module,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        assert_ne!(unsafe { RegisterClassExW(&class) }, 0);
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            )
        };
        let (mutex, _) = open_instance_mutex(&identifier).unwrap();
        let result = if window.is_null() {
            Err("test_listener_creation_failed")
        } else {
            gate.handoff_if_running(&identifier)
        };
        unsafe {
            DestroyWindow(window);
            UnregisterClassW(class_name.as_ptr(), module);
            ReleaseMutex(mutex);
            CloseHandle(mutex);
        }
        assert_eq!(result, Ok(true));
    }

    #[test]
    fn current_process_image_has_stable_file_identity() {
        let process_id = current_process_id();
        let first = process_image_identity(process_id).unwrap();
        let second = process_image_identity(process_id).unwrap();
        assert_eq!(first, second);
    }
}
