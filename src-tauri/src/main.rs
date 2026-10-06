#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod library;
mod settings;
mod wallpaper;

use serde::Serialize;
use settings::Settings;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow, WindowEvent, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

struct AppState {
    settings: Mutex<Settings>,
    first_run: AtomicBool,
    visible: AtomicBool,
    dialog_open: AtomicBool,
    shown_at: Mutex<Instant>,
    open_when_ready: AtomicBool,
    shortcut_error: Mutex<Option<String>>,
    tray_open_item: Mutex<Option<MenuItem<Wry>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    settings: Settings,
    folder: Option<String>,
    current: Option<String>,
    first_run: bool,
    shortcut_error: Option<String>,
}

#[derive(Serialize, Clone)]
struct OpenRequest {
    view: &'static str,
}

fn overlay(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn cover_monitor_under_cursor(win: &WebviewWindow) {
    let monitor = win
        .cursor_position()
        .ok()
        .and_then(|p| win.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.current_monitor().ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let _ = win.set_position(PhysicalPosition::new(m.position().x, m.position().y));
        let _ = win.set_size(PhysicalSize::new(m.size().width, m.size().height));
    }
}

fn show_overlay(app: &AppHandle, view: &'static str) {
    let Some(win) = overlay(app) else { return };
    let state = app.state::<AppState>();
    if !state.visible.swap(true, SeqCst) {
        *state.shown_at.lock().unwrap() = Instant::now();
        cover_monitor_under_cursor(&win);
        let _ = win.show();
    }
    let _ = win.set_always_on_top(true);
    let _ = win.unminimize();
    let _ = win.set_focus();
    let _ = win.emit("overlay://open", OpenRequest { view });
}

fn request_close(app: &AppHandle) {
    if !app.state::<AppState>().visible.load(SeqCst) {
        return;
    }
    if let Some(win) = overlay(app) {
        let _ = win.emit("overlay://close", ());
    }
}

fn toggle_overlay(app: &AppHandle) {
    if app.state::<AppState>().visible.load(SeqCst) {
        request_close(app);
    } else {
        show_overlay(app, "picker");
    }
}

fn on_shortcut(app: &AppHandle, _: &Shortcut, event: ShortcutEvent) {
    if event.state == ShortcutState::Pressed {
        toggle_overlay(app);
    }
}

fn register_shortcut(app: &AppHandle, accel: &str, previous: Option<&str>) -> Result<(), String> {
    let shortcut: Shortcut = accel.parse().map_err(|e| format!("\"{accel}\" isn't a valid shortcut ({e})"))?;
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    shortcuts.on_shortcut(shortcut, on_shortcut).map_err(|e| {
        if let Some(old) = previous.and_then(|p| p.parse::<Shortcut>().ok()) {
            let _ = shortcuts.on_shortcut(old, on_shortcut);
        }
        format!("Couldn't use {accel}. Another app might already be using it. ({e})")
    })
}

fn readable_shortcut(accel: &str) -> String {
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

fn update_tray_label(app: &AppHandle) {
    let state = app.state::<AppState>();
    let shortcut = state.settings.lock().unwrap().shortcut.clone();
    let item = state.tray_open_item.lock().unwrap().clone();
    if let Some(item) = item {
        let _ = item.set_text(format!("Open        {}", readable_shortcut(&shortcut)));
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let prefs = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &prefs, &PredefinedMenuItem::separator(app)?, &quit])?;
    *app.state::<AppState>().tray_open_item.lock().unwrap() = Some(open);

    let mut tray = TrayIconBuilder::with_id("tray")
        .tooltip("Wallpaper Switcher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_overlay(app, "picker"),
            "settings" => show_overlay(app, "settings"),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_overlay(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    update_tray_label(app);
    Ok(())
}

fn cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_cache_dir().map_err(|e| e.to_string())
}

fn allow_images_from(app: &AppHandle, folder: Option<&Path>) {
    let scope = app.asset_protocol_scope();
    if let Some(folder) = folder {
        let _ = scope.allow_directory(folder, true);
    }
    if let Ok(cache) = cache_dir(app) {
        let thumbs = cache.join("thumbs");
        let _ = std::fs::create_dir_all(&thumbs);
        let _ = scope.allow_directory(thumbs, false);
    }
}

fn current_settings(app: &AppHandle) -> Settings {
    app.state::<AppState>().settings.lock().unwrap().clone()
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let state = app.state::<AppState>();
    let settings = current_settings(app);
    let folder = settings::wallpaper_folder(app, &settings);
    let current = wallpaper::current().or_else(|| settings.last_wallpaper.clone());
    let shortcut_error = state.shortcut_error.lock().unwrap().clone();
    Snapshot {
        folder: folder.map(|f| f.to_string_lossy().into_owned()),
        current: current.map(|c| c.to_string_lossy().into_owned()),
        first_run: state.first_run.load(SeqCst),
        shortcut_error,
        settings,
    }
}

fn warm_thumbnails(app: &AppHandle) {
    let settings = current_settings(app);
    if let (Some(folder), Ok(cache)) = (settings::wallpaper_folder(app, &settings), cache_dir(app)) {
        allow_images_from(app, Some(&folder));
        std::thread::spawn(move || library::prewarm(cache, library::scan(&folder)));
    }
}

#[tauri::command]
fn get_state(app: AppHandle) -> Snapshot {
    snapshot(&app)
}

#[tauri::command]
fn ready(app: AppHandle, state: State<AppState>) {
    if state.open_when_ready.swap(false, SeqCst) {
        show_overlay(&app, "picker");
    }
}

#[tauri::command]
async fn list_wallpapers(app: AppHandle) -> Result<Vec<library::Wallpaper>, String> {
    let Some(folder) = settings::wallpaper_folder(&app, &current_settings(&app)) else {
        return Ok(vec![]);
    };
    allow_images_from(&app, Some(&folder));
    tauri::async_runtime::spawn_blocking(move || library::scan(&folder))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn thumbnail(app: AppHandle, path: String) -> Result<library::Thumb, String> {
    let source = PathBuf::from(path);
    if !library::is_image(&source) {
        return Err("Not a supported image".into());
    }
    let cache = cache_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || library::thumbnail(&cache, &source))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn apply_wallpaper(app: AppHandle, path: String) -> Result<(), String> {
    let source = PathBuf::from(path);
    if !library::is_image(&source) {
        return Err("Not a supported image".into());
    }
    let full_quality = current_settings(&app).full_jpeg_quality;
    let target = source.clone();
    tauri::async_runtime::spawn_blocking(move || wallpaper::set(&target, full_quality))
        .await
        .map_err(|e| e.to_string())??;

    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    settings.last_wallpaper = Some(source);
    let _ = settings::save(&app, &settings);
    Ok(())
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<Snapshot, String> {
    let previous = current_settings(&app);
    let mut next = settings;
    next.last_wallpaper = previous.last_wallpaper.clone();
    if !matches!(next.layout.as_str(), "slider" | "grid") {
        next.layout = "slider".into();
    }
    if !matches!(next.dim.as_str(), "clear" | "soft" | "deep") {
        next.dim = "soft".into();
    }

    let mut problems = Vec::new();
    if next.shortcut != previous.shortcut {
        match register_shortcut(&app, &next.shortcut, Some(&previous.shortcut)) {
            Ok(()) => *state.shortcut_error.lock().unwrap() = None,
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

    settings::save(&app, &next)?;
    let folder_changed = next.folder != previous.folder;
    *state.settings.lock().unwrap() = next;
    update_tray_label(&app);
    if folder_changed {
        warm_thumbnails(&app);
    }
    if problems.is_empty() {
        Ok(snapshot(&app))
    } else {
        Err(problems.join("\n"))
    }
}

#[tauri::command]
fn finish_welcome(app: AppHandle, state: State<AppState>, autostart: bool) -> Result<Snapshot, String> {
    let _ = autostart::set(autostart);
    let mut settings = current_settings(&app);
    settings.autostart = autostart::is_enabled();
    settings::save(&app, &settings)?;
    *state.settings.lock().unwrap() = settings;
    state.first_run.store(false, SeqCst);
    Ok(snapshot(&app))
}

#[tauri::command]
fn open_folder(app: AppHandle) -> Result<(), String> {
    let folder = settings::wallpaper_folder(&app, &current_settings(&app)).ok_or("There's no wallpaper folder yet")?;
    std::process::Command::new("explorer")
        .arg(&folder)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't open {}: {e}", folder.display()))
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    state.dialog_open.store(true, SeqCst);
    let start = settings::wallpaper_folder(&app, &current_settings(&app));
    let parent = overlay(&app);
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = handle.dialog().file().set_title("Choose your wallpaper folder");
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        if let Some(win) = parent.as_ref() {
            dialog = dialog.set_parent(win);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .ok()
    .flatten();
    state.dialog_open.store(false, SeqCst);
    if let Some(win) = overlay(&app) {
        let _ = win.set_focus();
    }
    picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn hide_overlay(app: AppHandle, state: State<AppState>) {
    state.visible.store(false, SeqCst);
    if let Some(win) = overlay(&app) {
        let _ = win.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn main() {
    let started_quietly = std::env::args().any(|a| a == "--autostart" || a == "--hidden");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if args.iter().any(|a| a == "--toggle") {
                toggle_overlay(app);
            } else if args.iter().any(|a| a == "--settings") {
                show_overlay(app, "settings");
            } else if !args.iter().any(|a| a == "--autostart" || a == "--hidden") {
                show_overlay(app, "picker");
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            settings: Mutex::new(Settings::default()),
            first_run: AtomicBool::new(false),
            visible: AtomicBool::new(false),
            dialog_open: AtomicBool::new(false),
            shown_at: Mutex::new(Instant::now()),
            open_when_ready: AtomicBool::new(!started_quietly),
            shortcut_error: Mutex::new(None),
            tray_open_item: Mutex::new(None),
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let state = handle.state::<AppState>();
            state.first_run.store(!settings::exists(&handle), SeqCst);

            let mut loaded = settings::load(&handle);
            loaded.autostart = autostart::is_enabled();
            if loaded.autostart {
                let _ = autostart::enable();
            }
            let shortcut = loaded.shortcut.clone();
            *state.settings.lock().unwrap() = loaded;

            if let Err(e) = register_shortcut(&handle, &shortcut, None) {
                *state.shortcut_error.lock().unwrap() = Some(e);
            }
            build_tray(&handle)?;
            warm_thumbnails(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    request_close(app);
                }
                WindowEvent::Focused(false) => {
                    let state = app.state::<AppState>();
                    let settled = state.shown_at.lock().unwrap().elapsed() > Duration::from_millis(350);
                    if settled && !state.dialog_open.load(SeqCst) {
                        request_close(app);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            ready,
            list_wallpapers,
            thumbnail,
            apply_wallpaper,
            save_settings,
            finish_welcome,
            open_folder,
            pick_folder,
            hide_overlay,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("Wallpaper Switcher failed to start");
}
