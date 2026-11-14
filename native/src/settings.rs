use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+W";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub folder: Option<PathBuf>,
    pub shortcut: String,
    pub layout: String,
    pub dim: String,
    pub enter_this_screen: bool,
    pub follow_screen: bool,
    pub map_size: String,
    pub ui_hidden: bool,
    pub hide_hints: bool,
    pub hide_title: bool,
    pub hide_details: bool,
    pub hide_map: bool,
    pub lock_mode: String,
    pub lock_spot: String,
    pub edit_glow: bool,
    pub autostart: bool,
    pub full_jpeg_quality: bool,
    pub last_wallpaper: Option<PathBuf>,
    pub lock_wallpaper: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: None,
            shortcut: DEFAULT_SHORTCUT.into(),
            layout: "slider".into(),
            dim: "soft".into(),
            enter_this_screen: false,
            follow_screen: true,
            map_size: "medium".into(),
            ui_hidden: false,
            hide_hints: true,
            hide_title: false,
            hide_details: false,
            hide_map: false,
            lock_mode: "off".into(),
            lock_spot: "left".into(),
            edit_glow: true,
            autostart: false,
            full_jpeg_quality: true,
            last_wallpaper: None,
            lock_wallpaper: None,
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return home_dir().map(|h| h.join("Library/Application Support").join(crate::APP_ID));
    }
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join(crate::APP_ID))
}

pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return home_dir().map(|h| h.join("Library/Caches").join(crate::APP_ID));
    }
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(crate::APP_ID))
}

fn file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("settings.json"))
}

pub fn exists() -> bool {
    file().is_some_and(|path| path.is_file())
}

pub fn load() -> Settings {
    let mut settings: Settings = file()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        // macOS has no public way to set the lock screen picture
        settings.lock_mode = "off".into();
    }
    settings
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let path = file().ok_or("Couldn't find a place to store settings")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn wallpaper_folder(settings: &Settings) -> Option<PathBuf> {
    if let Some(folder) = &settings.folder {
        if folder.is_dir() {
            return Some(folder.clone());
        }
    }
    let pictures = pictures_dir()?;
    for name in ["Wallpapers", "Wallpaper", "wallpapers", "wallpaper"] {
        let candidate = pictures.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let folder = pictures.join("Wallpapers");
    let _ = fs::create_dir_all(&folder);
    Some(folder)
}

fn pictures_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
        if let Ok(raw) = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None) {
            let path = raw.to_string().ok().map(PathBuf::from);
            windows::Win32::System::Com::CoTaskMemFree(Some(raw.0 as _));
            if path.is_some() {
                return path;
            }
        }
    }
    if cfg!(target_os = "macos") {
        return home_dir().map(|h| h.join("Pictures"));
    }
    std::env::var_os("USERPROFILE").map(|d| PathBuf::from(d).join("Pictures"))
}
