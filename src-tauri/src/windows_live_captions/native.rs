//! All COM interfaces stay on the owning MTA thread. The async side receives
//! only bounded caption snapshots and content-free error labels.

use super::{is_system_image, snapshot_text, Support};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_TIMEOUT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::SystemInformation::{GetSystemWindowsDirectoryW, OSVERSIONINFOW};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationElement, TreeScope_Descendants,
    UIA_AutomationIdPropertyId,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, SW_SHOWNORMAL,
};

const MIN_BUILD: u32 = 22621;
const WINDOW_CLASS: &str = "LiveCaptionsDesktopWindow";
const CAPTION_ID: &str = "CaptionsTextBlock";
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const SETUP_TIMEOUT: Duration = Duration::from_secs(6);
const UIA_TIMEOUT_MS: u32 = 750;
const NODE_RETRIES: usize = 3;
// Only one reader may be live, including a reader still cancelling a COM call.
static READER_ACTIVE: AtomicBool = AtomicBool::new(false);
// A provider that exceeds its COM timeout must not accumulate probe threads.
static SUPPORT_ACTIVE: AtomicBool = AtomicBool::new(false);

type Failure = Arc<Mutex<Option<String>>>;

pub struct CaptionSession {
    receiver: mpsc::Receiver<String>,
    worker: Thread,
    cancelled: Arc<AtomicBool>,
    failure: Failure,
}

impl CaptionSession {
    /// The first snapshot is a baseline, including an empty baseline. It stays
    /// queued until received so callers can discard history before translating.
    pub async fn recv(&mut self) -> Result<Option<String>, String> {
        if self.cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        if let Some(snapshot) = self.receiver.recv().await {
            return Ok(Some(snapshot));
        }
        let error = self
            .failure
            .lock()
            .map_err(|_| "windows_live_captions_unreadable".to_owned())?
            .take();
        error.map_or(Ok(None), Err)
    }

    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.receiver.close();
        self.worker.unpark();
        while self.receiver.try_recv().is_ok() {}
        if let Ok(mut failure) = self.failure.lock() {
            failure.take();
        }
    }
}

impl Drop for CaptionSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct CancelOnDrop {
    cancelled: Option<Arc<AtomicBool>>,
    worker: Option<Thread>,
}
impl CancelOnDrop {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled: Some(cancelled),
            worker: None,
        }
    }

    fn disarm(&mut self) {
        self.cancelled.take();
    }
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::Release);
            if let Some(worker) = &self.worker {
                worker.unpark();
            }
        }
    }
}

struct ActiveWorker(&'static AtomicBool);
impl ActiveWorker {
    fn acquire(active: &'static AtomicBool) -> Option<Self> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(active))
    }
}
impl Drop for ActiveWorker {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct Com;
impl Com {
    fn initialize() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|_| "windows_live_captions_unreadable".to_owned())?;
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct ProcessHandle(HANDLE);
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn build_number() -> Option<u32> {
    // Unlike GetVersionEx this is independent of application manifest shims.
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
    }
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    (unsafe { RtlGetVersion(&mut version) } == 0).then_some(version.dwBuildNumber)
}

fn executable() -> Result<PathBuf, String> {
    // Resolve the operating system's root through Win32, never an inherited
    // SystemRoot/PATH value or a similarly named program in a user directory.
    let mut buffer = [0u16; 32768];
    let length = unsafe { GetSystemWindowsDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("windows_live_captions_unsupported".into());
    }
    let root = String::from_utf16(&buffer[..length])
        .map_err(|_| "windows_live_captions_unsupported".to_owned())?;
    let path = PathBuf::from(root)
        .join("System32")
        .join("LiveCaptions.exe");
    if !path.is_file() {
        return Err("windows_live_captions_unsupported".into());
    }
    Ok(path)
}

pub fn open() -> Result<(), String> {
    if build_number().is_none_or(|build| build < MIN_BUILD) {
        return Err("windows_live_captions_unsupported".into());
    }
    let executable = executable()?;
    if trusted_window(&executable)?.is_some() {
        return Ok(());
    }
    // Windows owns initial setup. Never kill, hide, move, minimize, or replace
    // an existing Live Captions instance.
    let wide: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        Err("windows_live_captions_open_failed".into())
    } else {
        Ok(())
    }
}

pub async fn support() -> Support {
    let build_number = build_number();
    if build_number.is_none_or(|build| build < MIN_BUILD) || executable().is_err() {
        return Support {
            available: false,
            status: "unsupported".into(),
            build_number,
        };
    }
    let Some(active) = ActiveWorker::acquire(&SUPPORT_ACTIVE) else {
        return Support {
            available: true,
            status: "unreadable".into(),
            build_number,
        };
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut cancellation_guard = CancelOnDrop::new(cancelled.clone());
    let (sender, receiver) = oneshot::channel();
    // A UIA provider can time out; keep it away from the runtime and GUI thread.
    let spawned = thread::Builder::new()
        .name("mimi-live-captions-support".into())
        .spawn(move || {
            let _active = active;
            let status = inspect_status(&cancelled).unwrap_or("unreadable");
            let _ = sender.send(status);
        });
    let status = match spawned {
        Ok(handle) => {
            cancellation_guard.worker = Some(handle.thread().clone());
            drop(handle);
            match tokio::time::timeout(SETUP_TIMEOUT, receiver).await {
                Ok(Ok(status)) => status,
                _ => "unreadable",
            }
        }
        Err(_) => "unreadable",
    };
    Support {
        available: true,
        status: status.into(),
        build_number,
    }
}

fn inspect_status(cancelled: &AtomicBool) -> Result<&'static str, String> {
    ensure_active(cancelled)?;
    let executable = executable()?;
    let Some(window) = trusted_window(&executable)? else {
        return Ok("closed");
    };
    let _com = Com::initialize()?;
    let automation = automation()?;
    // This only locates the exact node and verifies identity. Do not read its
    // caption Name while the user is merely viewing settings.
    Ok(
        if find_caption(&automation, &window, cancelled)?.is_some() {
            "ready"
        } else {
            "setupRequired"
        },
    )
}

pub async fn start() -> Result<CaptionSession, String> {
    if build_number().is_none_or(|build| build < MIN_BUILD) {
        return Err("windows_live_captions_unsupported".into());
    }
    let executable = executable()?;
    let deadline = Instant::now() + SETUP_TIMEOUT;
    let active = loop {
        if let Some(active) = ActiveWorker::acquire(&READER_ACTIVE) {
            break active;
        }
        if Instant::now() >= deadline {
            return Err("windows_live_captions_reader_busy".into());
        }
        // A previous session may still be unwinding one short COM transaction.
        // Never spawn its replacement until that worker releases its slot.
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    if Instant::now() >= deadline {
        return Err("windows_live_captions_timeout".into());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut cancellation_guard = CancelOnDrop::new(cancelled.clone());
    let worker_cancelled = cancelled.clone();
    let failure = Arc::new(Mutex::new(None));
    let worker_failure = failure.clone();
    let (sender, receiver) = mpsc::channel(2);
    let (ready_sender, ready_receiver) = oneshot::channel();
    let handle = thread::Builder::new()
        .name("mimi-live-captions".into())
        .spawn(move || {
            let _active = active;
            run(
                executable,
                worker_cancelled,
                sender,
                ready_sender,
                worker_failure,
            );
        })
        .map_err(|_| "windows_live_captions_unreadable".to_owned())?;
    let worker = handle.thread().clone();
    cancellation_guard.worker = Some(worker.clone());
    // Detach instead of joining on an async or GUI thread. COM calls have short
    // timeouts, and dropping either setup or the session cancels the worker.
    drop(handle);
    match tokio::time::timeout(
        deadline.saturating_duration_since(Instant::now()),
        ready_receiver,
    )
    .await
    {
        Ok(Ok(Ok(()))) => {
            cancellation_guard.disarm();
            Ok(CaptionSession {
                receiver,
                worker,
                cancelled,
                failure,
            })
        }
        Ok(Ok(Err(error))) => Err(error),
        _ => Err("windows_live_captions_timeout".into()),
    }
}

fn run(
    executable: PathBuf,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::Sender<String>,
    ready: oneshot::Sender<Result<(), String>>,
    failure: Failure,
) {
    let setup = (|| {
        ensure_active(&cancelled)?;
        let com = Com::initialize()?;
        let automation = automation()?;
        let window = trusted_window(&executable)?
            .ok_or_else(|| "windows_live_captions_closed".to_owned())?;
        let caption = find_caption(&automation, &window, &cancelled)?
            .ok_or_else(|| "windows_live_captions_setup_required".to_owned())?;
        Ok::<_, String>((com, automation, window, caption))
    })();
    let (_com, automation, window, caption) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut caption = Some(caption);
    let baseline = match read_with_recovery(&automation, &window, &mut caption, &cancelled) {
        Ok(baseline) => baseline,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if cancelled.load(Ordering::Acquire) || sender.try_send(baseline.clone()).is_err() {
        return;
    }
    if ready.send(Ok(())).is_err() {
        return;
    }
    let mut last_sent = baseline;
    while !cancelled.load(Ordering::Acquire) {
        thread::park_timeout(POLL_INTERVAL);
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        match read_with_recovery(&automation, &window, &mut caption, &cancelled) {
            Ok(snapshot) if snapshot != last_sent => {
                // When translation is busy, retain the baseline and one pending
                // snapshot; retry the newest screen state after the queue drains.
                match sender.try_send(snapshot.clone()) {
                    Ok(()) => last_sent = snapshot,
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
            Ok(_) => {}
            Err(error) => {
                // Store one content-free terminal error outside the text queue
                // so a slow consumer can never keep a failed reader alive.
                if !cancelled.load(Ordering::Acquire) {
                    if let Ok(mut failure) = failure.lock() {
                        *failure = Some(error);
                    }
                }
                return;
            }
        }
    }
}

fn automation() -> Result<IUIAutomation2, String> {
    let automation: IUIAutomation2 =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| "windows_live_captions_unreadable".to_owned())?;
    let configure = || -> windows::core::Result<()> {
        unsafe {
            automation.SetAutoSetFocus(false)?;
            automation.SetConnectionTimeout(UIA_TIMEOUT_MS)?;
            automation.SetTransactionTimeout(UIA_TIMEOUT_MS)
        }
    };
    configure().map_err(|_| "windows_live_captions_unreadable".to_owned())?;
    Ok(automation)
}

struct TrustedWindow {
    hwnd: HWND,
    pid: u32,
    // Hold the original process identity without mutation rights. Check that it
    // has not exited as well as the HWND's owner before each content read.
    process: ProcessHandle,
}

fn trusted_window(executable: &std::path::Path) -> Result<Option<TrustedWindow>, String> {
    struct Search {
        candidates: Vec<HWND>,
    }
    unsafe extern "system" fn enumerate(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = unsafe { &mut *(data.0 as *mut Search) };
        if window_class_matches(hwnd) && search.candidates.len() < 16 {
            search.candidates.push(hwnd);
        }
        BOOL(1)
    }
    let mut search = Search {
        candidates: Vec::new(),
    };
    // Only class names are inspected here; no titles or text from other windows.
    unsafe { EnumWindows(Some(enumerate), LPARAM(&mut search as *mut _ as isize)) }
        .map_err(|_| "windows_live_captions_unreadable".to_owned())?;
    for hwnd in search.candidates {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let Ok(handle) = (unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }) else {
            continue;
        };
        let process = ProcessHandle(handle);
        let mut path = [0u16; 32768];
        let mut length = path.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(
                process.0,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut length,
            )
        }
        .is_err()
        {
            continue;
        }
        if length as usize >= path.len() {
            continue;
        }
        let Ok(path) = String::from_utf16(&path[..length as usize]) else {
            continue;
        };
        if is_system_image(&path, &executable.to_string_lossy()) {
            let window = TrustedWindow { hwnd, pid, process };
            if ensure_current_window(&window).is_ok() {
                return Ok(Some(window));
            }
        }
    }
    Ok(None)
}

fn window_class_matches(hwnd: HWND) -> bool {
    let mut class = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut class) };
    length > 0 && String::from_utf16_lossy(&class[..length as usize]).as_str() == WINDOW_CLASS
}

fn find_caption(
    automation: &IUIAutomation2,
    window: &TrustedWindow,
    cancelled: &AtomicBool,
) -> Result<Option<IUIAutomationElement>, String> {
    ensure_current_window(window)?;
    let find = || -> windows::core::Result<Option<IUIAutomationElement>> {
        check_cancelled(cancelled)?;
        let root = unsafe { automation.ElementFromHandle(window.hwnd) }?;
        check_cancelled(cancelled)?;
        let pid = unsafe { root.CurrentProcessId() }?;
        check_cancelled(cancelled)?;
        let class = unsafe { root.CurrentClassName() }?;
        if pid != window.pid as i32 || class != WINDOW_CLASS {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004005u32 as i32,
            )));
        }
        check_cancelled(cancelled)?;
        let condition = unsafe {
            automation
                .CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(CAPTION_ID))
        }?;
        check_cancelled(cancelled)?;
        let matches = unsafe { root.FindAll(TreeScope_Descendants, &condition) }?;
        check_cancelled(cancelled)?;
        match unsafe { matches.Length() }? {
            0 => Ok(None),
            1 => {
                let caption = unsafe { matches.GetElement(0) }?;
                check_cancelled(cancelled)?;
                if unsafe { caption.CurrentProcessId() }? != window.pid as i32 {
                    return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                        0x80004005u32 as i32,
                    )));
                }
                Ok(Some(caption))
            }
            _ => Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004005u32 as i32,
            ))),
        }
    };
    let caption = find().map_err(|_| "windows_live_captions_unreadable".to_owned())?;
    ensure_active(cancelled)?;
    ensure_current_window(window)?;
    Ok(caption)
}

fn ensure_current_window(window: &TrustedWindow) -> Result<(), String> {
    if unsafe { WaitForSingleObject(window.process.0, 0) } != WAIT_TIMEOUT {
        return Err("windows_live_captions_closed".into());
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window.hwnd, Some(&mut pid)) };
    if pid != window.pid || !window_class_matches(window.hwnd) {
        return Err("windows_live_captions_closed".into());
    }
    Ok(())
}

fn ensure_active(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("windows_live_captions_cancelled".into())
    } else {
        Ok(())
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> windows::core::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004004u32 as i32,
        )))
    } else {
        Ok(())
    }
}

fn read_with_recovery(
    automation: &IUIAutomation2,
    window: &TrustedWindow,
    caption: &mut Option<IUIAutomationElement>,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    for attempt in 0..NODE_RETRIES {
        ensure_active(cancelled)?;
        ensure_current_window(window)?;
        if attempt != 0 {
            thread::park_timeout(POLL_INTERVAL);
            ensure_active(cancelled)?;
        }
        if caption.is_none() {
            *caption = match find_caption(automation, window, cancelled) {
                Ok(caption) => caption,
                Err(error) if error == "windows_live_captions_closed" => return Err(error),
                Err(_) => continue,
            };
        }
        if let Some(element) = caption {
            match read_caption(window, element, cancelled) {
                Ok(snapshot) => return Ok(snapshot),
                Err(error) if error == "windows_live_captions_closed" => return Err(error),
                Err(_) => {
                    // A language change or XAML rebuild may replace just the
                    // node. Requery only inside the original trusted process.
                    caption.take();
                }
            }
        }
    }
    ensure_active(cancelled)?;
    Err("windows_live_captions_unreadable".into())
}

fn read_caption(
    window: &TrustedWindow,
    caption: &IUIAutomationElement,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    ensure_active(cancelled)?;
    ensure_current_window(window)?;
    let read = || -> windows::core::Result<String> {
        check_cancelled(cancelled)?;
        let pid = unsafe { caption.CurrentProcessId() }?;
        check_cancelled(cancelled)?;
        let id = unsafe { caption.CurrentAutomationId() }?;
        if pid != window.pid as i32 || id != CAPTION_ID {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004005u32 as i32,
            )));
        }
        check_cancelled(cancelled)?;
        let text = unsafe { caption.CurrentName() }?;
        // The trusted OS provider should expose only its bounded screen buffer.
        // Refuse unexpected growth instead of retaining a full session history.
        snapshot_text(&text).map_err(|_| {
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005u32 as i32))
        })
    };
    let snapshot = read().map_err(|_| "windows_live_captions_unreadable".to_owned())?;
    ensure_active(cancelled)?;
    ensure_current_window(window)?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_slot_is_released_on_drop_and_remains_exclusive() {
        static ACTIVE: AtomicBool = AtomicBool::new(false);
        let worker = ActiveWorker::acquire(&ACTIVE).unwrap();
        assert!(ActiveWorker::acquire(&ACTIVE).is_none());
        drop(worker);
        assert!(ActiveWorker::acquire(&ACTIVE).is_some());
    }

    #[test]
    fn setup_guard_cancels_when_dropped_but_can_transfer_ownership() {
        let cancelled = Arc::new(AtomicBool::new(false));
        drop(CancelOnDrop::new(cancelled.clone()));
        assert!(cancelled.load(Ordering::Acquire));
        cancelled.store(false, Ordering::Release);
        let mut guard = CancelOnDrop::new(cancelled.clone());
        guard.disarm();
        drop(guard);
        assert!(!cancelled.load(Ordering::Acquire));
        assert_eq!(Arc::strong_count(&cancelled), 1);
    }
}
