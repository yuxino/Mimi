//! Fresh, read-only Windows audio observations, scoped to each caller.
//! Default-output selection needs endpoint levels; application discovery needs
//! session process IDs, including applications whose windows are hidden.

use crate::audio::windows::{com_endpoint_id, with_com};
use windows::core::Interface;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eRender, IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// Raw WASAPI endpoint IDs and current peak levels, in native enumeration order.
pub(super) fn endpoint_levels() -> Vec<(String, f32)> {
    let mut levels = Vec::new();
    for_each_render_endpoint(|device| unsafe {
        let Some(id) = com_endpoint_id(device) else {
            return;
        };
        let level = device
            .Activate::<IAudioMeterInformation>(CLSCTX_ALL, None)
            .and_then(|meter| meter.GetPeakValue())
            .unwrap_or(0.0);
        levels.push((id, level));
    });
    levels
}

/// Preserve all session owners regardless of current activity or window visibility.
/// The application picker owns process identity, display names and its result limit.
pub(super) fn session_process_ids() -> Vec<u32> {
    let mut processes = Vec::new();
    for_each_render_endpoint(|device| unsafe {
        collect_session_process_ids(device, &mut processes);
    });
    processes
}

/// Device interfaces stay inside the caller thread's existing COM lifetime.
fn for_each_render_endpoint(mut visit: impl FnMut(&IMMDevice)) {
    with_com(|| unsafe {
        let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        else {
            return;
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return;
        };
        let Ok(count) = collection.GetCount() else {
            return;
        };
        for index in 0..count {
            if let Ok(device) = collection.Item(index) {
                visit(&device);
            }
        }
    });
}

unsafe fn collect_session_process_ids(device: &IMMDevice, out: &mut Vec<u32>) {
    let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
        return;
    };
    let Ok(enumerator): Result<IAudioSessionEnumerator, _> = manager.GetSessionEnumerator() else {
        return;
    };
    let Ok(count) = enumerator.GetCount() else {
        return;
    };
    for index in 0..count {
        let Ok(control) = enumerator.GetSession(index) else {
            continue;
        };
        // GetSession returns the base interface; process ownership is on Control2.
        let Ok(control) = control.cast::<IAudioSessionControl2>() else {
            continue;
        };
        if let Ok(pid) = control.GetProcessId() {
            out.push(pid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_render_endpoints_have_bounded_levels() {
        let levels = endpoint_levels();
        // When CPAL can see render devices this projection must see them too;
        // a headless CI machine is allowed to report none.
        let cpal_devices = cpal::traits::HostTrait::output_devices(&cpal::default_host())
            .map(|devices| devices.count())
            .unwrap_or(0);
        if cpal_devices > 0 {
            assert!(
                !levels.is_empty(),
                "observation missed {cpal_devices} render endpoints"
            );
        }
        for (id, level) in levels {
            assert!(!id.is_empty());
            assert!((0.0..=1.0).contains(&level), "level {level}");
        }
    }
}
