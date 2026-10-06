#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod glow;
mod library;
mod settings;
mod wallpaper;

use serde::Serialize;
use settings::Settings;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, State, WebviewWindow, WindowEvent, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

struct AppState {
    settings: Mutex<Settings>,
    first_run: AtomicBool,
    visible: AtomicBool,
    dialog_open: AtomicBool,
    shown_at: Mutex<Instant>,
    overlay_rect: Mutex<Option<[i32; 4]>>,
    target_rect: Mutex<Option<[i32; 4]>>,
    editing_lock: AtomicBool,
    glow_turn: AtomicU64,
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
    screens: Vec<ScreenInfo>,
    editing_lock: bool,
    first_run: bool,
    shortcut_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenInfo {
    number: usize,
    here: bool,
    editing: bool,
    rect: [i32; 4],
    current: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Applied {
    scope: &'static str,
    lock_screen: Option<bool>,
    lock_error: Option<String>,
}

#[derive(Serialize, Clone)]
struct OpenRequest {
    view: &'static str,
}

fn overlay(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn glow_target(app: &AppHandle) -> Option<[i32; 4]> {
    let state = app.state::<AppState>();
    if !state.visible.load(SeqCst) || state.editing_lock.load(SeqCst) || !current_settings(app).edit_glow {
        return None;
    }
    let target = (*state.target_rect.lock().unwrap())?;
    let (x, y) = center(target);
    let here = *state.overlay_rect.lock().unwrap();
    match here {
        Some([l, t, r, b]) if x >= l && x < r && y >= t && y < b => None,
        _ => Some(target),
    }
}

fn flash_glow(app: &AppHandle) {
    let state = app.state::<AppState>();
    let turn = state.glow_turn.fetch_add(1, SeqCst) + 1;
    let Some(rect) = glow_target(app) else { return };
    let handle = app.clone();
    glow::flash(rect, move || handle.state::<AppState>().glow_turn.load(SeqCst) == turn);
}

fn hide_glow(app: &AppHandle) {
    app.state::<AppState>().glow_turn.fetch_add(1, SeqCst);
}

fn monitor_rect(monitor: &Monitor) -> [i32; 4] {
    let (pos, size) = (monitor.position(), monitor.size());
    [pos.x, pos.y, pos.x + size.width as i32, pos.y + size.height as i32]
}

fn center(rect: [i32; 4]) -> (i32, i32) {
    ((rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2)
}

fn cover(app: &AppHandle, win: &WebviewWindow, monitor: &Monitor) {
    let (pos, size) = (*monitor.position(), *monitor.size());
    let _ = win.set_position(pos);
    let _ = win.set_size(size);
    let _ = win.set_position(pos);
    *app.state::<AppState>().overlay_rect.lock().unwrap() = Some(monitor_rect(monitor));
}

fn monitor_under_cursor(win: &WebviewWindow) -> Option<Monitor> {
    win.cursor_position()
        .ok()
        .and_then(|p| win.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.current_monitor().ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten())
}

fn show_overlay(app: &AppHandle, view: &'static str) {
    let Some(win) = overlay(app) else { return };
    let state = app.state::<AppState>();
    if !state.visible.swap(true, SeqCst) {
        *state.shown_at.lock().unwrap() = Instant::now();
        let monitor = monitor_under_cursor(&win);
        if let Some(m) = &monitor {
            cover(app, &win, m);
        }
        let _ = win.show();
        if let Some(m) = &monitor {
            cover(app, &win, m);
        }
        *state.target_rect.lock().unwrap() = monitor.as_ref().map(monitor_rect);
        state.editing_lock.store(false, SeqCst);
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

fn sorted_screens() -> Vec<wallpaper::Screen> {
    let mut screens = wallpaper::screens();
    screens.sort_by_key(|s| (s.rect[0], s.rect[1]));
    screens
}

fn screen_at(screens: &[wallpaper::Screen], rect: Option<[i32; 4]>) -> Option<usize> {
    let (x, y) = center(rect?);
    screens.iter().position(|s| s.contains(x, y))
}

fn screen_here(app: &AppHandle, screens: &[wallpaper::Screen]) -> Option<usize> {
    screen_at(screens, *app.state::<AppState>().overlay_rect.lock().unwrap())
}

fn screen_editing(app: &AppHandle, screens: &[wallpaper::Screen]) -> Option<usize> {
    screen_at(screens, *app.state::<AppState>().target_rect.lock().unwrap()).or_else(|| screen_here(app, screens))
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let state = app.state::<AppState>();
    let settings = current_settings(app);
    let folder = settings::wallpaper_folder(app, &settings);
    let screens = sorted_screens();
    let here = screen_here(app, &screens);
    let editing = screen_editing(app, &screens);
    let current = editing
        .and_then(|i| screens[i].wallpaper.clone())
        .or_else(|| screens.iter().find_map(|s| s.wallpaper.clone()))
        .or_else(|| settings.last_wallpaper.clone());
    let shortcut_error = state.shortcut_error.lock().unwrap().clone();
    Snapshot {
        folder: folder.map(|f| f.to_string_lossy().into_owned()),
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
        editing_lock: state.editing_lock.load(SeqCst) && settings.lock_mode == "own",
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
    let transcoded = source.file_name().is_some_and(|name| name == "TranscodedWallpaper");
    if !library::is_image(&source) && !transcoded {
        return Err("Not a supported image".into());
    }
    let cache = cache_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || library::thumbnail(&cache, &source))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn apply_wallpaper(app: AppHandle, path: String, everywhere: bool) -> Result<Applied, String> {
    let source = PathBuf::from(path);
    if !library::is_image(&source) {
        return Err("Not a supported image".into());
    }
    let settings = current_settings(&app);
    let state = app.state::<AppState>();
    let cache = cache_dir(&app);

    if settings.lock_mode == "own" && state.editing_lock.load(SeqCst) {
        let cache = cache?;
        let image = source.clone();
        tauri::async_runtime::spawn_blocking(move || wallpaper::set_lock_screen(&image, &cache))
            .await
            .map_err(|e| e.to_string())??;
        let mut settings = state.settings.lock().unwrap();
        settings.lock_wallpaper = Some(source);
        let _ = settings::save(&app, &settings);
        return Ok(Applied { scope: "lock", lock_screen: Some(true), lock_error: None });
    }

    let full_quality = settings.full_jpeg_quality;
    let follow_lock = settings.lock_mode == "main";
    let handle = app.clone();
    let image = source.clone();
    let applied = tauri::async_runtime::spawn_blocking(move || {
        let screens = sorted_screens();
        let screen = if everywhere || screens.len() < 2 {
            None
        } else {
            screen_editing(&handle, &screens).map(|i| screens[i].clone())
        };
        wallpaper::set(&image, screen.as_ref().map(|s| s.id.as_str()), full_quality)?;
        let mut applied = Applied {
            scope: if screen.is_some() { "screen" } else { "all" },
            lock_screen: None,
            lock_error: None,
        };
        if follow_lock && screen.as_ref().map_or(true, |s| s.is_primary()) {
            let result = cache.and_then(|dir| wallpaper::set_lock_screen(&image, &dir));
            applied.lock_screen = Some(result.is_ok());
            applied.lock_error = result.err();
        }
        Ok::<_, String>(applied)
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut settings = state.settings.lock().unwrap();
    if applied.lock_screen == Some(true) {
        settings.lock_wallpaper = Some(source.clone());
    }
    settings.last_wallpaper = Some(source);
    let _ = settings::save(&app, &settings);
    Ok(applied)
}

#[tauri::command]
fn select_screen(app: AppHandle, index: usize) -> Snapshot {
    let screens = sorted_screens();
    let Some(screen) = screens.get(index) else { return snapshot(&app) };
    let state = app.state::<AppState>();
    *state.target_rect.lock().unwrap() = Some(screen.rect);
    state.editing_lock.store(false, SeqCst);

    if current_settings(&app).follow_screen {
        if let Some(win) = overlay(&app) {
            let (x, y) = center(screen.rect);
            let monitor = win.available_monitors().ok().and_then(|monitors| {
                monitors.into_iter().find(|m| {
                    let [l, t, r, b] = monitor_rect(m);
                    x >= l && x < r && y >= t && y < b
                })
            });
            if let Some(monitor) = monitor {
                let from = *state.overlay_rect.lock().unwrap();
                let cursor = win.cursor_position().ok();
                cover(&app, &win, &monitor);
                let _ = win.set_focus();
                if let (Some([l, t, r, b]), Some(cursor)) = (from, cursor) {
                    let fx = ((cursor.x - l as f64) / (r - l).max(1) as f64).clamp(0.0, 1.0);
                    let fy = ((cursor.y - t as f64) / (b - t).max(1) as f64).clamp(0.0, 1.0);
                    let size = monitor.size();
                    let _ = win.set_cursor_position(PhysicalPosition::new(
                        (fx * size.width as f64).round() as i32,
                        (fy * size.height as f64).round() as i32,
                    ));
                }
            }
        }
    }
    flash_glow(&app);
    snapshot(&app)
}

#[tauri::command]
fn select_lock(app: AppHandle) -> Snapshot {
    if current_settings(&app).lock_mode == "own" {
        app.state::<AppState>().editing_lock.store(true, SeqCst);
    }
    snapshot(&app)
}

#[tauri::command]
async fn lock_screen_image(app: AppHandle) -> Option<String> {
    let known = current_settings(&app).lock_wallpaper.filter(|p| p.is_file());
    let cache = cache_dir(&app).ok()?;
    let read = tauri::async_runtime::spawn_blocking(move || wallpaper::lock_screen_image(&cache)).await.ok().flatten();
    read.or(known).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<Snapshot, String> {
    let previous = current_settings(&app);
    let mut next = settings;
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
    if next.lock_mode != "own" {
        state.editing_lock.store(false, SeqCst);
    }
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
    hide_glow(&app);
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
            overlay_rect: Mutex::new(None),
            target_rect: Mutex::new(None),
            editing_lock: AtomicBool::new(false),
            glow_turn: AtomicU64::new(0),
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
            if window.label() != "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                }
                return;
            }
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
            select_screen,
            select_lock,
            lock_screen_image,
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
