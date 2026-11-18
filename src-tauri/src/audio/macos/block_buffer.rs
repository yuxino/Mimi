//! Full-range CoreMedia audio reads. Native blocks need not be contiguous.

use core_media::block_buffer::{
    CMBlockBufferCopyDataBytes, CMBlockBufferGetDataLength, CMBlockBufferGetDataPointer,
    CMBlockBufferRef,
};
use std::ffi::c_void;
use std::ptr::null_mut;

const MAX_CAPTURE_BUFFER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadError {
    InvalidBuffer,
    SizeLimit,
    AllocationFailed,
    DataUnavailable,
}

/// Decode a whole buffer while its borrowed or copied storage remains alive.
/// The callback's return type cannot borrow the temporary byte slice.
///
/// # Safety
/// A non-null `buffer` must refer to a live CMBlockBuffer, retained and not
/// modified concurrently until this function and its callback return.
pub(super) unsafe fn with_bytes<R>(
    buffer: CMBlockBufferRef,
    decode: impl FnOnce(&[u8]) -> R,
) -> Result<Option<R>, ReadError> {
    if buffer.is_null() {
        return Err(ReadError::InvalidBuffer);
    }
    // SAFETY: The caller guarantees the native buffer is live and stable.
    let total_length = unsafe { CMBlockBufferGetDataLength(buffer) };
    if total_length == 0 {
        return Ok(None);
    }
    if total_length > MAX_CAPTURE_BUFFER_BYTES {
        return Err(ReadError::SizeLimit);
    }

    let mut contiguous_length = 0;
    let mut reported_total = 0;
    let mut pointer: *mut c_void = null_mut();
    // SAFETY: Live buffer, valid offset and initialized native out-parameters.
    let status = unsafe {
        CMBlockBufferGetDataPointer(
            buffer,
            0,
            &mut contiguous_length,
            &mut reported_total,
            &mut pointer,
        )
    };
    if status == 0
        && !pointer.is_null()
        && reported_total == total_length
        && contiguous_length >= total_length
    {
        // SAFETY: CoreMedia certified the entire bounded range addressable;
        // the caller retains it for this synchronous callback.
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), total_length) };
        return Ok(Some(decode(bytes)));
    }

    // A first-block pointer does not cover subsequent blocks. Copy the complete
    // logical range, including samples/channel planes split across blocks.
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total_length)
        .map_err(|_| ReadError::AllocationFailed)?;
    bytes.resize(total_length, 0u8);
    // SAFETY: Valid retained source and an initialized destination with exactly
    // the complete bounded range's capacity. No bytes are used after failure.
    if unsafe { CMBlockBufferCopyDataBytes(buffer, 0, total_length, bytes.as_mut_ptr().cast()) }
        != 0
    {
        return Err(ReadError::DataUnavailable);
    }
    Ok(Some(decode(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_media::block_buffer::{
        kCMBlockBufferAssureMemoryNowFlag, kCMBlockBufferDontOptimizeDepthFlag,
        CMBlockBufferAppendBufferReference, CMBlockBufferCreateEmpty,
        CMBlockBufferCreateWithMemoryBlock, CMBlockBufferReplaceDataBytes,
    };
    use std::ptr::null;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
    }

    struct NativeBuffer(CMBlockBufferRef);

    impl Drop for NativeBuffer {
        fn drop(&mut self) {
            // SAFETY: Exactly one owned create-rule reference per wrapper.
            unsafe { CFRelease(self.0.cast()) };
        }
    }

    fn empty() -> NativeBuffer {
        let mut raw = null_mut();
        // SAFETY: Default allocator, valid initialized result pointer.
        assert_eq!(
            unsafe { CMBlockBufferCreateEmpty(null(), 2, 0, &mut raw) },
            0
        );
        assert!(!raw.is_null());
        NativeBuffer(raw)
    }

    fn allocated(length: usize, allocate_now: bool) -> NativeBuffer {
        let mut raw = null_mut();
        let flags = if allocate_now {
            kCMBlockBufferAssureMemoryNowFlag
        } else {
            0
        };
        // SAFETY: CoreMedia owns default-allocated storage; there is no borrowed
        // memory or custom allocator. Data range equals requested block length.
        assert_eq!(
            unsafe {
                CMBlockBufferCreateWithMemoryBlock(
                    null(),
                    null(),
                    length,
                    null(),
                    null(),
                    0,
                    length,
                    flags,
                    &mut raw,
                )
            },
            0
        );
        assert!(!raw.is_null());
        NativeBuffer(raw)
    }

    fn from_bytes(bytes: &[u8]) -> NativeBuffer {
        let buffer = allocated(bytes.len(), true);
        // SAFETY: Live allocated destination and full readable source slice.
        assert_eq!(
            unsafe {
                CMBlockBufferReplaceDataBytes(bytes.as_ptr().cast(), buffer.0, 0, bytes.len())
            },
            0
        );
        buffer
    }

    #[test]
    fn contiguous_audio_keeps_the_borrowed_fast_path() {
        let source = [1, 2, 3, 4, 5, 6];
        let buffer = from_bytes(&source);
        let mut pointer = null_mut();
        let mut length = 0;
        let mut total = 0;
        // SAFETY: Live owned native buffer and valid out-parameters.
        assert_eq!(
            unsafe {
                CMBlockBufferGetDataPointer(buffer.0, 0, &mut length, &mut total, &mut pointer)
            },
            0
        );
        // SAFETY: Owned native buffer stays live and unmodified during decode.
        unsafe {
            with_bytes(buffer.0, |bytes| {
                assert_eq!(bytes, source);
                assert_eq!(bytes.as_ptr(), pointer.cast::<u8>());
            })
        }
        .unwrap()
        .unwrap();
    }

    #[test]
    fn complete_audio_and_last_sample_survive_noncontiguous_native_blocks() {
        let samples = [0.5f32, -0.25, 1.0];
        let source: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let buffer = empty();
        // Split inside a PCM sample, then at another boundary. Native references
        // retain each independently allocated child's memory after it drops.
        for bytes in [&source[..3], &source[3..8], &source[8..]] {
            let child = from_bytes(bytes);
            // SAFETY: Both buffers live; each child's complete valid range is
            // retained by the parent. Avoid optimizing separate native blocks.
            assert_eq!(
                unsafe {
                    CMBlockBufferAppendBufferReference(
                        buffer.0,
                        child.0,
                        0,
                        bytes.len(),
                        kCMBlockBufferDontOptimizeDepthFlag,
                    )
                },
                0
            );
        }
        let mut length = 0;
        let mut total = 0;
        let mut pointer = null_mut();
        // SAFETY: Live owned native buffer and valid out-parameters.
        assert_eq!(
            unsafe {
                CMBlockBufferGetDataPointer(buffer.0, 0, &mut length, &mut total, &mut pointer)
            },
            0
        );
        assert_eq!(
            length, 3,
            "old prefix-only read must reproduce lost samples"
        );
        assert_eq!(total, source.len());
        // SAFETY: Owned source retained through the synchronous callback.
        let decoded = unsafe {
            with_bytes(buffer.0, |bytes| {
                assert_eq!(bytes, source);
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect::<Vec<_>>()
            })
        }
        .unwrap()
        .unwrap();
        assert_eq!(decoded, samples);
    }

    #[test]
    fn empty_native_buffer_does_not_invent_audio() {
        let buffer = empty();
        // SAFETY: Owned valid empty native buffer.
        assert_eq!(
            unsafe { with_bytes(buffer.0, |_| panic!("no decode")) },
            Ok(None::<()>)
        );
    }

    #[test]
    fn null_is_rejected_before_native_access() {
        // SAFETY: Null is explicitly handled before any native dereference.
        assert_eq!(
            unsafe { with_bytes(null_mut(), |_| ()) },
            Err(ReadError::InvalidBuffer)
        );
    }

    #[test]
    fn oversized_native_range_is_rejected_before_storage_is_materialized() {
        let buffer = allocated(MAX_CAPTURE_BUFFER_BYTES + 1, false);
        // SAFETY: Owned native range; the reader checks size before data access.
        assert_eq!(
            unsafe { with_bytes(buffer.0, |_| ()) },
            Err(ReadError::SizeLimit)
        );
    }
}
