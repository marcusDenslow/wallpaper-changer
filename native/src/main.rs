#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod autostart;
mod core;
mod gfx;
#[path = "glow.rs"]
mod edge;
mod library;
mod render;
mod settings;
mod thumbs;
mod ui;
mod wallpaper;

use crate::core::{center, sorted_screens, Core, Snapshot};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{mpsc, Arc};
use std::time::Instant;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use ui::{Host, Msg, Ui, View};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

pub const APP_ID: &str = "com.wallpaperswitcher.app";

/// Debug log, only when WALLSWITCH_LOG is set.
pub fn log(msg: impl AsRef<str>) {
    use std::io::Write as _;
    if std::env::var_os("WALLSWITCH_LOG").is_none() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("wallswitch-debug.log")) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100_000).unwrap_or(0);
        let _ = writeln!(f, "{t:>6} [{}] {}", std::process::id(), msg.as_ref());
    }
}

#[derive(Debug)]
enum UserEvent {
    Wake,
    Hotkey,
    TrayClick,
    Menu(String),
    /// Forwarded request (from a second instance, or from the tray process to the overlay).
    Instance(u8),
    ChildExited,
}

/// Two processes: a tiny tray process (shortcut + tray icon) that lives all day, and an
/// overlay process that it starts on demand and that exits when the switcher closes, so
/// every byte the GPU driver allocated goes back to Windows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Tray,
    Overlay,
}

fn view_arg(v: View) -> &'static str {
    match v {
        View::Settings => "settings",
        _ => "picker",
    }
}

struct Overlay {
    win: gfx::GpuWindow,
    shown: bool,
    shown_at: Instant,
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    start: Instant,
    core: Core,
    ui: Ui,
    thumbs: thumbs::Thumbs,
    overlay: Option<Overlay>,
    msgs_tx: mpsc::Sender<Msg>,
    msgs_rx: mpsc::Receiver<Msg>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    tray: Option<TrayIcon>,
    tray_open: Option<MenuItem>,
    dialog_open: Arc<AtomicBool>,
    glow_turn: Arc<AtomicU64>,
    want_hide: bool,
    pending_open: Option<View>,
    quit: bool,
    started_quietly: bool,
    initialized: bool,
    role: Role,
    initial_view: View,
    child: Option<std::process::Child>,
    gpu_boot: Option<std::thread::JoinHandle<Result<gfx::GpuCore, String>>>,
    /// Window already shown (invisible) while the GPU finishes booting.
    waiting: Option<(Arc<winit::window::Window>, [i32; 4], View, Instant)>,
    gpu_ready: Arc<AtomicBool>,
    /// Overlay process: the closed window, hidden but kept (with its swapchain and
    /// thumbnails) until `idle_until`, so reopening is instant.
    parked: Option<Overlay>,
    idle_until: Option<Instant>,
}

/// The invisible early window gives up if the GPU isn't ready by then.
const BOOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

/// How long the overlay process stays warm after closing (then exits and frees everything).
const WARM_FOR: std::time::Duration = std::time::Duration::from_secs(120);

/// What the UI can ask of the app during a frame.
struct HostCtx<'a> {
    core: &'a mut Core,
    proxy: &'a EventLoopProxy<UserEvent>,
    msgs: &'a mpsc::Sender<Msg>,
    window: Option<&'a winit::window::Window>,
    hwnd: isize,
    hotkeys: &'a Option<GlobalHotKeyManager>,
    hotkey: &'a mut Option<HotKey>,
    tray_open: &'a Option<MenuItem>,
    dialog_open: &'a Arc<AtomicBool>,
    glow_turn: &'a Arc<AtomicU64>,
    visible: bool,
    want_hide: &'a mut bool,
    quit: &'a mut bool,
    overlay_size: (u32, u32),
    role: Role,
}

fn register_shortcut(
    hotkeys: &Option<GlobalHotKeyManager>,
    current: &mut Option<HotKey>,
    accel: &str,
    previous: Option<&str>,
) -> Result<(), String> {
    let Some(manager) = hotkeys else { return Err("Global shortcuts aren't available".into()) };
    let hotkey = HotKey::from_str(accel).map_err(|e| format!("\"{accel}\" isn't a valid shortcut ({e})"))?;
    if let Some(old) = current.take() {
        let _ = manager.unregister(old);
    }
    match manager.register(hotkey) {
        Ok(()) => {
            *current = Some(hotkey);
            Ok(())
        }
        Err(e) => {
            if let Some(old) = previous.and_then(|p| HotKey::from_str(p).ok()) {
                if manager.register(old).is_ok() {
                    *current = Some(old);
                }
            }
            Err(format!("Couldn't use {accel}. Another app might already be using it. ({e})"))
        }
    }
}

fn update_tray_label(item: &Option<MenuItem>, shortcut: &str) {
    if let Some(item) = item {
        item.set_text(format!("Open        {}", core::readable_shortcut(shortcut)));
    }
}

impl HostCtx<'_> {
    fn spawn<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static, wrap: impl FnOnce(T) -> Msg + Send + 'static) {
        let tx = self.msgs.clone();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = tx.send(wrap(job()));
            let _ = proxy.send_event(UserEvent::Wake);
        });
    }
}

impl Host for HostCtx<'_> {
    fn core(&mut self) -> &mut Core {
        self.core
    }

    fn select_screen(&mut self, index: usize) -> Snapshot {
        let screens = sorted_screens();
        let Some(screen) = screens.get(index) else { return self.core.snapshot() };
        self.core.target_rect = Some(screen.rect);
        self.core.editing_lock = false;
        if self.core.settings.follow_screen {
            if let Some(win) = self.window {
                let (x, y) = center(screen.rect);
                let monitor = win.available_monitors().find(|m| {
                    let r = monitor_rect(m);
                    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
                });
                if let Some(m) = monitor {
                    let from = self.core.overlay_rect;
                    let cursor = cursor_pos();
                    let rect = monitor_rect(&m);
                    win.set_outer_position(winit::dpi::PhysicalPosition::new(rect[0], rect[1]));
                    let _ = win.request_inner_size(winit::dpi::PhysicalSize::new((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32));
                    win.set_outer_position(winit::dpi::PhysicalPosition::new(rect[0], rect[1]));
                    self.core.overlay_rect = Some(rect);
                    win.focus_window();
                    if let (Some([l, t, r, b]), Some((cx, cy))) = (from, cursor) {
                        let fx = ((cx - l) as f64 / (r - l).max(1) as f64).clamp(0.0, 1.0);
                        let fy = ((cy - t) as f64 / (b - t).max(1) as f64).clamp(0.0, 1.0);
                        let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(
                            (fx * (rect[2] - rect[0]) as f64).round() as i32,
                            (fy * (rect[3] - rect[1]) as f64).round() as i32,
                        ));
                    }
                }
            }
        }
        self.flash_glow();
        self.core.snapshot()
    }

    fn spawn_apply(&mut self, path: String, everywhere: bool) {
        let job = self.core.apply_job(path, everywhere);
        self.spawn(job, Msg::Applied);
    }

    fn spawn_full_image(&mut self, path: String) {
        let (w, h) = self.overlay_size;
        let p2 = path.clone();
        self.spawn(
            move || {
                let img = image::ImageReader::open(&p2).ok()?.with_guessed_format().ok()?.decode().ok()?;
                // cover the screen at its own resolution, never more
                let scale = (w as f32 / img.width() as f32).max(h as f32 / img.height() as f32).min(1.0);
                let img = if scale < 1.0 {
                    img.resize((img.width() as f32 * scale).round() as u32, (img.height() as f32 * scale).round() as u32, image::imageops::FilterType::Triangle)
                } else {
                    img
                };
                Some(render::mip_chain(img.to_rgba8()))
            },
            move |r| match r {
                Some(levels) => Msg::FullImage(path, levels),
                None => Msg::FullImage(String::new(), Vec::new()),
            },
        );
    }

    fn spawn_lock_image(&mut self) {
        let job = self.core.lock_screen_job();
        self.spawn(job, Msg::LockImage);
    }

    fn pick_folder(&mut self) {
        let start = self.core.folder();
        let hwnd = self.hwnd;
        let open = self.dialog_open.clone();
        open.store(true, SeqCst);
        self.spawn(
            move || {
                let mut dialog = rfd::FileDialog::new().set_title("Choose your wallpaper folder");
                if let Some(dir) = start {
                    dialog = dialog.set_directory(dir);
                }
                let parent = Parent(hwnd);
                if hwnd != 0 {
                    dialog = dialog.set_parent(&parent);
                }
                let picked = dialog.pick_folder();
                open.store(false, SeqCst);
                picked.map(|p| p.to_string_lossy().into_owned())
            },
            Msg::Picked,
        );
    }

    fn save_settings(&mut self, next: settings::Settings) -> Result<Snapshot, String> {
        let hotkeys = self.hotkeys;
        let hotkey = &mut *self.hotkey;
        let result = self.core.save_settings(next, &mut |accel, prev| register_shortcut(hotkeys, hotkey, accel, prev));
        update_tray_label(self.tray_open, &self.core.settings.shortcut);
        if self.role == Role::Overlay {
            // we only checked the shortcut is free; the tray process owns it
            if let (Some(m), Some(h)) = (hotkeys, hotkey.take()) {
                let _ = m.unregister(h);
            }
            #[cfg(windows)]
            instance::signal(instance::TRAY_RELOAD);
        }
        result
    }

    fn flash_glow(&mut self) {
        let turn = self.glow_turn.fetch_add(1, SeqCst) + 1;
        if !self.visible || self.core.editing_lock || !self.core.settings.edit_glow {
            return;
        }
        let Some(target) = self.core.target_rect else { return };
        let (x, y) = center(target);
        if let Some([l, t, r, b]) = self.core.overlay_rect {
            if x >= l && x < r && y >= t && y < b {
                return;
            }
        }
        let counter = self.glow_turn.clone();
        edge::flash(target, move || counter.load(SeqCst) == turn);
    }

    fn hide(&mut self) {
        *self.want_hide = true;
    }

    fn quit(&mut self) {
        *self.quit = true;
        #[cfg(windows)]
        if self.role == Role::Overlay {
            instance::signal(instance::TRAY_QUIT);
        }
    }
}

/// Lets rfd parent its dialog to the overlay from another thread.
struct Parent(isize);
impl winit::raw_window_handle::HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<winit::raw_window_handle::WindowHandle<'_>, winit::raw_window_handle::HandleError> {
        let mut h = winit::raw_window_handle::Win32WindowHandle::new(std::num::NonZeroIsize::new(self.0).ok_or(winit::raw_window_handle::HandleError::Unavailable)?);
        h.hinstance = None;
        Ok(unsafe { winit::raw_window_handle::WindowHandle::borrow_raw(h.into()) })
    }
}
impl winit::raw_window_handle::HasDisplayHandle for Parent {
    fn display_handle(&self) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError> {
        Ok(winit::raw_window_handle::DisplayHandle::windows())
    }
}

fn monitor_rect(m: &winit::monitor::MonitorHandle) -> [i32; 4] {
    let (p, s) = (m.position(), m.size());
    [p.x, p.y, p.x + s.width as i32, p.y + s.height as i32]
}

fn cursor_pos() -> Option<(i32, i32)> {
    #[cfg(windows)]
    unsafe {
        let mut p = windows::Win32::Foundation::POINT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p).is_ok() {
            return Some((p.x, p.y));
        }
    }
    None
}

fn reduce_motion() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
        let mut on = windows::core::BOOL(1);
        if SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok() {
            return !on.as_bool();
        }
    }
    false
}

/// Gives freed heap memory back to Windows after the overlay closes. (No EmptyWorkingSet:
/// it frees nothing, it only pages memory out so Task Manager shows less, and the next
/// warm open has to fault it back in.)
fn trim_memory() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Memory::{GetProcessHeap, HeapCompact, HEAP_FLAGS};
        if let Ok(heap) = GetProcessHeap() {
            HeapCompact(heap, HEAP_FLAGS(0));
        }
    }
}

impl App {
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn visible(&self) -> bool {
        self.overlay.is_some()
    }

    fn host<'a>(
        core: &'a mut Core,
        proxy: &'a EventLoopProxy<UserEvent>,
        msgs: &'a mpsc::Sender<Msg>,
        overlay: Option<&'a Overlay>,
        hotkeys: &'a Option<GlobalHotKeyManager>,
        hotkey: &'a mut Option<HotKey>,
        tray_open: &'a Option<MenuItem>,
        dialog_open: &'a Arc<AtomicBool>,
        glow_turn: &'a Arc<AtomicU64>,
        want_hide: &'a mut bool,
        quit: &'a mut bool,
    ) -> HostCtx<'a> {
        let size = overlay.map(|o| o.win.window.inner_size()).map(|s| (s.width, s.height)).unwrap_or((1920, 1080));
        HostCtx {
            core,
            proxy,
            msgs,
            window: overlay.map(|o| &*o.win.window),
            hwnd: overlay.map(|o| o.win.hwnd()).unwrap_or(0),
            hotkeys,
            hotkey,
            tray_open,
            dialog_open,
            glow_turn,
            visible: overlay.is_some(),
            want_hide,
            quit,
            overlay_size: size,
            role: Role::Overlay,
        }
    }

    fn with_host<R>(&mut self, f: impl FnOnce(&mut Ui, &mut HostCtx) -> R) -> R {
        let mut host = Self::host(
            &mut self.core,
            &self.proxy,
            &self.msgs_tx,
            self.overlay.as_ref(),
            &self.hotkeys,
            &mut self.hotkey,
            &self.tray_open,
            &self.dialog_open,
            &self.glow_turn,
            &mut self.want_hide,
            &mut self.quit,
        );
        host.role = self.role;
        f(&mut self.ui, &mut host)
    }

    fn show_overlay(&mut self, event_loop: &ActiveEventLoop, view: View) {
        if let Some(o) = &self.overlay {
            o.win.window.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
            o.win.window.focus_window();
            let now = self.now();
            self.with_host(|ui, host| ui.open(view, host, now));
            self.request_redraw();
            return;
        }
        // monitor under the cursor
        let cursor = cursor_pos();
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        let monitor = cursor
            .and_then(|(x, y)| {
                monitors.iter().find(|m| {
                    let r = monitor_rect(m);
                    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
                })
            })
            .cloned()
            .or_else(|| event_loop.primary_monitor())
            .or_else(|| monitors.first().cloned());
        let rect = monitor.as_ref().map(monitor_rect).unwrap_or([0, 0, 1920, 1080]);
        log(format!("show_overlay {view:?} rect={rect:?}"));
        if let Some(w) = &mut self.waiting {
            w.2 = view;
            return;
        }
        self.idle_until = None;
        if let Some(mut o) = self.parked.take() {
            let size = o.win.window.inner_size();
            let current = o.win.window.outer_position().map(|p| [p.x, p.y, p.x + size.width as i32, p.y + size.height as i32]).ok();
            if current != Some(rect) {
                o.win.window.set_outer_position(winit::dpi::PhysicalPosition::new(rect[0], rect[1]));
                let _ = o.win.window.request_inner_size(winit::dpi::PhysicalSize::new((rect[2] - rect[0]) as u32, (rect[3] - rect[1]) as u32));
            }
            o.win.window.set_visible(true);
            o.win.window.focus_window();
            o.shown = false;
            self.core.overlay_rect = Some(rect);
            self.core.target_rect = Some(rect);
            self.core.editing_lock = false;
            self.overlay = Some(o);
            self.ui.reduce = reduce_motion();
            let now = self.now();
            self.with_host(|ui, host| ui.open(view, host, now));
            self.redraw();
            return;
        }
        let window = match gfx::GpuWindow::create_window(event_loop, rect) {
            Ok(w) => w,
            Err(e) => {
                log(&e);
                return;
            }
        };
        // invisible until the first frame is presented; showing it now overlaps the
        // ~100 ms Windows needs to show a window with the GPU setup. Clicks pass through
        // until it can draw, so it can never block the desktop.
        let _ = window.set_cursor_hittest(false);
        window.set_visible(true);
        window.focus_window();
        self.waiting = Some((window, rect, view, Instant::now()));
        self.finish_overlay(event_loop);
    }

    /// Attaches the GPU once it's ready. Called again on wake-ups until it is.
    fn finish_overlay(&mut self, event_loop: &ActiveEventLoop) {
        if self.waiting.is_none() || (self.gpu_boot.is_some() && !self.gpu_ready.load(SeqCst)) {
            return;
        }
        let Some((window, rect, view, _)) = self.waiting.take() else { return };
        let gpu = match self.gpu_boot.take() {
            Some(job) => job.join().unwrap_or_else(|_| Err("graphics thread panicked".into())),
            None => gfx::GpuCore::create(),
        };
        let _ = window.set_cursor_hittest(true);
        let win = match gpu.and_then(|core| gfx::GpuWindow::new(event_loop, window, core)) {
            Ok(w) => w,
            Err(e) => {
                // no graphics: don't linger with nothing on screen
                log(&e);
                self.quit = true;
                event_loop.exit();
                return;
            }
        };
        ui::style::install_fonts(&win.ctx);
        win.ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
        });
        let proxy = self.proxy.clone();
        win.ctx.set_request_repaint_callback(move |_| {
            let _ = proxy.send_event(UserEvent::Wake);
        });
        self.core.overlay_rect = Some(rect);
        self.core.target_rect = Some(rect);
        self.core.editing_lock = false;
        self.overlay = Some(Overlay { win, shown: false, shown_at: Instant::now() });
        self.ui.reduce = reduce_motion();
        let now = self.now();
        let (w, h) = {
            let o = self.overlay.as_ref().unwrap();
            let s = o.win.window.inner_size();
            let k = o.win.window.scale_factor() as f32;
            (s.width as f32 / k, s.height as f32 / k)
        };
        self.ui.set_viewport(w, h);
        self.with_host(|ui, host| ui.open(view, host, now));
        // hidden windows get no redraw events: paint the first frame ourselves, then show it
        self.redraw();
    }

    fn request_close(&mut self) {
        if !self.visible() {
            return;
        }
        let now = self.now();
        let _ = now;
        self.with_host(|ui, host| ui.close(host));
        self.request_redraw();
    }

    fn child_alive(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    /// Tray process: forward to the running overlay, or start one.
    fn route(&mut self, event_loop: &ActiveEventLoop, view: View, toggle: bool) {
        if self.role == Role::Overlay {
            if toggle {
                self.toggle(event_loop);
            } else {
                self.show_overlay(event_loop, view);
            }
            return;
        }
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
        }
        if self.child_alive() {
            #[cfg(windows)]
            instance::signal(if toggle {
                instance::OVERLAY_TOGGLE
            } else if view == View::Settings {
                instance::OVERLAY_SETTINGS
            } else {
                instance::OVERLAY_OPEN
            });
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        log("spawning overlay");
        match std::process::Command::new(exe).arg("--overlay").arg(view_arg(view)).spawn() {
            Ok(child) => {
                let pid = child.id();
                self.child = Some(child);
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    #[cfg(windows)]
                    unsafe {
                        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
                        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                            WaitForSingleObject(h, INFINITE);
                            let _ = windows::Win32::Foundation::CloseHandle(h);
                        }
                    }
                    let _ = proxy.send_event(UserEvent::ChildExited);
                });
            }
            Err(e) => log(format!("couldn't start overlay: {e}")),
        }
    }

    /// Tray process: pick up settings the overlay saved (shortcut, start-up).
    fn reload_settings(&mut self) {
        let next = settings::load();
        if next.shortcut != self.core.settings.shortcut || self.hotkey.is_none() {
            let prev = self.core.settings.shortcut.clone();
            match register_shortcut(&self.hotkeys, &mut self.hotkey, &next.shortcut, Some(&prev)) {
                Ok(()) => self.core.shortcut_error = None,
                Err(e) => self.core.shortcut_error = Some(e),
            }
        }
        self.core.settings = next;
        update_tray_label(&self.tray_open, &self.core.settings.shortcut);
    }

    fn toggle(&mut self, event_loop: &ActiveEventLoop) {
        if self.visible() && !matches!(self.ui.phase, ui::Phase::Closing | ui::Phase::Hidden) {
            self.request_close();
        } else if self.visible() {
            // closing: reopen once it's gone
            self.pending_open = Some(View::Picker);
        } else {
            self.show_overlay(event_loop, View::Picker);
        }
    }

    fn destroy_overlay(&mut self) {
        if let Some(o) = self.overlay.take() {
            o.win.window.set_visible(false);
            if let Some((_, tex)) = self.ui.full_image.take() {
                tex.destroy();
            }
            if self.role == Role::Overlay && !self.quit {
                // the process exits after WARM_FOR, which frees all of this
                self.parked = Some(o);
            } else {
                self.thumbs.release();
            }
        }
        self.glow_turn.fetch_add(1, SeqCst);
        trim_memory();
    }

    fn request_redraw(&self) {
        if let Some(o) = &self.overlay {
            o.win.window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let now = self.now();
        let Some(o) = self.overlay.as_mut() else { return };
        let fx = o.win.fx.clone();
        let window_arc = o.win.window.clone();
        let window = &*window_arc;
        let hwnd = o.win.hwnd();
        let t0 = Instant::now();
        let mut animating = false;
        {
            let mut host = HostCtx {
                core: &mut self.core,
                proxy: &self.proxy,
                msgs: &self.msgs_tx,
                window: Some(window),
                hwnd,
                hotkeys: &self.hotkeys,
                hotkey: &mut self.hotkey,
                tray_open: &self.tray_open,
                dialog_open: &self.dialog_open,
                glow_turn: &self.glow_turn,
                visible: true,
                want_hide: &mut self.want_hide,
                quit: &mut self.quit,
                overlay_size: (window.inner_size().width, window.inner_size().height),
                role: self.role,
            };
            let ui_state = &mut self.ui;
            let thumbs = &mut self.thumbs;
            o.win.paint(|ui| {
                animating = ui_state.frame(ui, &fx, thumbs, &mut host, now);
            });
        }
        if !o.shown {
            log(format!("first paint after {:?}", t0.elapsed()));
            o.shown = true;
            o.shown_at = Instant::now();
            window.focus_window();
        }
        if animating {
            window.request_redraw();
        }
    }

    fn after_events(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit {
            event_loop.exit();
            return;
        }
        if self.want_hide {
            self.want_hide = false;
            self.destroy_overlay();
            if let Some(v) = self.pending_open.take() {
                self.show_overlay(event_loop, v);
            } else if self.role == Role::Overlay {
                self.idle_until = Some(Instant::now() + WARM_FOR);
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        self.hotkeys = GlobalHotKeyManager::new().ok();
        if self.role == Role::Overlay {
            let view = self.initial_view;
            self.show_overlay(event_loop, view);
            if self.overlay.is_none() && self.waiting.is_none() {
                event_loop.exit();
            }
            return;
        }
        // global shortcut
        let shortcut = self.core.settings.shortcut.clone();
        if let Err(e) = register_shortcut(&self.hotkeys, &mut self.hotkey, &shortcut, None) {
            self.core.shortcut_error = Some(e);
        }
        // tray
        let open = MenuItem::with_id("open", "Open", true, None);
        let prefs = MenuItem::with_id("settings", "Settings…", true, None);
        let quit = MenuItem::with_id("quit", "Quit", true, None);
        let menu = Menu::new();
        let _ = menu.append_items(&[&open, &prefs, &PredefinedMenuItem::separator(), &quit]);
        let mut builder = TrayIconBuilder::new().with_tooltip("Wallpaper Switcher").with_menu(Box::new(menu)).with_menu_on_left_click(false);
        if let Ok(icon) = tray_icon::Icon::from_resource(1, None) {
            builder = builder.with_icon(icon);
        }
        self.tray = builder.build().ok();
        self.tray_open = Some(open);
        update_tray_label(&self.tray_open, &shortcut);
        self.core.warm_thumbnails();
        if !self.started_quietly {
            self.route(event_loop, View::Picker, false);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wake => {
                self.finish_overlay(event_loop);
                while let Ok(msg) = self.msgs_rx.try_recv() {
                    if !self.visible() {
                        continue;
                    }
                    self.with_host(|ui, host| ui.on_msg(msg, host));
                }
                self.request_redraw();
            }
            UserEvent::Hotkey | UserEvent::TrayClick => self.route(event_loop, View::Picker, true),
            UserEvent::Menu(id) => match id.as_str() {
                "open" => self.route(event_loop, View::Picker, false),
                "settings" => self.route(event_loop, View::Settings, false),
                "quit" => self.quit_all(),
                _ => {}
            },
            UserEvent::Instance(kind) => match (self.role, kind) {
                (_, 0) => self.route(event_loop, View::Picker, true),
                (_, 1) => self.route(event_loop, View::Settings, false),
                (_, 2) => self.route(event_loop, View::Picker, false),
                (Role::Tray, 3) => self.reload_settings(),
                (Role::Tray, _) => self.quit_all(),
                (Role::Overlay, _) => self.quit = true,
            },
            UserEvent::ChildExited => {
                self.child = None;
                self.reload_settings();
            }
        }
        self.after_events(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(o) = self.overlay.as_mut() else { return };
        match &event {
            WindowEvent::RedrawRequested => {
                self.redraw();
                self.after_events(event_loop);
                return;
            }
            WindowEvent::Resized(size) => {
                o.win.resize(*size);
                let k = o.win.window.scale_factor() as f32;
                self.ui.set_viewport(size.width as f32 / k, size.height as f32 / k);
            }
            WindowEvent::CloseRequested => {
                self.request_close();
                return;
            }
            WindowEvent::Focused(false) => {
                log("focus lost");
                let settled = o.shown && o.shown_at.elapsed().as_millis() > 350;
                if settled && !self.dialog_open.load(SeqCst) {
                    self.request_close();
                }
            }
            WindowEvent::Focused(true) => {}
            _ => {}
        }
        let Some(o) = self.overlay.as_mut() else { return };
        let response = o.win.on_window_event(&event);
        if response.repaint {
            o.win.window.request_redraw();
        }
        self.after_events(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.dialog_open.load(SeqCst) {
            event_loop.set_control_flow(ControlFlow::wait_duration(std::time::Duration::from_millis(100)));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
        if let Some((_, _, _, since)) = &self.waiting {
            if since.elapsed() >= BOOT_TIMEOUT {
                log("graphics didn't start in time; giving up");
                self.waiting = None;
                self.quit = true;
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(*since + BOOT_TIMEOUT));
        }
        if let (Some(t), None) = (self.idle_until, &self.overlay) {
            if Instant::now() >= t {
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(t));
        }
        if let Some(next) = self.ui.next_due() {
            if self.visible() {
                let wait = (next - self.now()).max(0.0);
                event_loop.set_control_flow(ControlFlow::wait_duration(std::time::Duration::from_micros((wait * 1000.0) as u64)));
                if wait <= 0.0 {
                    self.request_redraw();
                }
            }
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        if let winit::event::StartCause::ResumeTimeReached { .. } = cause {
            self.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.destroy_overlay();
    }
}

impl App {
    fn quit_all(&mut self) {
        #[cfg(windows)]
        if self.child_alive() {
            instance::signal(instance::OVERLAY_QUIT);
        }
        self.quit = true;
    }
}

// ---------- single instance ----------

#[cfg(windows)]
mod instance {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForMultipleObjects, EVENT_MODIFY_STATE, INFINITE};
    use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    /// Events the tray process listens to: toggle, settings, open, reload settings, quit.
    pub const TRAY: [PCWSTR; 5] = [
        w!("Local\\WallpaperSwitcher.Toggle"),
        w!("Local\\WallpaperSwitcher.Settings"),
        w!("Local\\WallpaperSwitcher.Open"),
        w!("Local\\WallpaperSwitcher.Reload"),
        w!("Local\\WallpaperSwitcher.Quit"),
    ];
    /// Events the overlay process listens to: toggle, settings, open, quit.
    pub const OVERLAY: [PCWSTR; 4] = [
        w!("Local\\WallpaperSwitcher.Overlay.Toggle"),
        w!("Local\\WallpaperSwitcher.Overlay.Settings"),
        w!("Local\\WallpaperSwitcher.Overlay.Open"),
        w!("Local\\WallpaperSwitcher.Overlay.Quit"),
    ];
    pub const TRAY_RELOAD: PCWSTR = TRAY[3];
    pub const TRAY_QUIT: PCWSTR = TRAY[4];
    pub const OVERLAY_TOGGLE: PCWSTR = OVERLAY[0];
    pub const OVERLAY_SETTINGS: PCWSTR = OVERLAY[1];
    pub const OVERLAY_OPEN: PCWSTR = OVERLAY[2];
    pub const OVERLAY_QUIT: PCWSTR = OVERLAY[3];

    pub fn signal(name: PCWSTR) -> bool {
        unsafe {
            match OpenEventW(EVENT_MODIFY_STATE, false, name) {
                Ok(ev) => {
                    let ok = SetEvent(ev).is_ok();
                    let _ = windows::Win32::Foundation::CloseHandle(ev);
                    ok
                }
                Err(_) => false,
            }
        }
    }

    /// Returns false if another instance is running (after forwarding `kind` to it).
    pub fn claim(kind: Option<u8>) -> bool {
        unsafe {
            let _mutex = CreateMutexW(None, false, w!("Local\\WallpaperSwitcher.Instance"));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                if let Some(k) = kind {
                    let _ = AllowSetForegroundWindow(ASFW_ANY);
                    signal(TRAY[k as usize]);
                }
                return false;
            }
            std::mem::forget(_mutex);
            true
        }
    }

    pub fn listen(names: &'static [PCWSTR], on: impl Fn(u8) + Send + 'static) {
        let handles: Vec<isize> = names
            .iter()
            .filter_map(|name| unsafe { CreateEventW(None, false, false, *name).ok() })
            .map(|h| h.0 as isize)
            .collect();
        std::thread::spawn(move || {
            let handles: Vec<HANDLE> = handles.into_iter().map(|h| HANDLE(h as _)).collect();
            loop {
                let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
                let i = r.0.wrapping_sub(WAIT_OBJECT_0.0);
                if (i as usize) < handles.len() {
                    on(i as u8);
                } else {
                    return;
                }
            }
        });
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let overlay_view = args.iter().position(|a| a == "--overlay").map(|i| match args.get(i + 1).map(|s| s.as_str()) {
        Some("settings") => View::Settings,
        _ => View::Picker,
    });
    let role = if overlay_view.is_some() { Role::Overlay } else { Role::Tray };
    let quiet = args.iter().any(|a| a == "--autostart" || a == "--hidden");
    let kind = if args.iter().any(|a| a == "--toggle") {
        Some(0)
    } else if args.iter().any(|a| a == "--settings") {
        Some(1)
    } else if quiet {
        None
    } else {
        Some(2)
    };
    #[cfg(windows)]
    if role == Role::Tray && !instance::claim(kind) {
        return;
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("Wallpaper Switcher failed to start");
    let proxy = event_loop.create_proxy();
    // overlay process: start the slow GPU setup immediately, in parallel with everything else
    let gpu_ready = Arc::new(AtomicBool::new(false));
    let gpu_boot = (role == Role::Overlay).then(|| {
        let p = proxy.clone();
        gfx::GpuCore::start(gpu_ready.clone(), move || {
            let _ = p.send_event(UserEvent::Wake);
        })
    });
    #[cfg(windows)]
    {
        let p = proxy.clone();
        let names: &'static [windows::core::PCWSTR] = if role == Role::Tray { &instance::TRAY } else { &instance::OVERLAY };
        instance::listen(names, move |k| {
            let _ = p.send_event(UserEvent::Instance(k));
        });
    }
    {
        let p = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                let _ = p.send_event(UserEvent::Hotkey);
            }
        }));
        let p = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = p.send_event(UserEvent::TrayClick);
            }
        }));
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = p.send_event(UserEvent::Menu(e.id.0.clone()));
        }));
    }

    let mut core = Core::new();
    let wake = {
        let p = proxy.clone();
        move || {
            let _ = p.send_event(UserEvent::Wake);
        }
    };
    let thumbs = thumbs::Thumbs::new(core.cache.clone(), wake);
    if role == Role::Overlay {
        core.preload(thumbs.handle());
    }
    let (tx, rx) = mpsc::channel();
    let mut app = App {
        proxy,
        start: Instant::now(),
        core,
        ui: Ui::new(reduce_motion()),
        thumbs,
        overlay: None,
        msgs_tx: tx,
        msgs_rx: rx,
        hotkeys: None,
        hotkey: None,
        tray: None,
        tray_open: None,
        dialog_open: Arc::new(AtomicBool::new(false)),
        glow_turn: Arc::new(AtomicU64::new(0)),
        want_hide: false,
        pending_open: None,
        quit: false,
        started_quietly: quiet,
        initialized: false,
        role,
        initial_view: overlay_view.unwrap_or(View::Picker),
        child: None,
        gpu_boot,
        gpu_ready,
        waiting: None,
        parked: None,
        idle_until: None,
    };
    let _ = event_loop.run_app(&mut app);
}
