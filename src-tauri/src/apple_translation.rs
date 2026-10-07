//! Apple's local text translation adapter. Checks and translation never request
//! downloads; only `prepare` may present Apple's language-download consent UI.

use serde::{Deserialize, Serialize};

pub(crate) const MAX_TEXT_BYTES: usize = 65_536;

pub(crate) fn supports_preparation_ui_language(language: &str) -> bool {
    matches!(language, "en" | "zh" | "zh-TW" | "ja" | "de" | "fr" | "ko")
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleTranslationCapabilities {
    pub available: bool,
    pub languages: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppleTranslationStatus {
    Installed,
    Supported,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(
    not(all(target_os = "macos", target_arch = "aarch64")),
    allow(dead_code)
)]
pub enum AppleTranslationError {
    #[error("apple_translation_unavailable")]
    Unavailable,
    #[error("apple_translation_assets_missing")]
    AssetsNotInstalled,
    #[error("apple_translation_language_unsupported")]
    UnsupportedLanguagePair,
    #[error("apple_translation_invalid_input")]
    InvalidInput,
    #[error("apple_translation_cancelled")]
    Cancelled,
    #[error("apple_translation_busy")]
    Busy,
    #[error("apple_translation_timeout")]
    Timeout,
    #[error("apple_translation_invalid_result")]
    InvalidResult,
    #[error("apple_translation_failed")]
    Native,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::{c_char, CString};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;
    use tokio::sync::{oneshot, Semaphore};

    const MAX_NATIVE_BYTES: usize = 262_144;
    type Callback = unsafe extern "C" fn(u64, *const u8, usize) -> i32;
    unsafe extern "C" {
        fn mimi_apple_translation_query(id: u64, callback: Callback);
        fn mimi_apple_translation_status(
            id: u64,
            source: *const c_char,
            target: *const c_char,
            callback: Callback,
        );
        fn mimi_apple_translation_prepare(
            id: u64,
            source: *const c_char,
            target: *const c_char,
            ui_language: *const c_char,
            callback: Callback,
        );
        fn mimi_apple_translation_translate(
            id: u64,
            source: *const c_char,
            target: *const c_char,
            text: *const u8,
            length: usize,
            callback: Callback,
        );
        fn mimi_apple_translation_cancel(id: u64);
    }

    #[derive(Deserialize)]
    #[serde(tag = "event", rename_all = "snake_case")]
    enum Response {
        Capabilities {
            available: bool,
            languages: Vec<String>,
        },
        Status {
            status: AppleTranslationStatus,
        },
        Prepared,
        Translation {
            text: String,
        },
        Error {
            code: i64,
        },
    }

    type Sender = oneshot::Sender<Result<Response, AppleTranslationError>>;
    fn routes() -> &'static Mutex<HashMap<u64, Sender>> {
        static ROUTES: OnceLock<Mutex<HashMap<u64, Sender>>> = OnceLock::new();
        ROUTES.get_or_init(Mutex::default)
    }

    fn remove(id: u64) -> Option<Sender> {
        routes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
    }

    struct RequestGuard(u64);
    impl Drop for RequestGuard {
        fn drop(&mut self) {
            // Remove the receiver before cancelling native work. Late callbacks
            // contain only an ID, never a pointer into a dropped Rust future.
            remove(self.0);
            unsafe { mimi_apple_translation_cancel(self.0) };
        }
    }

    fn error(code: i64) -> AppleTranslationError {
        match code {
            1 => AppleTranslationError::Unavailable,
            2 => AppleTranslationError::AssetsNotInstalled,
            3 => AppleTranslationError::UnsupportedLanguagePair,
            4 => AppleTranslationError::InvalidInput,
            5 => AppleTranslationError::Cancelled,
            6 => AppleTranslationError::Busy,
            7 => AppleTranslationError::InvalidResult,
            _ => AppleTranslationError::Native,
        }
    }

    fn parse(bytes: &[u8]) -> Result<Response, AppleTranslationError> {
        if bytes.is_empty() || bytes.len() > MAX_NATIVE_BYTES {
            return Err(AppleTranslationError::InvalidResult);
        }
        match serde_json::from_slice(bytes).map_err(|_| AppleTranslationError::InvalidResult)? {
            Response::Error { code } => Err(error(code)),
            Response::Translation { text }
                if text.len() > MAX_TEXT_BYTES || text.trim().is_empty() =>
            {
                Err(AppleTranslationError::InvalidResult)
            }
            Response::Capabilities {
                available,
                mut languages,
            } => {
                if languages.len() > 256 || languages.iter().any(|value| language(value).is_err()) {
                    return Err(AppleTranslationError::InvalidResult);
                }
                languages.sort();
                languages.dedup();
                Ok(Response::Capabilities {
                    available,
                    languages,
                })
            }
            value => Ok(value),
        }
    }

    unsafe extern "C" fn callback(id: u64, bytes: *const u8, length: usize) -> i32 {
        std::panic::catch_unwind(|| {
            let Some(sender) = remove(id) else { return 0 };
            let result = if bytes.is_null() || length == 0 || length > MAX_NATIVE_BYTES {
                Err(AppleTranslationError::InvalidResult)
            } else {
                // The native allocation is borrowed only until this callback
                // returns; serde creates owned strings before sending them.
                parse(unsafe { std::slice::from_raw_parts(bytes, length) })
            };
            i32::from(sender.send(result).is_ok())
        })
        .unwrap_or(0)
    }

    fn language(value: &str) -> Result<CString, AppleTranslationError> {
        if value.is_empty()
            || value.eq_ignore_ascii_case("auto")
            || value.len() > 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(AppleTranslationError::InvalidInput);
        }
        CString::new(value).map_err(|_| AppleTranslationError::InvalidInput)
    }

    async fn request(
        timeout: Duration,
        start: impl FnOnce(u64),
    ) -> Result<Response, AppleTranslationError> {
        static SLOTS: Semaphore = Semaphore::const_new(8);
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let _slot = SLOTS
            .try_acquire()
            .map_err(|_| AppleTranslationError::Busy)?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        routes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, sender);
        let _guard = RequestGuard(id);
        start(id);
        tokio::time::timeout(timeout, receiver)
            .await
            .map_err(|_| AppleTranslationError::Timeout)?
            .map_err(|_| AppleTranslationError::Cancelled)?
    }

    pub async fn capabilities() -> Result<AppleTranslationCapabilities, AppleTranslationError> {
        match request(Duration::from_secs(10), |id| unsafe {
            mimi_apple_translation_query(id, callback)
        })
        .await?
        {
            Response::Capabilities {
                available,
                languages,
            } => Ok(AppleTranslationCapabilities {
                available,
                languages,
            }),
            _ => Err(AppleTranslationError::InvalidResult),
        }
    }

    pub async fn status(
        source: &str,
        target: &str,
    ) -> Result<AppleTranslationStatus, AppleTranslationError> {
        let source = language(source)?;
        let target = language(target)?;
        match request(Duration::from_secs(10), |id| unsafe {
            mimi_apple_translation_status(id, source.as_ptr(), target.as_ptr(), callback)
        })
        .await?
        {
            Response::Status { status } => Ok(status),
            _ => Err(AppleTranslationError::InvalidResult),
        }
    }

    pub async fn prepare(
        source: &str,
        target: &str,
        ui_language: &str,
    ) -> Result<(), AppleTranslationError> {
        if !supports_preparation_ui_language(ui_language) {
            return Err(AppleTranslationError::InvalidInput);
        }
        let source = language(source)?;
        let target = language(target)?;
        let ui_language = language(ui_language)?;
        match request(Duration::from_secs(600), |id| unsafe {
            mimi_apple_translation_prepare(
                id,
                source.as_ptr(),
                target.as_ptr(),
                ui_language.as_ptr(),
                callback,
            )
        })
        .await?
        {
            Response::Prepared => Ok(()),
            _ => Err(AppleTranslationError::InvalidResult),
        }
    }

    pub async fn translate(
        source: &str,
        target: &str,
        text: &str,
    ) -> Result<String, AppleTranslationError> {
        let source = language(source)?;
        let target = language(target)?;
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(AppleTranslationError::InvalidInput);
        }
        match request(Duration::from_secs(15), |id| unsafe {
            mimi_apple_translation_translate(
                id,
                source.as_ptr(),
                target.as_ptr(),
                text.as_ptr(),
                text.len(),
                callback,
            )
        })
        .await?
        {
            Response::Translation { text } => Ok(text),
            _ => Err(AppleTranslationError::InvalidResult),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rejects_invalid_language_and_unbounded_results() {
            assert!(language("auto").is_err());
            assert!(language("en\0-US").is_err());
            assert!(language("zh-Hans").is_ok());
            let bytes = serde_json::to_vec(
                &serde_json::json!({"event":"translation", "text":"x".repeat(MAX_TEXT_BYTES + 1)}),
            )
            .unwrap();
            assert!(matches!(
                parse(&bytes),
                Err(AppleTranslationError::InvalidResult)
            ));
            assert!(matches!(
                parse(br#"{"event":"translation","text":"  "}"#),
                Err(AppleTranslationError::InvalidResult)
            ));
        }

        #[test]
        fn malformed_result_errors_never_retain_private_text() {
            let result = parse(br#"{"event":"private content"}"#).err().unwrap();
            assert!(!format!("{result:?} {result}").contains("private content"));
        }

        #[test]
        fn late_callbacks_are_rejected_without_reading_their_buffer() {
            assert_eq!(
                unsafe { callback(u64::MAX, std::ptr::null(), usize::MAX) },
                0
            );
        }

        #[test]
        fn callback_copies_text_before_native_buffer_is_released() {
            let (sender, mut receiver) = oneshot::channel();
            let id = u64::MAX - 1;
            routes().lock().unwrap().insert(id, sender);
            let mut bytes = br#"{"event":"translation","text":"owned result"}"#.to_vec();
            assert_eq!(unsafe { callback(id, bytes.as_ptr(), bytes.len()) }, 1);
            bytes.fill(0);
            assert!(
                matches!(receiver.try_recv().unwrap().unwrap(), Response::Translation { text } if text == "owned result")
            );
        }

        #[tokio::test]
        async fn timed_out_request_removes_callback_route() {
            let identifier = AtomicU64::new(0);
            let result =
                request(Duration::ZERO, |id| identifier.store(id, Ordering::Relaxed)).await;
            assert!(matches!(result, Err(AppleTranslationError::Timeout)));
            let id = identifier.load(Ordering::Relaxed);
            assert_ne!(id, 0);
            assert!(remove(id).is_none());
            // A cancelled future cannot leave a callback that reads its buffer.
            assert_eq!(unsafe { callback(id, std::ptr::null(), usize::MAX) }, 0);
        }

        #[tokio::test]
        async fn dropping_pending_request_removes_callback_route() {
            let identifier = AtomicU64::new(0);
            let mut future = Box::pin(request(Duration::from_secs(10), |id| {
                identifier.store(id, Ordering::Relaxed);
            }));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(future.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            let id = identifier.load(Ordering::Relaxed);
            assert_ne!(id, 0);
            drop(future);
            assert!(remove(id).is_none());
            assert_eq!(unsafe { callback(id, std::ptr::null(), usize::MAX) }, 0);
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub use native::{capabilities, prepare, status, translate};

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod unsupported {
    use super::*;
    pub async fn capabilities() -> Result<AppleTranslationCapabilities, AppleTranslationError> {
        Ok(AppleTranslationCapabilities::default())
    }
    pub async fn status(_: &str, _: &str) -> Result<AppleTranslationStatus, AppleTranslationError> {
        Ok(AppleTranslationStatus::Unavailable)
    }
    pub async fn prepare(_: &str, _: &str, _: &str) -> Result<(), AppleTranslationError> {
        Err(AppleTranslationError::Unavailable)
    }
    pub async fn translate(_: &str, _: &str, _: &str) -> Result<String, AppleTranslationError> {
        Err(AppleTranslationError::Unavailable)
    }
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub use unsupported::{capabilities, prepare, status, translate};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_accepts_resolved_interface_languages_only() {
        for language in ["en", "zh", "zh-TW", "ja", "de", "fr", "ko"] {
            assert!(supports_preparation_ui_language(language), "{language}");
        }
        for language in ["system", "zh-Hant", "zh_tw", "fr-CA", "", "invalid"] {
            assert!(!supports_preparation_ui_language(language), "{language}");
        }
    }

    #[test]
    fn native_error_codes_have_stable_content_free_labels() {
        let cases = [
            (
                AppleTranslationError::Unavailable,
                "apple_translation_unavailable",
            ),
            (
                AppleTranslationError::AssetsNotInstalled,
                "apple_translation_assets_missing",
            ),
            (
                AppleTranslationError::UnsupportedLanguagePair,
                "apple_translation_language_unsupported",
            ),
            (
                AppleTranslationError::InvalidInput,
                "apple_translation_invalid_input",
            ),
            (
                AppleTranslationError::Cancelled,
                "apple_translation_cancelled",
            ),
            (AppleTranslationError::Busy, "apple_translation_busy"),
            (AppleTranslationError::Timeout, "apple_translation_timeout"),
            (
                AppleTranslationError::InvalidResult,
                "apple_translation_invalid_result",
            ),
            (AppleTranslationError::Native, "apple_translation_failed"),
        ];
        for (error, label) in cases {
            assert_eq!(error.to_string(), label);
        }
    }
}
