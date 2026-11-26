//! AppKit running-application enumeration, filtering, and 64x64 PNG icon rasterization.

use crate::audio::applications::{
    sort_applications, ApplicationIconBudget, ApplicationSnapshot, AudioApplication,
    MAX_APPLICATION_ICON_PNG_BYTES,
};
use crate::audio::SystemAudioCaptureError;
use crate::core::audio::system_audio_target::SystemAudioTarget;
use objc2::AnyThread;
use std::panic::AssertUnwindSafe;

/// Enumerates selectable foreground applications on the main thread without
/// requesting screen contents or recording authorization.
pub(super) fn list_audio_applications() -> Result<ApplicationSnapshot, SystemAudioCaptureError> {
    objc2::exception::catch(AssertUnwindSafe(|| {
        let own = own_bundle_identifier();
        let mut running_apps = std::collections::HashMap::new();
        let mut applications = objc2_app_kit::NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter_map(|app| {
                let application = picker_application(
                    app.isTerminated(),
                    app.activationPolicy() == objc2_app_kit::NSApplicationActivationPolicy::Regular,
                    app.bundleIdentifier().map(|id| id.to_string()),
                    app.localizedName().map(|name| name.to_string()),
                    own.as_deref(),
                )?;
                running_apps.entry(application.id.clone()).or_insert(app);
                Some(application)
            })
            .collect();
        sort_applications(&mut applications);
        let mut icon_budget = ApplicationIconBudget::default();
        for application in &mut applications {
            application.icon_data_url = running_apps
                .get(&application.id)
                .and_then(|app| {
                    objc2::exception::catch(AssertUnwindSafe(|| app.icon()))
                        .ok()
                        .flatten()
                })
                .and_then(|image| picker_icon_png(&image))
                .and_then(|png| icon_budget.encode_png(&png));
        }
        ApplicationSnapshot {
            supported: true,
            applications,
        }
    }))
    .map_err(|_| SystemAudioCaptureError::ApplicationListFailed)
}

pub(super) fn picker_application(
    terminated: bool,
    regular: bool,
    id: Option<String>,
    name: Option<String>,
    own: Option<&str>,
) -> Option<AudioApplication> {
    if terminated || !regular {
        return None;
    }
    let (id, name) = (id?, name?);
    let target = SystemAudioTarget::Application { id, name };
    if !target.validate() || target.application_id() == own {
        return None;
    }
    let SystemAudioTarget::Application { id, name } = target else {
        unreachable!()
    };
    Some(AudioApplication {
        id,
        name,
        icon_data_url: None,
    })
}

/// Render an existing AppKit icon into a fixed-size bitmap. This does not read
/// windows, enumerate screen contents, start capture or request a new permission.
/// Runtime callers execute on the main thread. Failures leave a text-only choice.
pub(super) fn picker_icon_png(image: &objc2_app_kit::NSImage) -> Option<Vec<u8>> {
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};
    // The picker renders at 20 logical pixels; keep enough source pixels for
    // Retina displays instead of enlarging a 32px bitmap to 40 device pixels.
    const EDGE: isize = 64;
    let previous_context = NSGraphicsContext::currentContext();
    let result = objc2::exception::catch(AssertUnwindSafe(|| {
        // SAFETY: Null planes let AppKit own one EDGE x EDGE RGBA buffer with an
        // explicit EDGE * 4-byte row. No caller-owned memory is retained by AppKit.
        let bitmap = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), EDGE, EDGE, 8, 4,
                true, false, NSDeviceRGBColorSpace, EDGE * 4, 32,
            )
        }?;
        let pixels = bitmap.bitmapData();
        if pixels.is_null() || bitmap.bytesPerRow() != EDGE * 4 {
            return None;
        }
        // SAFETY: The bitmap owns this exact RGBA allocation. Clear alpha so
        // transparent padding never includes uninitialized bitmap bytes.
        unsafe { std::ptr::write_bytes(pixels, 0, (EDGE * EDGE * 4) as usize) };
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
        NSGraphicsContext::setCurrentContext(Some(&context));
        image.drawInRect_fromRect_operation_fraction(
            NSRect::new(NSPoint::ZERO, NSSize::new(EDGE as f64, EDGE as f64)),
            NSRect::ZERO, NSCompositingOperation::Copy, 1.0,
        );
        // SAFETY: An empty property dictionary supplies no incorrectly typed
        // codec options. The newly rendered representation contains no source metadata.
        let data = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        (data.len() <= MAX_APPLICATION_ICON_PNG_BYTES).then(|| data.to_vec())
    })).ok().flatten();
    // Restore even after an Objective-C drawing/encoding exception. A missing
    // app icon must not disturb the graphics context used by the next UI draw.
    NSGraphicsContext::setCurrentContext(previous_context.as_deref());
    result
}

pub(super) fn own_bundle_identifier() -> Option<String> {
    use objc2_foundation::NSBundle;
    NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|id| id.to_string())
}
