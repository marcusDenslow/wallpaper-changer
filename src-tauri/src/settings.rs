use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

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

fn file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("settings.json"))
}

pub fn exists(app: &AppHandle) -> bool {
    file(app).is_some_and(|path| path.is_file())
}

pub fn load(app: &AppHandle) -> Settings {
    file(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = file(app).ok_or("Couldn't find a place to store settings")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn wallpaper_folder(app: &AppHandle, settings: &Settings) -> Option<PathBuf> {
    if let Some(folder) = &settings.folder {
        if folder.is_dir() {
            return Some(folder.clone());
        }
    }
    let pictures = app.path().picture_dir().ok()?;
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
