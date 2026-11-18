//! Optional, bounded local shell icons. Never log paths or affect capture identity.
use super::applications::{
    ApplicationIconBudget, AudioApplication, Process, MAX_APPLICATION_ICON_PNG_BYTES,
};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, Prefix};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICImagingFactory, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::{
    SHCreateMemStream, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

pub(super) fn populate(applications: &mut [AudioApplication]) {
    let Some(reader) = IconReader::new() else {
        return;
    };
    let mut budget = ApplicationIconBudget::default();
    for application in applications {
        // Revalidate PID + creation time before reading an executable's icon.
        // An exited/reused process leaves only the existing neutral fallback.
        let png = Process::selected(&application.id)
            .ok()
            .and_then(|process| process.image_path().ok())
            .and_then(|path| reader.png(Path::new(&std::ffi::OsString::from_wide(&path))));
        application.icon_data_url = png.and_then(|png| budget.encode_png(&png));
    }
}

struct ComApartment(bool);
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}
struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        let _ = unsafe { DestroyIcon(self.0) };
    }
}
struct IconReader {
    // Field order releases COM interfaces before uninitializing the apartment.
    factory: IWICImagingFactory,
    _apartment: ComApartment,
}
impl IconReader {
    fn new() -> Option<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result.is_err() && result != RPC_E_CHANGED_MODE {
            return None;
        }
        let apartment = ComApartment(result.is_ok());
        let factory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                .ok()?;
        Some(Self {
            factory,
            _apartment: apartment,
        })
    }

    fn png(&self, path: &Path) -> Option<Vec<u8>> {
        // Shell metadata can consult a remote executable; only local disk paths
        // belong in this presentation-only lookup.
        if !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            || !path.is_file()
        {
            return None;
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut info = SHFILEINFOW::default();
        let result = unsafe {
            SHGetFileInfoW(
                windows::core::PCWSTR(path.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            )
        };
        if result == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let icon = OwnedIcon(info.hIcon);
        self.encode(&icon).ok()
    }

    fn encode(&self, icon: &OwnedIcon) -> windows::core::Result<Vec<u8>> {
        const SIDE: u32 = 32;
        // All native objects are local RAII owners, including early failures.
        unsafe {
            let bitmap = self.factory.CreateBitmapFromHICON(icon.0)?;
            let scaled = self.factory.CreateBitmapScaler()?;
            scaled.Initialize(&bitmap, SIDE, SIDE, WICBitmapInterpolationModeFant)?;
            let stream = SHCreateMemStream(None).ok_or_else(|| {
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY)
            })?;
            let encoder = self
                .factory
                .CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let (mut frame, mut properties) = (None, None);
            encoder.CreateNewFrame(&mut frame, &mut properties)?;
            let frame = frame.ok_or_else(|| {
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL)
            })?;
            frame.Initialize(properties.as_ref())?;
            frame.SetSize(SIDE, SIDE)?;
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format)?;
            frame.WriteSource(&scaled, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;
            let mut stat = STATSTG::default();
            stream.Stat(&mut stat, STATFLAG_NONAME)?;
            if stat.cbSize == 0 || stat.cbSize > MAX_APPLICATION_ICON_PNG_BYTES as u64 {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            stream.Seek(0, STREAM_SEEK_SET, None)?;
            let mut png = vec![0; stat.cbSize as usize];
            let mut read = 0;
            stream
                .Read(png.as_mut_ptr().cast(), png.len() as u32, Some(&mut read))
                .ok()?;
            if read as usize != png.len() {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            Ok(png)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Imaging::WICDecodeMetadataCacheOnLoad;

    #[test]
    fn local_executable_icons_round_trip_as_bounded_32px_pngs() {
        let reader = IconReader::new().expect("native WIC factory");
        let executable = std::env::current_exe().unwrap();
        for _ in 0..32 {
            let png = reader
                .png(&executable)
                .expect("local executable shell icon");
            assert!(png.len() <= MAX_APPLICATION_ICON_PNG_BYTES);
            assert!(ApplicationIconBudget::default().encode_png(&png).is_some());
            unsafe {
                let stream = SHCreateMemStream(Some(&png)).unwrap();
                let decoder = reader
                    .factory
                    .CreateDecoderFromStream(
                        &stream,
                        std::ptr::null(),
                        WICDecodeMetadataCacheOnLoad,
                    )
                    .unwrap();
                let frame = decoder.GetFrame(0).unwrap();
                let (mut width, mut height) = (0, 0);
                frame.GetSize(&mut width, &mut height).unwrap();
                assert_eq!((width, height), (32, 32));
            }
        }
    }

    #[test]
    fn absent_or_remote_executables_have_no_icon() {
        let reader = IconReader::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        assert!(reader.png(&directory.path().join("absent.exe")).is_none());
        assert!(reader
            .png(Path::new(r"\\example.invalid\share\app.exe"))
            .is_none());
    }
}
