//! App state and the operations the old Tauri commands provided.

use crate::settings::{self, Settings};
use crate::{autostart, library, wallpaper};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct ScreenInfo {
    pub number: usize,
    pub here: bool,
    pub editing: bool,
    pub rect: [i32; 4],
    pub current: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub settings: Settings,
    pub folder: Option<String>,
    pub current: Option<String>,
    pub screens: Vec<ScreenInfo>,
    pub editing_lock: bool,
    pub first_run: bool,
    pub shortcut_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Applied {
    pub scope: &'static str,
    pub lock_screen: Option<bool>,
    pub lock_error: Option<String>,
}

pub struct Core {
    pub settings: Settings,
    pub first_run: bool,
    pub overlay_rect: Option<[i32; 4]>,
    pub target_rect: Option<[i32; 4]>,
    pub editing_lock: bool,
    pub shortcut_error: Option<String>,
    pub cache: PathBuf,
}

pub fn center(rect: [i32; 4]) -> (i32, i32) {
    ((rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2)
}

pub fn sorted_screens() -> Vec<wallpaper::Screen> {
    let mut screens = wallpaper::screens();
    screens.sort_by_key(|s| (s.rect[0], s.rect[1]));
    screens
}

fn screen_at(screens: &[wallpaper::Screen], rect: Option<[i32; 4]>) -> Option<usize> {
    let (x, y) = center(rect?);
    screens.iter().position(|s| s.contains(x, y))
}

impl Core {
    pub fn new() -> Self {
        let first_run = !settings::exists();
        let mut loaded = settings::load();
        loaded.autostart = autostart::is_enabled();
        if loaded.autostart {
            let _ = autostart::enable();
        }
        Self {
            settings: loaded,
            first_run,
            overlay_rect: None,
            target_rect: None,
            editing_lock: false,
            shortcut_error: None,
            cache: settings::cache_dir().unwrap_or_else(std::env::temp_dir),
        }
    }

    pub fn folder(&self) -> Option<PathBuf> {
        settings::wallpaper_folder(&self.settings)
    }

    fn screen_here(&self, screens: &[wallpaper::Screen]) -> Option<usize> {
        screen_at(screens, self.overlay_rect)
    }

    pub fn screen_editing(&self, screens: &[wallpaper::Screen]) -> Option<usize> {
        screen_at(screens, self.target_rect).or_else(|| self.screen_here(screens))
    }

    pub fn snapshot(&self) -> Snapshot {
        let screens = sorted_screens();
        let here = self.screen_here(&screens);
        let editing = self.screen_editing(&screens);
        let current = editing
            .and_then(|i| screens[i].wallpaper.clone())
            .or_else(|| screens.iter().find_map(|s| s.wallpaper.clone()))
            .or_else(|| self.settings.last_wallpaper.clone());
        Snapshot {
            folder: self.folder().map(|f| f.to_string_lossy().into_owned()),
            current: current.map(|c| c.to_string_lossy().into_owned()),
            screens: screens
                .iter()
                .enumerate()
                .map(|(i, s)| ScreenInfo {
                    number: i + 1,
                    here: Some(i) == here,
                    editing: Some(i) == editing,
                    rect: s.rect,
                    current: s.wallpaper.as_ref().map(|p| p.to_string_lossy().into_owned()),
                })
                .collect(),
            editing_lock: self.editing_lock && self.settings.lock_mode == "own",
            first_run: self.first_run,
            shortcut_error: self.shortcut_error.clone(),
            settings: self.settings.clone(),
        }
    }

    pub fn list_wallpapers(&self) -> Vec<library::Wallpaper> {
        self.folder().map(|f| library::scan(&f)).unwrap_or_default()
    }

    pub fn warm_thumbnails(&self) {
        if let Some(folder) = self.folder() {
            let cache = self.cache.clone();
            std::thread::spawn(move || library::prewarm(cache, library::scan(&folder)));
        }
    }

    /// Runs on a worker thread: everything it needs is passed in.
    pub fn apply_job(&self, path: String, everywhere: bool) -> impl FnOnce() -> Result<Applied, String> + Send {
        let source = PathBuf::from(path);
        let settings = self.settings.clone();
        let editing_lock = self.editing_lock;
        let cache = self.cache.clone();
        let target_rect = self.target_rect;
        let overlay_rect = self.overlay_rect;
        move || {
            if !library::is_image(&source) {
                return Err("Not a supported image".into());
            }
            if settings.lock_mode == "own" && editing_lock {
                wallpaper::set_lock_screen(&source, &cache)?;
                return Ok(Applied { scope: "lock", lock_screen: Some(true), lock_error: None });
            }
            let screens = sorted_screens();
            let screen = if everywhere || screens.len() < 2 {
                None
            } else {
                screen_at(&screens, target_rect).or_else(|| screen_at(&screens, overlay_rect)).map(|i| screens[i].clone())
            };
            wallpaper::set(&source, screen.as_ref().map(|s| s.id.as_str()), settings.full_jpeg_quality)?;
            let mut applied =
                Applied { scope: if screen.is_some() { "screen" } else { "all" }, lock_screen: None, lock_error: None };
            if settings.lock_mode == "main" && screen.as_ref().map_or(true, |s| s.is_primary()) {
                let result = wallpaper::set_lock_screen(&source, &cache);
                applied.lock_screen = Some(result.is_ok());
                applied.lock_error = result.err();
            }
            Ok(applied)
        }
    }

    /// Bookkeeping after a successful apply (the part that ran after the await in Tauri).
    pub fn applied(&mut self, path: &str, applied: &Applied) {
        let source = PathBuf::from(path);
        if applied.scope == "lock" {
            self.settings.lock_wallpaper = Some(source);
        } else {
            if applied.lock_screen == Some(true) {
                self.settings.lock_wallpaper = Some(source.clone());
            }
            self.settings.last_wallpaper = Some(source);
        }
        let _ = settings::save(&self.settings);
    }

    pub fn select_lock(&mut self) -> Snapshot {
        if self.settings.lock_mode == "own" {
            self.editing_lock = true;
        }
        self.snapshot()
    }

    pub fn lock_screen_job(&self) -> impl FnOnce() -> Option<String> + Send {
        let known = self.settings.lock_wallpaper.clone().filter(|p| p.is_file());
        let cache = self.cache.clone();
        move || wallpaper::lock_screen_image(&cache).or(known).map(|p| p.to_string_lossy().into_owned())
    }

    /// Validates and stores settings. `register` re-registers the global shortcut.
    pub fn save_settings(
        &mut self,
        mut next: Settings,
        register: &mut dyn FnMut(&str, Option<&str>) -> Result<(), String>,
    ) -> Result<Snapshot, String> {
        let previous = self.settings.clone();
        next.last_wallpaper = previous.last_wallpaper.clone();
        next.lock_wallpaper = previous.lock_wallpaper.clone();
        if !matches!(next.lock_mode.as_str(), "off" | "main" | "own") {
            next.lock_mode = "off".into();
        }
        if !matches!(next.lock_spot.as_str(), "left" | "above") {
            next.lock_spot = "left".into();
        }
        if !matches!(next.layout.as_str(), "slider" | "grid") {
            next.layout = "slider".into();
        }
        if !matches!(next.dim.as_str(), "clear" | "soft" | "deep") {
            next.dim = "soft".into();
        }
        if !matches!(next.map_size.as_str(), "small" | "medium" | "large") {
            next.map_size = "medium".into();
        }
        let mut problems = Vec::new();
        if next.shortcut != previous.shortcut {
            match register(&next.shortcut, Some(&previous.shortcut)) {
                Ok(()) => self.shortcut_error = None,
                Err(e) => {
                    next.shortcut = previous.shortcut.clone();
                    problems.push(e);
                }
            }
        }
        if next.autostart != previous.autostart {
            if let Err(e) = autostart::set(next.autostart) {
                problems.push(format!("Couldn't change the start-up setting: {e}"));
            }
            next.autostart = autostart::is_enabled();
        }
        settings::save(&next)?;
        let folder_changed = next.folder != previous.folder;
        if next.lock_mode != "own" {
            self.editing_lock = false;
        }
        self.settings = next;
        if folder_changed {
            self.warm_thumbnails();
        }
        if problems.is_empty() {
            Ok(self.snapshot())
        } else {
            Err(problems.join("\n"))
        }
    }

    pub fn finish_welcome(&mut self, autostart_on: bool) -> Snapshot {
        let _ = autostart::set(autostart_on);
        self.settings.autostart = autostart::is_enabled();
        let _ = settings::save(&self.settings);
        self.first_run = false;
        self.snapshot()
    }

    pub fn open_folder(&self) -> Result<(), String> {
        let folder = self.folder().ok_or("There's no wallpaper folder yet")?;
        std::process::Command::new("explorer")
            .arg(&folder)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Couldn't open {}: {e}", folder.display()))
    }
}

pub fn readable_shortcut(accel: &str) -> String {
    accel
        .split('+')
        .map(|key| match key.trim().to_ascii_lowercase().as_str() {
            "super" | "cmd" | "command" => "Win".to_string(),
            "control" | "ctrl" => "Ctrl".to_string(),
            other => {
                let mut chars = other.chars();
                chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

pub fn is_image(p: &str) -> bool {
    library::is_image(Path::new(p))
}
