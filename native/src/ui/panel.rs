//! Settings and welcome views (the `.settings` and `.welcome` sections).

use super::style::{self, Face};
use super::{Hit, Ui, View};
use crate::anim::{self, t, Tween};
use crate::render::Fx;
use crate::settings::Settings;
use egui::{Galley, Key, Modifiers, Painter, Pos2, Rect, Stroke, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ctl {
    Tab(u8),
    FolderChange,
    Shortcut,
    Autostart,
    FullJpeg,
    Layout(u8),
    Dim(u8),
    UiHidden,
    Chip(u8),
    EnterThisScreen,
    FollowScreen,
    EditGlow,
    MapSize(u8),
    LockSelect,
    LockOption(u8),
    LockSpot(u8),
    Back,
    Quit,
    WelcomeOpen,
    WelcomeChange,
    WelcomeAutostart,
    WelcomeDone,
}

#[derive(Clone, Debug)]
pub enum Patch {
    Bool(&'static str, bool),
    Layout(String),
    Dim(String),
    MapSize(String),
    LockMode(String),
    LockSpot(String),
}

impl Patch {
    pub fn apply(&self, s: &mut Settings) {
        match self {
            Patch::Bool(k, v) => {
                let v = *v;
                match *k {
                    "autostart" => s.autostart = v,
                    "fullJpegQuality" => s.full_jpeg_quality = v,
                    "uiHidden" => s.ui_hidden = v,
                    "hideHints" => s.hide_hints = v,
                    "hideTitle" => s.hide_title = v,
                    "hideDetails" => s.hide_details = v,
                    "hideMap" => s.hide_map = v,
                    "enterThisScreen" => s.enter_this_screen = v,
                    "followScreen" => s.follow_screen = v,
                    "editGlow" => s.edit_glow = v,
                    _ => {}
                }
            }
            Patch::Layout(v) => s.layout = v.clone(),
            Patch::Dim(v) => s.dim = v.clone(),
            Patch::MapSize(v) => s.map_size = v.clone(),
            Patch::LockMode(v) => s.lock_mode = v.clone(),
            Patch::LockSpot(v) => s.lock_spot = v.clone(),
        }
    }
}

pub enum Action {
    Run(&'static str),
    Set(Patch),
}

const TABS: [&str; 3] = ["General", "Appearance", "Screens"];
const LAYOUTS: [(&str, &str); 2] = [("slider", "Row"), ("grid", "Grid")];
const DIMS: [(&str, &str); 3] = [("clear", "Clear"), ("soft", "Soft"), ("deep", "Deep")];
const MAPS: [(&str, &str); 3] = [("small", "Small"), ("medium", "Medium"), ("large", "Large")];
const SPOTS: [(&str, &str); 2] = [("left", "Bottom left"), ("above", "Above")];
const LOCKS: [(&str, &str, &str); 3] = [
    ("off", "Leave it alone", "Wallpaper Switcher leaves it as it is."),
    ("main", "Same as main screen", "Changes along with your main screen."),
    ("own", "Its own wallpaper", "Pick it on the screen map, or press Ctrl + L."),
];
const CHIPS: [(&str, &str); 4] = [("hideHints", "Keyboard hints"), ("hideTitle", "Names"), ("hideDetails", "Details"), ("hideMap", "Screen map")];

pub struct PanelState {
    pub tab: u8,
    tab_x: Tween,
    tab_w: Tween,
    tab_instant: bool,
    pane_o: [Tween; 3],
    pane_y: [Tween; 3],
    pub select_open: bool,
    select_anim: Tween,
    select_up: bool,
    select_focus: usize,
    pub recording: bool,
    pub record_hint: String,
    pub error: String,
    pub focus: Option<Ctl>,
    pub focus_visible: bool,
    pub actions: Vec<Action>,
    pub welcome_autostart: bool,
    pub welcome_items: usize,
    anims: HashMap<Ctl, Tween>,
    hovers: HashMap<Ctl, Tween>,
    from_picker: bool,
    scroll: Tween,
    reveal: Option<f64>,
    panel_in: Tween,
    order: Vec<Ctl>,
    settings: Settings,
    multi: bool,
    tab_rects: [Rect; 3],
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            tab: 0,
            tab_x: Tween::new(0.0),
            tab_w: Tween::new(0.0),
            tab_instant: true,
            pane_o: [Tween::new(1.0), Tween::new(0.0), Tween::new(0.0)],
            pane_y: [Tween::new(0.0), Tween::new(8.0), Tween::new(8.0)],
            select_open: false,
            select_anim: Tween::new(0.0),
            select_up: false,
            select_focus: 0,
            recording: false,
            record_hint: String::new(),
            error: String::new(),
            focus: None,
            focus_visible: false,
            actions: Vec::new(),
            welcome_autostart: true,
            welcome_items: 0,
            anims: HashMap::new(),
            hovers: HashMap::new(),
            from_picker: false,
            scroll: Tween::new(0.0),
            reveal: None,
            panel_in: Tween::new(0.0),
            order: Vec::new(),
            settings: Settings::default(),
            multi: false,
            tab_rects: [Rect::NOTHING; 3],
        }
    }
}

impl PanelState {
    pub fn sync(&mut self, s: &Settings, multi: bool, _now: f64) {
        self.settings = s.clone();
        self.multi = multi;
    }

    pub fn show(&mut self, from_picker: bool, shortcut_error: Option<String>, s: &Settings, multi: bool, now: f64) {
        self.from_picker = from_picker;
        self.error = shortcut_error.unwrap_or_default();
        self.sync(s, multi, now);
        self.tab_instant = true;
        self.set_tab(self.tab, now, true);
        self.focus = Some(Ctl::Back);
        self.panel_in.set(1.0, now, t(520.0, anim::MOVE));
        self.scroll.jump(0.0);
    }

    pub fn phase_open(&mut self, view: View, now: f64) {
        if view == View::Welcome {
            self.reveal = Some(now);
        }
    }

    pub fn stop_recording(&mut self, _s: &Settings) {
        self.recording = false;
        self.record_hint.clear();
    }

    pub fn close_selects(&mut self, now: f64) {
        if self.select_open {
            self.select_open = false;
            self.select_anim.set(0.0, now, t(200.0, anim::MOVE));
        }
    }

    pub fn animating(&self, now: f64) -> bool {
        self.tab_x.active(now)
            || self.tab_w.active(now)
            || self.pane_o.iter().chain(self.pane_y.iter()).any(|t| t.active(now))
            || self.select_anim.active(now)
            || self.anims.values().chain(self.hovers.values()).any(|t| t.active(now))
            || self.scroll.active(now)
            || self.panel_in.active(now)
            || self.reveal.is_some_and(|r| now - r < 1100.0)
    }

    pub fn scroll(&mut self, dy: f32, now: f64) {
        let target = (self.scroll.target() + dy).max(0.0);
        self.scroll.set(target, now, t(160.0, anim::OUT));
    }

    fn set_tab(&mut self, tab: u8, now: f64, instant: bool) {
        if self.tab != tab {
            self.close_selects(now);
        }
        self.tab = tab;
        self.tab_instant = instant;
        for i in 0..3u8 {
            let on = i == tab;
            let (o, y) = (&mut self.pane_o[i as usize], &mut self.pane_y[i as usize]);
            if instant {
                o.jump(if on { 1.0 } else { 0.0 });
                y.jump(if on { 0.0 } else { 8.0 });
            } else if on {
                o.set(1.0, now, t(260.0, anim::OUT).delayed(90.0));
                y.set(0.0, now, t(420.0, anim::MOVE).delayed(90.0));
            } else {
                o.set(0.0, now, t(140.0, anim::OUT));
                y.set(8.0, now, t(0.0, anim::LINEAR).delayed(140.0));
            }
        }
    }

    pub fn step_tab(&mut self, step: i32, _multi: bool, now: f64) {
        let next = ((self.tab as i32 + step).rem_euclid(3)) as u8;
        let focus_tab = matches!(self.focus, Some(Ctl::Tab(_)));
        self.set_tab(next, now, false);
        if focus_tab {
            self.focus = Some(Ctl::Tab(next));
        }
    }

    fn anim(&mut self, c: Ctl, target: f32, now: f64, timing: anim::Timing) -> f32 {
        let tw = self.anims.entry(c).or_insert_with(|| Tween::new(target));
        tw.set(target, now, timing);
        tw.get(now)
    }

    fn hover(&mut self, c: Ctl, on: bool, now: f64) -> f32 {
        let tw = self.hovers.entry(c).or_insert_with(|| Tween::new(0.0));
        tw.set(if on { 1.0 } else { 0.0 }, now, t(160.0, anim::EASE));
        tw.get(now)
    }

    /// A click on a control (mouse) or activation (keyboard).
    pub fn click(&mut self, c: Ctl, now: f64) {
        self.focus = Some(c);
        let s = &self.settings;
        let flip = |k: &'static str, v: bool| Action::Set(Patch::Bool(k, !v));
        let act = match c {
            Ctl::Tab(i) => {
                self.set_tab(i, now, false);
                None
            }
            Ctl::FolderChange | Ctl::WelcomeChange => Some(Action::Run("pick-folder")),
            Ctl::WelcomeOpen => Some(Action::Run("open-folder")),
            Ctl::WelcomeDone => Some(Action::Run("finish-welcome")),
            Ctl::Back => Some(Action::Run("settings-back")),
            Ctl::Quit => Some(Action::Run("quit")),
            Ctl::Shortcut => {
                if self.recording {
                    self.recording = false;
                    self.record_hint.clear();
                } else {
                    self.recording = true;
                    self.record_hint = "Press a shortcut…".into();
                }
                None
            }
            Ctl::Autostart => Some(flip("autostart", s.autostart)),
            Ctl::FullJpeg => Some(flip("fullJpegQuality", s.full_jpeg_quality)),
            Ctl::UiHidden => Some(flip("uiHidden", s.ui_hidden)),
            Ctl::EnterThisScreen => Some(flip("enterThisScreen", s.enter_this_screen)),
            Ctl::FollowScreen => Some(flip("followScreen", s.follow_screen)),
            Ctl::EditGlow => Some(flip("editGlow", s.edit_glow)),
            Ctl::Chip(i) => {
                let v = match i {
                    0 => s.hide_hints,
                    1 => s.hide_title,
                    2 => s.hide_details,
                    _ => s.hide_map,
                };
                Some(Action::Set(Patch::Bool(CHIPS[i as usize].0, !v)))
            }
            Ctl::Layout(i) => Some(Action::Set(Patch::Layout(LAYOUTS[i as usize].0.into()))),
            Ctl::Dim(i) => Some(Action::Set(Patch::Dim(DIMS[i as usize].0.into()))),
            Ctl::MapSize(i) => Some(Action::Set(Patch::MapSize(MAPS[i as usize].0.into()))),
            Ctl::LockSpot(i) => Some(Action::Set(Patch::LockSpot(SPOTS[i as usize].0.into()))),
            Ctl::LockSelect => {
                if self.select_open {
                    self.close_selects(now);
                } else {
                    self.open_select(now);
                }
                None
            }
            Ctl::LockOption(i) => {
                self.close_selects(now);
                self.focus = Some(Ctl::LockSelect);
                if self.settings.lock_mode == LOCKS[i as usize].0 {
                    None
                } else {
                    Some(Action::Set(Patch::LockMode(LOCKS[i as usize].0.into())))
                }
            }
            Ctl::WelcomeAutostart => {
                self.welcome_autostart = !self.welcome_autostart;
                None
            }
        };
        if let Some(Action::Set(p)) = &act {
            p.apply(&mut self.settings);
        }
        if let Some(a) = act {
            self.actions.push(a);
        }
    }

    fn open_select(&mut self, now: f64) {
        self.select_open = true;
        self.select_anim.set(1.0, now, t(200.0, anim::MOVE));
        self.select_focus = LOCKS.iter().position(|l| l.0 == self.settings.lock_mode).unwrap_or(0);
    }

    /// Keyboard handling inside settings/welcome. Returns true if consumed.
    pub fn handle_key(&mut self, key: Key, m: Modifiers, view: View, now: f64) -> bool {
        // open select menu: roving focus over options
        if self.select_open {
            match key {
                Key::ArrowDown | Key::ArrowUp => {
                    let step = if key == Key::ArrowDown { 1 } else { LOCKS.len() - 1 };
                    self.select_focus = (self.select_focus + step) % LOCKS.len();
                }
                Key::Home => self.select_focus = 0,
                Key::End => self.select_focus = LOCKS.len() - 1,
                Key::Enter | Key::Space => self.click(Ctl::LockOption(self.select_focus as u8), now),
                Key::Tab => {
                    self.close_selects(now);
                    self.move_focus(if m.shift { -1 } else { 1 });
                }
                _ => {}
            }
            self.focus_visible = true;
            return true;
        }
        match key {
            Key::Tab => {
                self.focus_visible = true;
                self.move_focus(if m.shift { -1 } else { 1 });
                true
            }
            Key::ArrowLeft | Key::ArrowRight | Key::Home | Key::End if matches!(self.focus, Some(Ctl::Tab(_))) && view == View::Settings => {
                self.focus_visible = true;
                match key {
                    Key::Home => self.set_tab(0, now, false),
                    Key::End => self.set_tab(2, now, false),
                    Key::ArrowLeft => self.step_tab(-1, self.multi, now),
                    _ => self.step_tab(1, self.multi, now),
                }
                self.focus = Some(Ctl::Tab(self.tab));
                true
            }
            Key::ArrowDown | Key::ArrowUp | Key::Enter | Key::Space if self.focus == Some(Ctl::LockSelect) => {
                self.focus_visible = true;
                self.open_select(now);
                true
            }
            Key::Space | Key::Enter => {
                let Some(f) = self.focus else { return false };
                // checkboxes toggle on Space only; buttons on both
                let checkbox = matches!(
                    f,
                    Ctl::Autostart | Ctl::FullJpeg | Ctl::UiHidden | Ctl::EnterThisScreen | Ctl::FollowScreen | Ctl::EditGlow | Ctl::Chip(_) | Ctl::WelcomeAutostart
                );
                if checkbox && key == Key::Enter {
                    return view == View::Settings;
                }
                self.click(f, now);
                true
            }
            _ => false,
        }
    }

    fn move_focus(&mut self, step: i32) {
        if self.order.is_empty() {
            return;
        }
        let i = self.focus.and_then(|f| self.order.iter().position(|o| *o == f));
        let next = match i {
            None => {
                if step > 0 {
                    0
                } else {
                    self.order.len() - 1
                }
            }
            Some(i) => (i as i32 + step).rem_euclid(self.order.len() as i32) as usize,
        };
        self.focus = Some(self.order[next]);
    }
}

/// `keyToken()` + `handleRecording()` from app.js. Ok(None) = cancel (Esc), Err = hint.
pub fn record_key(key: Key, m: Modifiers) -> Option<Result<Option<String>, String>> {
    if key == Key::Escape {
        return Some(Ok(None));
    }
    let token: Option<String> = match key {
        k if k.name().len() == 1 && k.name().chars().all(|c| c.is_ascii_alphabetic()) => Some(k.name().to_string()),
        Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9 => {
            Some(key.name().trim_start_matches("Num").to_string())
        }
        Key::ArrowUp => Some("ArrowUp".into()),
        Key::ArrowDown => Some("ArrowDown".into()),
        Key::ArrowLeft => Some("ArrowLeft".into()),
        Key::ArrowRight => Some("ArrowRight".into()),
        Key::Home | Key::End | Key::PageUp | Key::PageDown | Key::Insert | Key::Delete => Some(key.name().to_string()),
        Key::Space => Some("Space".into()),
        Key::Backtick => Some("`".into()),
        Key::Minus => Some("-".into()),
        Key::Equals => Some("=".into()),
        Key::OpenBracket => Some("[".into()),
        Key::CloseBracket => Some("]".into()),
        Key::Backslash => Some("\\".into()),
        Key::Semicolon => Some(";".into()),
        Key::Quote => Some("'".into()),
        Key::Comma => Some(",".into()),
        Key::Period => Some(".".into()),
        Key::Slash => Some("/".into()),
        k => {
            let n = k.name();
            if n.starts_with('F') && n[1..].parse::<u8>().is_ok() {
                Some(n.to_string())
            } else {
                None
            }
        }
    };
    let mut mods = Vec::new();
    if m.ctrl {
        mods.push("Ctrl");
    }
    if m.alt {
        mods.push("Alt");
    }
    if m.shift {
        mods.push("Shift");
    }
    match token {
        Some(k) if !mods.is_empty() => Some(Ok(Some(format!("{}+{k}", mods.join("+"))))),
        _ => Some(Err("Add Ctrl, Alt, Shift or Win".into())),
    }
}

// ---------- painting primitives ----------

pub fn paint_button(p: &Painter, r: Rect, g: &Arc<Galley>, primary: bool, quiet: bool, hover: bool, a: f32, radius: f32) {
    paint_button_h(p, r, g, primary, quiet, if hover { 1.0 } else { 0.0 }, a, radius)
}

#[allow(clippy::too_many_arguments)]
fn paint_button_h(p: &Painter, r: Rect, g: &Arc<Galley>, primary: bool, quiet: bool, h: f32, a: f32, radius: f32) {
    let (fill, border, text) = if primary {
        (style::lerp_color(style::ink(1.0), style::white(1.0), h), egui::Color32::TRANSPARENT, style::rgba(style::DARK_TEXT, 1.0))
    } else if quiet {
        (style::white(0.06 * h), egui::Color32::TRANSPARENT, style::lerp_color(style::ink(0.66), style::ink(1.0), h))
    } else {
        (
            style::lerp_color(style::white(0.06), style::white(0.11), h),
            style::lerp_color(style::ink(0.14), style::white(0.22), h),
            style::ink(1.0),
        )
    };
    style::bordered(p, r, radius, style::fade(fill, a), style::fade(border, a));
    style::centered_text(p, r, g.clone(), style::fade(text, a));
}

const LINE: f32 = 1.45;

struct Field {
    label: String,
    hint: Option<String>,
    path: Option<String>,
    control: Control,
    border: bool,
    nested: bool,
    whole_row: Option<Ctl>,
}

enum Control {
    Button(Ctl, &'static str),
    Buttons(Vec<(Ctl, &'static str)>),
    Shortcut,
    Switch(Ctl, bool),
    Segmented(Vec<(Ctl, &'static str, bool)>),
    Select,
    Chips(Vec<(Ctl, &'static str, bool)>),
}

fn button_size(p: &Painter, label: &str) -> Vec2 {
    let g = style::text(label, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
    Vec2::new(g.size().x + 28.0 + 2.0, 14.0 * LINE + 16.0 + 2.0)
}

pub fn draw(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64) {
    let wo = ui.welcome_o.get(now);
    if wo > 0.002 {
        draw_welcome(ui, p, fx, screen, now, wo);
    }
    let so = ui.settings_o.get(now);
    if so > 0.002 {
        draw_settings(ui, p, fx, screen, now, so);
    }
    if ui.view != View::Settings {
        let ps = &mut ui.panel;
        ps.panel_in.set(0.0, now, t(520.0, anim::MOVE));
    }
}

fn fields_for(ui: &Ui, tab: u8) -> Vec<Field> {
    let s = &ui.panel.settings;
    let multi = ui.multi_screen();
    let map_on = multi || (s.lock_mode == "own" && !ui.snapshot.screens.is_empty());
    let mut v = Vec::new();
    let f = |label: &str, hint: Option<&str>, control: Control| Field {
        label: label.into(),
        hint: hint.map(|h| h.into()),
        path: None,
        control,
        border: true,
        nested: false,
        whole_row: None,
    };
    let sw = |label: &str, hint: &str, c: Ctl, on: bool| Field { whole_row: Some(c), ..f(label, Some(hint), Control::Switch(c, on)) };
    match tab {
        0 => {
            let mut folder = f("Wallpaper folder", None, Control::Button(Ctl::FolderChange, "Change"));
            folder.path = Some(ui.snapshot.folder.clone().unwrap_or_else(|| "Not set".into()));
            v.push(folder);
            v.push(f("Shortcut", Some("Opens the switcher from anywhere."), Control::Shortcut));
            if cfg!(target_os = "macos") {
                v.push(sw("Start at login", "Waits quietly in the menu bar until you press the shortcut.", Ctl::Autostart, s.autostart));
            } else {
                v.push(sw("Start with Windows", "Waits quietly in the tray until you press the shortcut.", Ctl::Autostart, s.autostart));
                v.push(sw("Full-quality JPEGs", "Stops Windows recompressing JPEGs to 85%.", Ctl::FullJpeg, s.full_jpeg_quality));
            }
        }
        1 => {
            v.push(f(
                "Layout",
                None,
                Control::Segmented(LAYOUTS.iter().enumerate().map(|(i, l)| (Ctl::Layout(i as u8), l.1, s.layout == l.0)).collect()),
            ));
            v.push(f(
                "Background",
                Some("Darkens the apps behind it."),
                Control::Segmented(DIMS.iter().enumerate().map(|(i, l)| (Ctl::Dim(i as u8), l.1, s.dim == l.0)).collect()),
            ));
            v.push(sw("Hide interface", "Ctrl + H toggles this while the switcher is open.", Ctl::UiHidden, s.ui_hidden));
            let vals = [s.hide_hints, s.hide_title, s.hide_details, s.hide_map];
            let chips = CHIPS
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 3 || map_on)
                .map(|(i, c)| (Ctl::Chip(i as u8), c.1, vals[i]))
                .collect();
            v.push(Field { nested: true, border: false, ..f("What gets hidden", None, Control::Chips(chips)) });
        }
        _ => {
            if multi {
                v.push(sw("Enter sets only this screen", "Swaps what Enter and Shift + Enter do.", Ctl::EnterThisScreen, s.enter_this_screen));
                v.push(sw("Move to the screen you edit", "Off: stays on the screen you opened it on.", Ctl::FollowScreen, s.follow_screen));
                v.push(sw("Flash when switching screens", "A quick white edge shows which screen you picked.", Ctl::EditGlow, s.edit_glow));
            }
            if map_on {
                v.push(f(
                    "Screen map size",
                    Some("The map in the corner."),
                    Control::Segmented(MAPS.iter().enumerate().map(|(i, l)| (Ctl::MapSize(i as u8), l.1, s.map_size == l.0)).collect()),
                ));
            }
            if !cfg!(target_os = "macos") {
                let hint = LOCKS.iter().find(|l| l.0 == s.lock_mode).unwrap_or(&LOCKS[0]).2;
                v.push(f("Lock screen", Some(hint), Control::Select));
                if s.lock_mode == "own" {
                    v.push(f(
                        "Lock screen on the map",
                        Some("Where its tile sits next to your screens."),
                        Control::Segmented(SPOTS.iter().enumerate().map(|(i, l)| (Ctl::LockSpot(i as u8), l.1, s.lock_spot == l.0)).collect()),
                    ));
                }
            }
            // `.pane > .field:first-child { border-top: 0 }` matches the DOM-first field even when hidden
            if multi {
                v[0].border = false;
            }
            return v;
        }
    }
    if let Some(first) = v.first_mut() {
        first.border = false;
    }
    v
}

fn control_size(ui: &Ui, p: &Painter, c: &Control, max_w: f32) -> Vec2 {
    match c {
        Control::Button(_, label) => button_size(p, label),
        Control::Buttons(list) => {
            let sizes: Vec<Vec2> = list.iter().map(|(_, l)| button_size(p, l)).collect();
            Vec2::new(sizes.iter().map(|s| s.x).sum::<f32>() + 8.0 * (sizes.len() as f32 - 1.0), sizes[0].y)
        }
        Control::Shortcut => {
            if ui.panel.recording {
                let g = style::text(&ui.panel.record_hint, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
                Vec2::new(g.size().x + 28.0 + 2.0, (14.0 * LINE + 12.0 + 2.0).max(36.0))
            } else {
                let keys = style::pretty_shortcut(&ui.panel.settings.shortcut);
                Vec2::new(style::kbd_width(p, &keys, &style::KBD) + 16.0 + 2.0, 36.0f32.max(22.0 + 12.0 + 2.0))
            }
        }
        Control::Switch(..) => Vec2::new(42.0, 24.0),
        Control::Segmented(opts) => {
            let w: f32 = opts
                .iter()
                .map(|(_, l, _)| style::text(l, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None).size().x + 28.0)
                .sum::<f32>()
                + 2.0 * (opts.len() as f32 - 1.0)
                + 6.0
                + 2.0;
            Vec2::new(w, 14.0 * LINE + 12.0 + 6.0 + 2.0)
        }
        Control::Select => {
            let chosen = LOCKS.iter().find(|l| l.0 == ui.panel.settings.lock_mode).unwrap_or(&LOCKS[0]).1;
            let g = style::text(chosen, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
            Vec2::new(14.0 + g.size().x + 10.0 + 12.0 + 12.0 + 2.0, (14.0 * LINE + 14.0 + 2.0).max(36.0))
        }
        Control::Chips(chips) => {
            let rows = chip_rows(p, chips, max_w);
            let h = rows.len() as f32 * chip_h() + (rows.len() as f32 - 1.0) * 6.0;
            let w = rows.iter().map(|r| r.iter().map(|(_, w)| *w).sum::<f32>() + 6.0 * (r.len() as f32 - 1.0)).fold(0.0, f32::max);
            Vec2::new(w, h)
        }
    }
}

fn chip_h() -> f32 {
    13.5 * LINE + 12.0 + 2.0
}

fn chip_rows(p: &Painter, chips: &[(Ctl, &'static str, bool)], max_w: f32) -> Vec<Vec<(usize, f32)>> {
    let mut rows: Vec<Vec<(usize, f32)>> = vec![vec![]];
    let mut row_w = 0.0;
    for (i, (_, l, _)) in chips.iter().enumerate() {
        let w = style::text(l, 13.5, Face::Sans500, style::ink(1.0)).galley(p, None).size().x + 26.0 + 2.0;
        let need = if rows.last().unwrap().is_empty() { w } else { row_w + 6.0 + w };
        if need > max_w && !rows.last().unwrap().is_empty() {
            rows.push(vec![(i, w)]);
            row_w = w;
        } else {
            rows.last_mut().unwrap().push((i, w));
            row_w = need;
        }
    }
    rows
}

struct Measured {
    field: Field,
    height: f32,
    label: Arc<Galley>,
    sub: Option<Arc<Galley>>,
    ctl: Vec2,
    text_h: f32,
}

fn measure_fields(ui: &Ui, p: &Painter, fields: Vec<Field>, width: f32, pad: f32) -> Vec<Measured> {
    fields
        .into_iter()
        .map(|f| {
            let ctl = control_size(ui, p, &f.control, width * 0.75);
            let text_w = (width - ctl.x - 24.0).max(40.0);
            let label = style::text(&f.label, 15.0, Face::Sans600, style::ink(1.0)).line(15.0 * LINE).job(None);
            let mut label = label;
            label.wrap.max_width = text_w;
            let label = p.layout_job(label);
            let sub = if let Some(path) = &f.path {
                Some(style::galley_truncate_start(p, style::text(path, 13.5, Face::Sans400, style::ink(0.66)).line(13.5 * LINE), text_w))
            } else {
                f.hint.as_ref().map(|h| {
                    let mut j = style::text(h, 13.5, Face::Sans400, style::ink(0.66)).line(13.5 * LINE).job(None);
                    j.wrap.max_width = text_w;
                    p.layout_job(j)
                })
            };
            let text_h = label.size().y + sub.as_ref().map(|s| s.size().y + 2.0).unwrap_or(0.0);
            let top_pad = if f.nested { 0.0 } else { pad };
            let height = text_h.max(ctl.y) + top_pad + pad + if f.border { 1.0 } else { 0.0 };
            Measured { field: f, height, label, sub, ctl, text_h }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_fields(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64, list: &[Measured], left: f32, top: f32, width: f32, pad: f32, a: f32, interactive: bool) -> Vec<Ctl> {
    let mut order = Vec::new();
    let mut y = top;
    let glow = ui.glow.get(now);
    for m in list {
        let f = &m.field;
        if f.border {
            p.hline(left..=left + width, y + 0.5, Stroke::new(1.0, style::white(0.07 * a)));
            y += 1.0;
        }
        let top_pad = if f.nested { 0.0 } else { pad };
        let inner_h = m.height - top_pad - pad - if f.border { 1.0 } else { 0.0 };
        let row = Rect::from_min_size(Pos2::new(left, y), Vec2::new(width, m.height - if f.border { 1.0 } else { 0.0 }));
        let cy = y + top_pad + inner_h / 2.0;
        // text block
        let ty = cy - m.text_h / 2.0;
        p.galley(Pos2::new(left, ty), fade_galley(p, &m.label, a), egui::Color32::WHITE);
        if let Some(sub) = &m.sub {
            p.galley(Pos2::new(left, ty + m.label.size().y + 2.0), fade_galley(p, sub, a), egui::Color32::WHITE);
        }
        if let (Some(c), true) = (f.whole_row, interactive) {
            ui.hits.push((row, Hit::Control(c), true));
        }
        let cr = Rect::from_min_size(Pos2::new(left + width - m.ctl.x, cy - m.ctl.y / 2.0), m.ctl);
        draw_control(ui, p, fx, screen, now, &f.control, cr, a, glow, interactive, &mut order);
        y += m.height - if f.border { 1.0 } else { 0.0 };
    }
    order
}

fn fade_galley(p: &Painter, g: &Arc<Galley>, a: f32) -> Arc<Galley> {
    if a >= 0.999 {
        return g.clone();
    }
    let mut job = (*g.job).clone();
    for s in &mut job.sections {
        s.format.color = style::fade(s.format.color, a);
    }
    p.layout_job(job)
}

#[allow(clippy::too_many_arguments)]
fn draw_control(ui: &mut Ui, p: &Painter, _fx: &Arc<Fx>, _screen: Rect, now: f64, c: &Control, r: Rect, a: f32, glow: [f32; 3], interactive: bool, order: &mut Vec<Ctl>) {
    let fv = ui.panel.focus_visible;
    let focus = ui.panel.focus;
    let hovered = ui.hovered.clone();
    let is_hover = |c: Ctl| hovered == Some(Hit::Control(c));
    match c {
        Control::Button(ctl, label) => {
            let h = ui.panel.hover(*ctl, is_hover(*ctl), now);
            let g = style::text(label, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
            paint_button_h(p, r, &g, false, false, h, a, 9.0);
            push(ui, order, r, *ctl, interactive, true);
            if fv && focus == Some(*ctl) {
                style::focus_ring(p, r, 9.0, glow, 2.0);
            }
        }
        Control::Buttons(list) => {
            let mut x = r.min.x;
            for (ctl, label) in list {
                let s = button_size(p, label);
                let br = Rect::from_min_size(Pos2::new(x, r.min.y), s);
                let h = ui.panel.hover(*ctl, is_hover(*ctl), now);
                let g = style::text(label, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
                paint_button_h(p, br, &g, false, false, h, a, 9.0);
                push(ui, order, br, *ctl, interactive, true);
                if fv && focus == Some(*ctl) {
                    style::focus_ring(p, br, 9.0, glow, 2.0);
                }
                x += s.x + 8.0;
            }
        }
        Control::Shortcut => {
            let ctl = Ctl::Shortcut;
            let h = ui.panel.hover(ctl, is_hover(ctl), now);
            let rec = ui.panel.recording;
            let fill = style::lerp_color(style::white(0.06), style::white(0.11), h);
            let border = if rec {
                style::mix(glow, 0.2, [255, 255, 255], 1.0)
            } else {
                style::lerp_color(style::ink(0.14), style::white(0.22), h)
            };
            style::bordered(p, r, 9.0, style::fade(fill, a), style::fade(border, a));
            if rec {
                let g = style::text(&ui.panel.record_hint, 14.0, Face::Sans500, style::ink(0.66)).galley(p, None);
                style::centered_text(p, r, g, style::ink(0.66 * a));
            } else {
                let keys = style::pretty_shortcut(&ui.panel.settings.shortcut);
                style::draw_kbds(p, &keys, &style::KBD, r.min.x + 9.0, r.center().y, a);
            }
            push(ui, order, r, ctl, interactive, true);
            if fv && focus == Some(ctl) {
                style::focus_ring(p, r, 9.0, glow, 2.0);
            }
        }
        Control::Switch(ctl, on) => {
            let k = ui.panel.anim(*ctl, if *on { 1.0 } else { 0.0 }, now, t(260.0, anim::MOVE));
            let kc = ui.panel.anim(Ctl::Tab(100 + ctl_index(*ctl)), if *on { 1.0 } else { 0.0 }, now, t(200.0, anim::EASE));
            let off = style::white(0.14);
            let onc = style::mix(glow, 0.28, [0x4a, 0x4d, 0x58], 1.0);
            p.rect_filled(r, 12.0, style::fade(style::lerp_color(off, onc, kc), a));
            let knob = Pos2::new(r.min.x + 3.0 + 9.0 + 18.0 * k, r.center().y);
            p.circle_filled(knob, 9.0, style::ink(a));
            push(ui, order, r, *ctl, interactive, false);
            if fv && focus == Some(*ctl) {
                style::focus_ring(p, r, 12.0, glow, 2.0);
            }
        }
        Control::Segmented(opts) => {
            style::bordered(p, r, 10.0, style::white(0.06 * a), style::ink(0.14 * a));
            let mut x = r.min.x + 1.0 + 3.0;
            for (ctl, label, on) in opts {
                let g = style::text(label, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
                let br = Rect::from_min_size(Pos2::new(x, r.min.y + 4.0), Vec2::new(g.size().x + 28.0, r.height() - 8.0));
                let sel = ui.panel.anim(*ctl, if *on { 1.0 } else { 0.0 }, now, t(160.0, anim::EASE));
                let h = ui.panel.hover(*ctl, is_hover(*ctl), now);
                if sel > 0.001 {
                    p.rect_filled(br, 7.0, style::ink(sel * a));
                }
                let text = style::lerp_color(style::lerp_color(style::ink(0.66), style::ink(1.0), h), style::rgba(style::DARK_TEXT, 1.0), sel);
                style::centered_text(p, br, g, style::fade(text, a));
                push(ui, order, br, *ctl, interactive, true);
                if fv && focus == Some(*ctl) {
                    style::focus_ring(p, br, 7.0, glow, 2.0);
                }
                x += br.width() + 2.0;
            }
        }
        Control::Select => {
            let ctl = Ctl::LockSelect;
            let open = ui.panel.select_open;
            let h = ui.panel.hover(ctl, is_hover(ctl) || open, now);
            style::bordered(p, r, 10.0, style::fade(style::lerp_color(style::white(0.06), style::white(0.1), h), a), style::ink(0.14 * a));
            let chosen = LOCKS.iter().find(|l| l.0 == ui.panel.settings.lock_mode).unwrap_or(&LOCKS[0]).1;
            let g = style::text(chosen, 14.0, Face::Sans500, style::ink(a)).galley(p, None);
            p.galley(Pos2::new(r.min.x + 15.0, r.center().y - g.size().y / 2.0), g, egui::Color32::WHITE);
            // chevron 12x12, rotates 180° when open
            let rot = ui.panel.anim(Ctl::Tab(200), if open { 1.0 } else { 0.0 }, now, t(220.0, anim::MOVE));
            let c = Pos2::new(r.max.x - 13.0 - 6.0, r.center().y);
            let pts = [(-3.0, -1.5), (0.0, 1.5), (3.0, -1.5)].map(|(x, y): (f32, f32)| {
                let ang = rot * std::f32::consts::PI;
                Pos2::new(c.x + x * ang.cos() - y * ang.sin(), c.y + x * ang.sin() + y * ang.cos())
            });
            p.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.5, style::ink(0.66 * a))));
            push(ui, order, r, ctl, interactive, true);
            if fv && focus == Some(ctl) && !open {
                style::focus_ring(p, r, 10.0, glow, 2.0);
            }
            ui.panel_select_rect = r;
        }
        Control::Chips(chips) => {
            let rows = chip_rows(p, chips, r.width().max(1.0) + 0.5);
            let mut y = r.min.y;
            for row in rows {
                let w: f32 = row.iter().map(|(_, w)| *w).sum::<f32>() + 6.0 * (row.len() as f32 - 1.0);
                let mut x = r.max.x - w;
                for (i, cw) in row {
                    let (ctl, label, on) = chips[i];
                    let cr = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw, chip_h()));
                    let h = ui.panel.hover(ctl, is_hover(ctl), now);
                    let sel = ui.panel.anim(ctl, if on { 1.0 } else { 0.0 }, now, t(160.0, anim::EASE));
                    let fill = style::lerp_color(style::white(0.06), style::ink(1.0), sel);
                    let border = style::lerp_color(style::ink(0.14), egui::Color32::TRANSPARENT, sel);
                    style::bordered(p, cr, cr.height() / 2.0, style::fade(fill, a), style::fade(border, a));
                    let text = style::lerp_color(style::lerp_color(style::ink(0.66), style::ink(1.0), h), style::rgba(style::DARK_TEXT, 1.0), sel);
                    let g = style::text(label, 13.5, Face::Sans500, style::ink(1.0)).galley(p, None);
                    style::centered_text(p, cr, g, style::fade(text, a));
                    push(ui, order, cr, ctl, interactive, true);
                    if fv && focus == Some(ctl) {
                        style::focus_ring(p, cr, cr.height() / 2.0, glow, 2.0);
                    }
                    x += cw + 6.0;
                }
                y += chip_h() + 6.0;
            }
        }
    }
}

fn ctl_index(c: Ctl) -> u8 {
    match c {
        Ctl::Autostart => 1,
        Ctl::FullJpeg => 2,
        Ctl::UiHidden => 3,
        Ctl::EnterThisScreen => 4,
        Ctl::FollowScreen => 5,
        Ctl::EditGlow => 6,
        Ctl::WelcomeAutostart => 7,
        _ => 0,
    }
}

fn push(ui: &mut Ui, order: &mut Vec<Ctl>, r: Rect, c: Ctl, interactive: bool, hit: bool) {
    order.push(c);
    if interactive && hit {
        ui.hits.push((r, Hit::Control(c), true));
    }
}

fn draw_settings(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64, a: f32) {
    let (vw, vh) = (screen.width(), screen.height());
    let short = vh <= 860.0;
    let narrow = vw <= 760.0;
    let pad_x = if narrow { 22.0 } else { 36.0 };
    let pad_top = if narrow { 26.0 } else if short { 26.0 } else { 34.0 };
    let pad_bottom = if narrow { 20.0 } else { 26.0 };
    let field_pad = if short { 11.0 } else { 16.0 };
    let title_size = if narrow || short { 36.0 } else { 44.0 };
    let title_mb = if short { 10.0 } else { 18.0 };
    let (sec_top, sec_bottom) = if short { (24.0, 80.0) } else { (40.0, 96.0) };
    let panel_w = 560.0f32.min(vw - 48.0);
    let inner_w = panel_w - pad_x * 2.0;
    let interactive = ui.view == View::Settings;

    // measure all panes; the pane area is as tall as the tallest one (grid-area: 1 / 1)
    let panes: Vec<Vec<Measured>> = (0..3u8).map(|tab| measure_fields(ui, p, fields_for(ui, tab), inner_w, field_pad)).collect();
    let pane_h = panes.iter().map(|m| m.iter().map(|f| f.height).sum::<f32>()).fold(0.0, f32::max);
    let title = style::text("Settings", title_size, Face::Serif, style::ink(1.0)).letter(-0.01).line(title_size * 1.1).galley(p, None);
    let tabs_h = 4.0 + 14.5 * LINE + 13.0 + 1.0;
    let error = if ui.panel.error.is_empty() {
        None
    } else {
        let mut j = style::text(&ui.panel.error, 13.5, Face::Sans400, style::rgba(style::ERROR, 1.0)).line(13.5 * LINE).job(None);
        j.wrap.max_width = inner_w;
        Some(p.layout_job(j))
    };
    let back_label = if ui.panel.from_picker { "Back to wallpapers" } else { "Done" };
    let btn = button_size(p, back_label);
    let footer_h = 14.0 + 1.0 + 20.0 + btn.y;
    let panel_h = pad_top + title.size().y + title_mb + tabs_h + pane_h + error.as_ref().map(|e| 4.0 + e.size().y).unwrap_or(0.0) + footer_h + pad_bottom + 2.0;

    let avail = vh - sec_top - sec_bottom;
    let max_scroll = (panel_h - avail).max(0.0);
    if ui.panel.scroll.target() > max_scroll {
        ui.panel.scroll.jump(max_scroll);
    }
    let scroll = ui.panel.scroll.get(now);
    let top0 = sec_top + ((avail - panel_h) / 2.0).max(0.0) - scroll;
    let k = ui.panel.panel_in.get(now);
    let sc = 0.985 + 0.015 * k;
    let dy = 16.0 * (1.0 - k);
    let base = Rect::from_min_size(Pos2::new((vw - panel_w) / 2.0, top0), Vec2::new(panel_w, panel_h));
    let pr = Rect::from_center_size(base.center() + Vec2::new(0.0, dy), base.size() * sc);
    // transform: scale the content around the centre — we approximate by offsetting content (scale ≥ 0.985)
    let off = pr.min - base.min;
    let glow = ui.glow.get(now);

    style::shadow(p, fx, screen, pr, 20.0, 60.0, 140.0, -50.0, style::black(0.95 * a));
    style::shadow(p, fx, screen, pr, 20.0, 0.0, 160.0, -60.0, style::glow_alpha(glow, 0.22 * a));
    style::bordered(p, pr, 20.0, style::rgba([17, 18, 23], 0.94 * a), style::white(0.08 * a));
    if interactive {
        ui.hits.push((pr, Hit::PanelInner, true));
    }
    let left = base.min.x + pad_x + off.x;
    let mut y = base.min.y + 1.0 + pad_top + off.y;
    style::draw_galley(p, fx, screen, Pos2::new(left, y), &title, a, None);
    y += title.size().y + title_mb;

    // tabs
    let mut order: Vec<Ctl> = Vec::new();
    let tab_top = y;
    let mut x = left;
    let glow_fv = ui.panel.focus_visible;
    for (i, name) in TABS.iter().enumerate() {
        let ctl = Ctl::Tab(i as u8);
        let on = ui.panel.tab == i as u8;
        let hov = ui.hovered == Some(Hit::Control(ctl));
        let h = ui.panel.hover(ctl, hov, now);
        let sel = ui.panel.anim(Ctl::Tab(10 + i as u8), if on { 1.0 } else { 0.0 }, now, t(180.0, anim::EASE));
        let color = style::lerp_color(style::lerp_color(style::ink(0.42), style::ink(0.66), h), style::ink(1.0), sel);
        let g = style::text(name, 14.5, Face::Sans500, style::ink(1.0)).galley(p, None);
        let r = Rect::from_min_size(Pos2::new(x, tab_top), Vec2::new(g.size().x, tabs_h - 1.0));
        p.galley_with_override_text_color(Pos2::new(x, tab_top + 4.0 + (14.5 * LINE - g.size().y) / 2.0), g.clone(), style::fade(color, a));
        ui.panel.tab_rects[i] = r;
        if on {
            order.push(ctl);
        }
        if interactive {
            ui.hits.push((r, Hit::Control(ctl), true));
        }
        if glow_fv && ui.panel.focus == Some(ctl) {
            style::focus_ring(p, r.shrink2(Vec2::new(0.0, 4.0)), 4.0, glow, 4.0);
        }
        x += g.size().x + 28.0;
    }
    p.hline(left..=left + inner_w, tab_top + tabs_h - 0.5, Stroke::new(1.0, style::white(0.07 * a)));
    let tr = ui.panel.tab_rects[ui.panel.tab as usize];
    let timing = if ui.panel.tab_instant || ui.reduce { t(0.0, anim::LINEAR) } else { t(420.0, anim::MOVE) };
    ui.panel.tab_instant = false;
    ui.panel.tab_x.set(tr.min.x - left, now, timing);
    ui.panel.tab_w.set(tr.width(), now, timing);
    let ix = left + ui.panel.tab_x.get(now);
    p.rect_filled(Rect::from_min_size(Pos2::new(ix, tab_top + tabs_h - 2.0), Vec2::new(ui.panel.tab_w.get(now), 2.0)), 2.0, style::ink(a));
    y = tab_top + tabs_h;

    // panes
    let mut select_info = None;
    for (i, list) in panes.iter().enumerate() {
        let po = ui.panel.pane_o[i].get(now);
        let py = ui.panel.pane_y[i].get(now);
        let active = ui.panel.tab == i as u8;
        if po <= 0.002 && !active {
            continue;
        }
        let o = draw_fields(ui, p, fx, screen, now, list, left, y + py, inner_w, field_pad, a * po, interactive && active);
        if active {
            order.extend(o);
            if ui.panel.tab == 2 {
                select_info = Some(ui.panel_select_rect);
            }
        }
    }
    y += pane_h;

    if let Some(e) = &error {
        y += 4.0;
        p.galley(Pos2::new(left, y), fade_galley(p, e, a), egui::Color32::WHITE);
        y += e.size().y;
    }

    // footer
    y += 14.0;
    p.hline(left..=left + inner_w, y + 0.5, Stroke::new(1.0, style::white(0.07 * a)));
    y += 1.0 + 20.0;
    let back = Rect::from_min_size(Pos2::new(left, y), btn);
    let gb = style::text(back_label, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
    let hb = ui.panel.hover(Ctl::Back, ui.hovered == Some(Hit::Control(Ctl::Back)), now);
    paint_button_h(p, back, &gb, true, false, hb, a, 9.0);
    let qs = button_size(p, "Quit Wallpaper Switcher");
    let quit = Rect::from_min_size(Pos2::new(left + inner_w - qs.x, y), qs);
    let gq = style::text("Quit Wallpaper Switcher", 14.0, Face::Sans500, style::ink(1.0)).galley(p, None);
    let hq = ui.panel.hover(Ctl::Quit, ui.hovered == Some(Hit::Control(Ctl::Quit)), now);
    paint_button_h(p, quit, &gq, false, true, hq, a, 9.0);
    order.push(Ctl::Back);
    order.push(Ctl::Quit);
    if interactive {
        ui.hits.push((back, Hit::Control(Ctl::Back), true));
        ui.hits.push((quit, Hit::Control(Ctl::Quit), true));
    }
    if glow_fv && ui.panel.focus == Some(Ctl::Back) {
        style::focus_ring(p, back, 9.0, glow, 2.0);
    }
    if glow_fv && ui.panel.focus == Some(Ctl::Quit) {
        style::focus_ring(p, quit, 9.0, glow, 2.0);
    }
    if interactive {
        ui.panel.order = order;
    }

    // the dropdown menu floats above everything in the panel
    if let Some(br) = select_info {
        draw_select_menu(ui, p, fx, screen, now, br, a, interactive);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_select_menu(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64, br: Rect, a: f32, interactive: bool) {
    let k = ui.panel.select_anim.get(now);
    let open = ui.panel.select_open;
    let oo = if open {
        ui.panel.anim(Ctl::Tab(201), 1.0, now, t(150.0, anim::OUT))
    } else {
        ui.panel.anim(Ctl::Tab(201), 0.0, now, t(150.0, anim::OUT))
    };
    if oo <= 0.002 && !open {
        return;
    }
    let gals: Vec<_> = LOCKS.iter().map(|l| style::text(l.1, 14.0, Face::Sans400, style::ink(1.0)).galley(p, None)).collect();
    let opt_h = 14.0 * LINE + 16.0;
    let w = gals.iter().map(|g| g.size().x + 44.0).fold(br.width(), f32::max) + 8.0 + 2.0;
    let h = opt_h * LOCKS.len() as f32 + 8.0 + 2.0;
    if open && k < 0.05 {
        let room = screen.height() - br.max.y - 100.0;
        ui.panel.select_up = room < h && br.min.y > h + 20.0;
    }
    let up = ui.panel.select_up;
    let sc = 0.98 + 0.02 * k;
    let dy = if up { 4.0 } else { -4.0 } * (1.0 - k);
    let base = if up {
        Rect::from_min_size(Pos2::new(br.max.x - w, br.min.y - 6.0 - h), Vec2::new(w, h))
    } else {
        Rect::from_min_size(Pos2::new(br.max.x - w, br.max.y + 6.0), Vec2::new(w, h))
    };
    // transform-origin: top right (or bottom right)
    let anchor = if up { base.right_bottom() } else { base.right_top() };
    let r = Rect::from_min_max(anchor + (base.min - anchor) * sc, anchor + (base.max - anchor) * sc).translate(Vec2::new(0.0, dy));
    let ma = a * oo;
    style::shadow(p, fx, screen, r, 12.0, 24.0, 60.0, -18.0, style::black(0.9 * ma));
    style::bordered(p, r, 12.0, style::rgba([36, 37, 44], ma), style::white(0.1 * ma));
    let mut y = r.min.y + 1.0 + 4.0 * sc;
    for (i, g) in gals.iter().enumerate() {
        let or = Rect::from_min_size(Pos2::new(r.min.x + 1.0 + 4.0 * sc, y), Vec2::new(r.width() - 2.0 - 8.0 * sc, opt_h * sc));
        let ctl = Ctl::LockOption(i as u8);
        let hov = ui.hovered == Some(Hit::Control(ctl)) || (ui.panel.select_focus == i && open);
        let chosen = ui.panel.settings.lock_mode == LOCKS[i].0;
        if hov {
            p.rect_filled(or, 8.0, style::white(0.07 * ma));
        }
        let color = if hov || chosen { style::ink(ma) } else { style::ink(0.66 * ma) };
        p.galley_with_override_text_color(Pos2::new(or.min.x + 12.0, or.center().y - g.size().y / 2.0), g.clone(), color);
        if chosen {
            // check mark: 8×4 L rotated -45°
            let c = Pos2::new(or.max.x - 12.0 - 4.0, or.center().y - 1.0);
            p.add(egui::Shape::line(
                vec![c + Vec2::new(-3.5, -0.5), c + Vec2::new(-1.0, 2.0), c + Vec2::new(3.5, -2.5)],
                Stroke::new(1.5, color),
            ));
        }
        if interactive && open {
            ui.hits.push((or, Hit::Control(ctl), true));
        }
        y += opt_h * sc;
    }
}

// ---------- welcome ----------

fn draw_welcome(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64, a: f32) {
    let (vw, vh) = (screen.width(), screen.height());
    let inner_w = 600.0f32.min(vw - 48.0);
    let left = (vw - inner_w) / 2.0;
    let interactive = ui.view == View::Welcome;
    let glow = ui.glow.get(now);
    let (reveal_at, reduce) = (ui.panel.reveal, ui.reduce);
    let reveal = |i: usize| -> (f32, f32) {
        match reveal_at {
            None => (0.0, 16.0),
            Some(r) => {
                let el = (now - r) as f32 - i as f32 * 90.0;
                let o = anim::OUT.at((el / 520.0).clamp(0.0, 1.0));
                let ty = 16.0 * (1.0 - anim::MOVE.at((el / 760.0).clamp(0.0, 1.0)));
                if reduce {
                    (o, 0.0)
                } else {
                    (o, ty)
                }
            }
        }
    };
    // measure
    let title_size = clampf(52.0, vw * 0.06, 108.0);
    // `.welcome-inner` is min(600px, 100%) wide, so at 108px the title wraps onto two lines
    let title = style::text("Wallpaper Switcher", title_size, Face::Serif, style::ink(1.0)).letter(-0.016).line(title_size);
    let title_lines = style::wrap_lines(p, title, inner_w);
    let title_h: f32 = title_lines.iter().map(|g| g.size().y).sum();
    let keys = style::pretty_shortcut(&ui.panel.settings.shortcut);
    let press = style::text("Press", 17.0, Face::Sans400, style::ink(0.66)).galley(p, None);
    let rest = style::text("from anywhere to open it.", 17.0, Face::Sans400, style::ink(0.66)).galley(p, None);
    let kw = style::kbd_width(p, &keys, &style::KBD_LARGE);
    let lead_w = press.size().x + 10.0 + kw + 10.0 + rest.size().x;
    let lead_h = (17.0 * LINE).max(30.0);
    let n = ui.panel.welcome_items;
    let folder = ui.snapshot.folder.clone().unwrap_or_default();
    let hint = if n > 0 {
        format!("Found {}. Folders inside it show up as stacks.", super::plural(n, "wallpaper"))
    } else {
        "Drop your wallpapers in here. Folders inside it show up as stacks.".into()
    };
    let card_inner = inner_w - 48.0;
    let mut fields = vec![
        Field {
            label: "Wallpaper folder".into(),
            hint: None,
            path: Some(folder),
            control: Control::Buttons(vec![(Ctl::WelcomeOpen, "Open"), (Ctl::WelcomeChange, "Change")]),
            border: false,
            nested: false,
            whole_row: None,
        },
        Field {
            label: (if cfg!(target_os = "macos") { "Start at login" } else { "Start with Windows" }).into(),
            hint: Some((if cfg!(target_os = "macos") { "Waits in the menu bar so the shortcut always works." } else { "Waits in the tray so the shortcut always works." }).into()),
            path: None,
            control: Control::Switch(Ctl::WelcomeAutostart, ui.panel.welcome_autostart),
            border: true,
            nested: false,
            whole_row: Some(Ctl::WelcomeAutostart),
        },
    ];
    let field_pad = if vh <= 860.0 { 11.0 } else { 16.0 };
    let measured = {
        let mut m = measure_fields(ui, p, std::mem::take(&mut fields), card_inner, field_pad);
        // the folder field has a third line: the hint under the path
        let h = style::text(&hint, 13.5, Face::Sans400, style::ink(0.66)).line(13.5 * LINE).job(None);
        let mut h = h;
        h.wrap.max_width = (card_inner - m[0].ctl.x - 24.0).max(40.0);
        let hg = p.layout_job(h);
        m[0].text_h += hg.size().y + 2.0;
        m[0].height += hg.size().y + 2.0;
        (m, hg)
    };
    let (measured, hint_g) = measured;
    let card_h = 8.0 + measured.iter().map(|m| m.height).sum::<f32>() + 2.0;
    let done_label = if n > 0 { "Show my wallpapers" } else { "Done" };
    let dg = style::text(done_label, 15.0, Face::Sans500, style::ink(1.0)).galley(p, None);
    let done_size = Vec2::new(dg.size().x + 48.0 + 2.0, 15.0 * LINE + 24.0 + 2.0);
    let total = title_h + 22.0 + lead_h + 46.0 + card_h + 30.0 + done_size.y;
    let area_top = 40.0;
    let area_h = vh - 40.0 - 110.0;
    let top = area_top + ((area_h - total) / 2.0).max(0.0);
    let inner = Rect::from_min_size(Pos2::new(left, top), Vec2::new(inner_w, total));
    if interactive {
        ui.hits.push((inner, Hit::PanelInner, true));
    }
    let mut order = Vec::new();
    // 1. title
    let (o1, t1) = reveal(0);
    let sh = style::TextShadow { offset: 0.0, sharp_alpha: 0.0, blur: 30.0, blur_alpha: 0.5, blur_offset: 2.0 };
    let mut ty = top + t1;
    for line in &title_lines {
        let tx = left + (inner_w - line.size().x) / 2.0;
        style::draw_galley(p, fx, screen, Pos2::new(tx, ty), line, a * o1, Some(sh));
        ty += line.size().y;
    }
    let mut y = top + title_h + 22.0;
    // 2. lead
    let (o2, t2) = reveal(1);
    let la = a * o2;
    let mut x = left + (inner_w - lead_w) / 2.0;
    let cy = y + lead_h / 2.0 + t2;
    let lsh = style::TextShadow { offset: 0.0, sharp_alpha: 0.0, blur: 14.0, blur_alpha: 0.5, blur_offset: 1.0 };
    style::draw_galley(p, fx, screen, Pos2::new(x, cy - press.size().y / 2.0), &press, la, Some(lsh));
    x += press.size().x + 10.0;
    style::draw_kbds(p, &keys, &style::KBD_LARGE, x, cy, la);
    x += kw + 10.0;
    style::draw_galley(p, fx, screen, Pos2::new(x, cy - rest.size().y / 2.0), &rest, la, Some(lsh));
    y += lead_h + 46.0;
    // 3. card
    let (o3, t3) = reveal(2);
    let ca = a * o3;
    let card = Rect::from_min_size(Pos2::new(left, y + t3), Vec2::new(inner_w, card_h));
    style::shadow(p, fx, screen, card, 18.0, 50.0, 120.0, -50.0, style::black(0.95 * ca));
    style::bordered(p, card, 18.0, style::rgba([17, 18, 23], 0.94 * ca), style::white(0.08 * ca));
    let o = draw_fields(ui, p, fx, screen, now, &measured, left + 24.0 + 1.0, y + t3 + 1.0 + 4.0, card_inner - 2.0, field_pad, ca, interactive);
    order.extend(o);
    // the folder hint line under the path
    let f0 = &measured[0];
    let text_top = y + t3 + 5.0 + field_pad + ((f0.height - field_pad * 2.0) - f0.text_h) / 2.0;
    let hy = text_top + f0.label.size().y + 2.0 + f0.sub.as_ref().map(|s| s.size().y + 2.0).unwrap_or(0.0);
    p.galley(Pos2::new(left + 25.0, hy), fade_galley(p, &hint_g, ca), egui::Color32::WHITE);
    y += card_h + 30.0;
    // 4. actions
    let (o4, t4) = reveal(3);
    let br = Rect::from_min_size(Pos2::new(left + (inner_w - done_size.x) / 2.0, y + t4), done_size);
    let h = ui.panel.hover(Ctl::WelcomeDone, ui.hovered == Some(Hit::Control(Ctl::WelcomeDone)), now);
    paint_button_h(p, br, &dg, true, false, h, a * o4, 11.0);
    if interactive {
        ui.hits.push((br, Hit::Control(Ctl::WelcomeDone), true));
    }
    order.push(Ctl::WelcomeDone);
    if ui.panel.focus_visible && ui.panel.focus == Some(Ctl::WelcomeDone) {
        style::focus_ring(p, br, 11.0, glow, 2.0);
    }
    if interactive {
        ui.panel.order = order;
    }
}

fn clampf(lo: f32, v: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}
