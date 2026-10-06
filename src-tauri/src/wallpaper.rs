use std::path::{Path, PathBuf};

pub fn set(path: &Path, full_jpeg_quality: bool) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} no longer exists", path.display()));
    }
    imp::set(path, full_jpeg_quality)
}

pub fn current() -> Option<PathBuf> {
    imp::current()
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD};
    use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper, DWPOS_FILL};
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
    };

    fn with_desktop<T: Send + 'static>(
        work: impl FnOnce(&IDesktopWallpaper) -> windows::core::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        std::thread::spawn(move || unsafe {
            let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let result = CoCreateInstance::<_, IDesktopWallpaper>(&DesktopWallpaper, None, CLSCTX_ALL)
                .and_then(|desktop| work(&desktop));
            if init.is_ok() {
                CoUninitialize();
            }
            result.map_err(|e| e.message().to_string())
        })
        .join()
        .map_err(|_| "The wallpaper thread crashed".to_string())?
    }

    fn keep_full_jpeg_quality() {
        let quality: u32 = 100;
        unsafe {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                w!("Control Panel\\Desktop"),
                w!("JPEGImportQuality"),
                REG_DWORD.0,
                Some((&quality as *const u32).cast()),
                4,
            );
        }
    }

    pub fn set(path: &Path, full_jpeg_quality: bool) -> Result<(), String> {
        if full_jpeg_quality {
            keep_full_jpeg_quality();
        }
        let wide = HSTRING::from(path.as_os_str());
        let applied = with_desktop(move |desktop| unsafe {
            desktop.SetPosition(DWPOS_FILL)?;
            desktop.SetWallpaper(PCWSTR::null(), &wide)
        });
        if applied.is_ok() {
            return Ok(());
        }

        let mut buffer: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(buffer.as_mut_ptr().cast()),
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        }
        .map_err(|e| format!("Windows refused the wallpaper: {}", e.message()))
    }

    pub fn current() -> Option<PathBuf> {
        with_desktop(|desktop| unsafe {
            let raw = desktop.GetWallpaper(PCWSTR::null())?;
            let path = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as *const _));
            Ok(path)
        })
        .ok()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::{Path, PathBuf};

    pub fn set(_path: &Path, _full_jpeg_quality: bool) -> Result<(), String> {
        Err("Setting the wallpaper only works on Windows".into())
    }

    pub fn current() -> Option<PathBuf> {
        None
    }
}
