use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Screen {
    pub id: String,
    pub rect: [i32; 4],
    pub wallpaper: Option<PathBuf>,
}

impl Screen {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let [left, top, right, bottom] = self.rect;
        x >= left && x < right && y >= top && y < bottom
    }

    pub fn is_primary(&self) -> bool {
        self.rect[0] == 0 && self.rect[1] == 0
    }
}

pub fn set(path: &Path, screen: Option<&str>, full_jpeg_quality: bool) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} no longer exists", path.display()));
    }
    imp::set(path, screen, full_jpeg_quality)
}

pub fn screens() -> Vec<Screen> {
    imp::screens()
}

pub fn set_lock_screen(path: &Path, cache: &Path) -> Result<(), String> {
    let source = lock_screen_copy(path, cache)?;
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(imp::set_lock_screen(&source));
    });
    result
        .recv_timeout(Duration::from_secs(8))
        .unwrap_or_else(|_| Err("Windows took too long to answer".into()))
}

pub fn lock_screen_image(cache: &Path) -> Option<PathBuf> {
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(imp::lock_screen_bytes());
    });
    let bytes = result.recv_timeout(Duration::from_secs(5)).ok()?.ok()?;
    if bytes.is_empty() {
        return None;
    }
    let ext = if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    };
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let dir = cache.join("lockscreen-current");
    let path = dir.join(format!("{:016x}.{ext}", hasher.finish()));
    if !path.is_file() {
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).ok()?;
        fs::write(&path, &bytes).ok()?;
    }
    Some(path)
}

fn lock_screen_copy(path: &Path, cache: &Path) -> Result<PathBuf, String> {
    let webp = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("webp"));
    if !webp {
        return Ok(path.to_path_buf());
    }
    let dir = cache.join("lockscreen");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    let target = dir.join(format!("{stamp}.jpg"));
    let pixels = image::open(path).map_err(|e| e.to_string())?.into_rgb8();
    let file = fs::File::create(&target).map_err(|e| e.to_string())?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(BufWriter::new(file), 95)
        .encode_image(&pixels)
        .map_err(|e| e.to_string())?;
    Ok(target)
}

#[cfg(windows)]
mod imp {
    use super::Screen;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use windows::core::{w, HSTRING, PCWSTR, PWSTR};
    use windows::Storage::Streams::DataReader;
    use windows::Storage::StorageFile;
    use windows::System::UserProfile::LockScreen;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
        COINIT_MULTITHREADED,
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

    unsafe fn take_string(raw: PWSTR) -> String {
        let text = raw.to_string().unwrap_or_default();
        CoTaskMemFree(Some(raw.0 as *const _));
        text
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

    pub fn set(path: &Path, screen: Option<&str>, full_jpeg_quality: bool) -> Result<(), String> {
        if full_jpeg_quality {
            keep_full_jpeg_quality();
        }
        let wide = HSTRING::from(path.as_os_str());
        let monitor = screen.map(HSTRING::from);
        let applied = with_desktop(move |desktop| unsafe {
            desktop.SetPosition(DWPOS_FILL)?;
            match &monitor {
                Some(id) => desktop.SetWallpaper(id, &wide),
                None => desktop.SetWallpaper(PCWSTR::null(), &wide),
            }
        });
        if applied.is_ok() || screen.is_some() {
            return applied;
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

    pub fn screens() -> Vec<Screen> {
        with_desktop(|desktop| unsafe {
            let mut found = Vec::new();
            for index in 0..desktop.GetMonitorDevicePathCount()? {
                let Ok(raw) = desktop.GetMonitorDevicePathAt(index) else { continue };
                let id = take_string(raw);
                let monitor = HSTRING::from(id.as_str());
                let Ok(rect) = desktop.GetMonitorRECT(&monitor) else { continue };
                if rect.right <= rect.left || rect.bottom <= rect.top {
                    continue;
                }
                let wallpaper = desktop
                    .GetWallpaper(&monitor)
                    .map(|raw| take_string(raw))
                    .ok()
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from);
                found.push(Screen { id, rect: [rect.left, rect.top, rect.right, rect.bottom], wallpaper });
            }
            Ok(found)
        })
        .unwrap_or_default()
    }

    pub fn set_lock_screen(path: &Path) -> Result<(), String> {
        let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let result = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))
            .and_then(|pending| pending.join())
            .and_then(|file| LockScreen::SetImageFileAsync(&file))
            .and_then(|pending| pending.join());
        if init.is_ok() {
            unsafe { CoUninitialize() };
        }
        result.map_err(|e| e.message().to_string())
    }

    pub fn lock_screen_bytes() -> Result<Vec<u8>, String> {
        let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let result = (|| -> windows::core::Result<Vec<u8>> {
            let stream = LockScreen::GetImageStream()?;
            let size = stream.Size()?;
            if size == 0 || size > 256 * 1024 * 1024 {
                return Ok(Vec::new());
            }
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            let loaded = reader.LoadAsync(size as u32)?.join()?;
            let mut bytes = vec![0u8; loaded as usize];
            reader.ReadBytes(&mut bytes)?;
            Ok(bytes)
        })();
        if init.is_ok() {
            unsafe { CoUninitialize() };
        }
        result.map_err(|e| e.message().to_string())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Screen;
    use std::path::Path;

    pub fn set(_path: &Path, _screen: Option<&str>, _full_jpeg_quality: bool) -> Result<(), String> {
        Err("Setting the wallpaper only works on Windows".into())
    }

    pub fn screens() -> Vec<Screen> {
        Vec::new()
    }

    pub fn set_lock_screen(_path: &Path) -> Result<(), String> {
        Err("Setting the lock screen only works on Windows".into())
    }

    pub fn lock_screen_bytes() -> Result<Vec<u8>, String> {
        Err("Reading the lock screen only works on Windows".into())
    }
}
