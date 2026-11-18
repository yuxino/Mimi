//! Read-only CoreAudio output metadata for local presentation.
//! This never changes ScreenCaptureKit's system-mix capture route and names
//! must never enter logs or copied support diagnostics.

use objc2::rc::Retained;
use objc2_foundation::NSString;
use std::ffi::c_void;

const SYSTEM_OBJECT: u32 = 1;
const DEFAULT_OUTPUT_DEVICE: u32 = u32::from_be_bytes(*b"dOut");
const OBJECT_NAME: u32 = u32::from_be_bytes(*b"lnam");
const GLOBAL_SCOPE: u32 = u32::from_be_bytes(*b"glob");

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    #[link_name = "AudioObjectGetPropertyData"]
    fn audio_object_get_property_data(
        object_id: u32,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        data_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}

fn read_property<T: Default>(object_id: u32, selector: u32) -> Option<T> {
    let address = AudioObjectPropertyAddress {
        selector,
        scope: GLOBAL_SCOPE,
        element: 0,
    };
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: Private callers pair each selector with its SDK-defined data
    // type (AudioObjectID or retained CFStringRef) and provide its exact size.
    let status = unsafe {
        audio_object_get_property_data(
            object_id,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            (&mut value as *mut T).cast(),
        )
    };
    (status == 0 && size as usize == std::mem::size_of::<T>()).then_some(value)
}

fn default_output_device_id() -> Option<u32> {
    read_property::<u32>(SYSTEM_OBJECT, DEFAULT_OUTPUT_DEVICE).filter(|device| *device != 0)
}

fn device_name(device: u32) -> Option<String> {
    let name = read_property::<*mut NSString>(device, OBJECT_NAME)?;
    // SAFETY: kAudioObjectPropertyName returns a caller-owned CFStringRef,
    // which is toll-free bridged with NSString. Retained consumes that +1
    // ownership and releases it after conversion, including an empty string.
    let name = unsafe { Retained::from_raw(name) }?;
    Some(name.to_string())
}

fn snapshot_output_name(
    mut default_device: impl FnMut() -> Option<u32>,
    name: impl FnOnce(u32) -> Option<String>,
) -> Option<String> {
    let device = default_device()?;
    let name = name(device)?;
    // A headphone disconnect/default switch during the query invalidates
    // this best-effort snapshot; the next normal UI poll reads it again.
    if default_device()? != device {
        return None;
    }
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub fn default_output_device_name() -> Option<String> {
    snapshot_output_name(default_output_device_id, device_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_switch_during_the_query_does_not_claim_the_old_headphones() {
        let mut devices = [Some(11), Some(12)].into_iter();
        assert_eq!(
            snapshot_output_name(
                || devices.next().flatten(),
                |_| Some("Old headphones".into())
            ),
            None
        );
    }

    #[test]
    fn output_metadata_is_nullable_and_preserves_the_current_device_name() {
        assert_eq!(
            snapshot_output_name(|| None, |_| Some("Headphones".into())),
            None
        );
        assert_eq!(snapshot_output_name(|| Some(11), |_| None), None);
        assert_eq!(
            snapshot_output_name(|| Some(11), |_| Some("  ".into())),
            None
        );
        assert_eq!(
            snapshot_output_name(|| Some(11), |_| Some("  Synthetic headphones  ".into())),
            Some("Synthetic headphones".into())
        );
    }
}
