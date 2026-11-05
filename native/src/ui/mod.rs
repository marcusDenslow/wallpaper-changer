//! The overlay: a port of app.js. State and behaviour live here; drawing is in
//! `picker.rs` (row, grid, caption, hints, map, apply) and `panel.rs` (settings, welcome).

mod panel;
mod picker;
pub mod style;

use crate::anim::{self, t, Timing, Tween};
use crate::core::{Applied, Core, Snapshot};
use crate::library::Wallpaper;
use crate::render::Fx;
use crate::settings::Settings;
use crate::thumbs::{self, Thumbs};
use egui::{Key, Modifiers, Pos2, Rect};
use std::collections::HashMap;
use std::sync::Arc;

/// What the UI needs from the app around it.
pub trait Host {
    fn core(&mut self) -> &mut Core;
    fn select_screen(&mut self, index: usize) -> Snapshot;
    fn spawn_apply(&mut self, path: String, everywhere: bool);
    fn spawn_full_image(&mut self, path: String);
    fn spawn_lock_image(&mut self);
    fn pick_folder(&mut self);
    fn save_settings(&mut self, next: Settings) -> Result<Snapshot, String>;
    fn flash_glow(&mut self);
    fn hide(&mut self);
    fn quit(&mut self);
}

pub enum Msg {
    Applied(Result<Applied, String>),
    LockImage(Option<String>),
    Picked(Option<String>),
    FullImage(String, Vec<(u32, u32, Vec<u8>)>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Hidden,
    Opening,
    Open,
    Applying,
    Closing,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Picker,
    Settings,
    Welcome,
}

#[derive(Clone, Debug)]
pub enum Entry {
    Folder { key: String, name: String, items: Vec<usize> },
    Wall { key: String, item: usize },
}

impl Entry {
    fn key(&self) -> &str {
        match self {
            Entry::Folder { key, .. } | Entry::Wall { key, .. } => key,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Section {
    pub name: Option<String>,
    pub items: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub error: bool,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Geo {
    pub w: f32,
    pub h: f32,
    pub small: f32,
    pub gap: f32,
    pub radius: i32,
    pub vw: f32,
    pub vh: f32,
    pub stage_c: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct P {
    pub x: f32,
    pub y: f32,
    pub s: f32,
    pub o: f32,
    pub b: f32,
}

const COL_S0: f32 = 0.9;
const COL_PEEK: f32 = 0.2;
const COL_SHRINK: f32 = 0.075;
const COL_DEPTH: i32 = 3;
const REVEAL_STEP: f32 = 90.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cls {
    Normal,
    Slow,
    Drift,
    Folding,
    FoldingKeep,
}

#[derive(Clone, Debug)]
pub enum CardKind {
    Wall { item: usize },
    Folder { entry: String },
}

/// A `.card` element with its CSS-transitioned properties.
pub struct Card {
    pub key: String,
    pub kind: CardKind,
    pub order: u64,
    pub tgt: P,
    pub x: Tween,
    pub y: Tween,
    pub s: Tween,
    pub op: Tween,
    pub b: Tween,
    pub ey: Tween,
    pub k: Tween,
    pub sel: Tween,
    /// folder sheets fan out when selected (`.sheet` transform, 600ms)
    pub fsel: Tween,
    pub enter: bool,
    pub leaving: bool,
    pub no_anim: bool,
    pub cls: Cls,
    pub delay: f32,
    pub z: i32,
    pub selected: bool,
    pub remove_at: Option<f64>,
    /// time each sheet's image became visible (fade-in), or instant when it already existed
    pub img_at: [Option<f64>; 3],
    pub created: f64,
}

impl Card {
    fn new(key: String, kind: CardKind, order: u64, p: P, created: f64) -> Self {
        Self {
            key,
            kind,
            order,
            tgt: p,
            x: Tween::new(p.x),
            y: Tween::new(p.y),
            s: Tween::new(p.s),
            op: Tween::new(p.o),
            b: Tween::new(p.b),
            ey: Tween::new(0.0),
            k: Tween::new(1.0),
            sel: Tween::new(0.0),
            fsel: Tween::new(0.0),
            enter: false,
            leaving: false,
            no_anim: false,
            cls: Cls::Normal,
            delay: 0.0,
            z: 0,
            selected: false,
            remove_at: None,
            img_at: [None; 3],
            created,
        }
    }

    pub fn pointer_none(&self) -> bool {
        self.tgt.o == 0.0 || self.leaving
    }

    /// The style recalc: start transitions for every property whose target changed.
    fn commit(&mut self, now: f64, phase: Phase, reduce: bool) {
        let closing = phase == Phase::Closing;
        let applying = phase == Phase::Applying;
        let o = if self.enter || self.leaving || closing || applying { 0.0 } else { self.tgt.o };
        let ey = if closing {
            18.0
        } else if self.enter && !reduce {
            30.0
        } else {
            0.0
        };
        let k = if self.enter && !reduce { 0.94 } else { 1.0 };
        let sel = if self.selected { 1.0 } else { 0.0 };
        if self.no_anim {
            self.x.jump(self.tgt.x);
            self.y.jump(self.tgt.y);
            self.s.jump(self.tgt.s);
            self.op.jump(o);
            self.b.jump(self.tgt.b);
            self.ey.jump(ey);
            self.k.jump(k);
            self.sel.jump(sel);
            self.fsel.jump(sel);
            return;
        }
        let d = self.delay;
        let (mut tf, mut to, mut tb, mut ts) = match self.cls {
            Cls::Normal => (t(600.0, anim::MOVE), t(420.0, anim::OUT), t(420.0, anim::OUT), t(420.0, anim::OUT)),
            Cls::Slow => (t(820.0, anim::MOVE), t(560.0, anim::OUT), t(700.0, anim::OUT), t(560.0, anim::OUT)),
            Cls::Drift => (t(420.0, anim::OUT), t(300.0, anim::OUT), t(420.0, anim::OUT), t(420.0, anim::OUT)),
            Cls::Folding => (
                t(520.0, anim::FOLD),
                t(220.0, anim::OUT).delayed(280.0),
                t(520.0, anim::FOLD),
                t(420.0, anim::OUT),
            ),
            Cls::FoldingKeep => (t(520.0, anim::FOLD), t(300.0, anim::OUT), t(520.0, anim::OUT), t(420.0, anim::FOLD)),
        };
        if !matches!(self.cls, Cls::Folding) {
            tf = tf.delayed(d);
            to = to.delayed(d);
            tb = tb.delayed(d);
            ts = ts.delayed(d);
        }
        if matches!(self.cls, Cls::FoldingKeep) {
            tf.delay = 0.0;
            to.delay = 0.0;
            tb.delay = 0.0;
            ts.delay = 0.0;
        }
        if closing {
            let c = t(200.0, anim::IN);
            tf = c;
            to = c;
            tb = c;
            ts = c;
        } else if applying {
            to = Timing { dur: 240.0, delay: 0.0, ease: to.ease };
        }
        if reduce {
            tf = t(0.0, anim::LINEAR);
            tb = Timing { dur: 0.0, ..tb };
            ts = Timing { dur: 0.0, ..ts };
        }
        self.x.set(self.tgt.x, now, tf);
        self.y.set(self.tgt.y, now, tf);
        self.s.set(self.tgt.s, now, tf);
        self.ey.set(ey, now, tf);
        self.k.set(k, now, tf);
        self.op.set(o, now, to);
        self.b.set(self.tgt.b, now, tb);
        self.sel.set(sel, now, ts);
        self.fsel.set(sel, now, if reduce { t(0.0, anim::LINEAR) } else { t(600.0, anim::MOVE) });
    }

    fn active(&self, now: f64) -> bool {
        [&self.x, &self.y, &self.s, &self.op, &self.b, &self.ey, &self.k, &self.sel, &self.fsel].iter().any(|t| t.active(now))
    }
}

/// Grid tile (`.grid .tile`).
pub struct Tile {
    pub item: usize,
    pub enter: Tween,
    pub enter_delay: f32,
    pub hover: Tween,
    pub sel: Tween,
    pub selk: Tween,
    pub img_at: Option<f64>,
    pub rect: Rect,
    pub created: f64,
}

pub struct Heading {
    pub name: String,
    pub count: usize,
    pub enter: Tween,
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    CaptionSwap,
    CollapseFold(u64),
    CollapseReveal(u64),
    EnterRelease(u64),
    EnterCaption(u64),
    EnterHints(u64),
    EnterDelays(u64),
    OpenShow(u64),
    OpenDone(u64),
    CloseDone,
    ApplyAnimDone,
    ApplyClose,
    MoveSelect(usize, u64),
    MoveEnter(u64),
    TitleRemove(u64),
}

pub struct TitleLine {
    pub id: u64,
    pub text: String,
    pub y: Tween,
    pub o: Tween,
    pub leaving: bool,
}

#[derive(Clone, Debug)]
pub struct Hint {
    pub keys: Vec<String>,
    pub label: String,
    pub optional: bool,
}

pub struct Apply {
    pub path: String,
    pub from: Rect,
    pub from_radius: f32,
    pub start: f64,
    pub reverse_at: Option<f64>,
    pub result: Option<Result<Applied, String>>,
    pub anim_done: bool,
    pub to_lock: bool,
    pub fade: Tween,
}

/// Interactive regions recorded while drawing, used for hit-testing the next input.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Card(String),
    Tile(usize),
    Caption,
    MapScreen(usize),
    MapLock,
    Hints,
    Button(&'static str),
    PanelInner,
    Control(panel::Ctl),
}

pub struct Ui {
    pub phase: Phase,
    pub view: View,
    settings_from_picker: Option<bool>,
    pub snapshot: Snapshot,
    pub settings: Settings,
    pub items: Vec<Wallpaper>,
    pub entries: Vec<Entry>,
    pub sections: Vec<Section>,
    pub flat: Vec<usize>,
    pub row: usize,
    pub col: Option<usize>,
    pub gindex: usize,
    pub filter: String,
    anchor: Option<String>,
    pub status: Option<Status>,
    pub busy: bool,
    moving: bool,
    pub geo: Geo,
    pub cards: Vec<Card>,
    order: u64,
    pub tiles: Vec<Tile>,
    pub headings: Vec<Heading>,
    pub grid_scroll: Tween,
    pub grid_content: f32,
    steps: Vec<(f64, Step)>,
    token: u64,
    fold_token: u64,
    enter_token: u64,
    now: f64,
    pub reduce: bool,
    // caption
    pub capmode_side: bool,
    pub caption_swap: Tween,
    pub caption_enter: Tween,
    pub caption_close: Tween,
    pub titles: Vec<TitleLine>,
    title_id: u64,
    last_title: String,
    pub meta: Vec<(String, MetaKind)>,
    pub show_filter_chip: bool,
    pub empty_actions: bool,
    pub empty: bool,
    pub hints: Vec<Hint>,
    pub hints_enter: Tween,
    // whole-layer opacities
    pub backdrop: Tween,
    pub picker_o: Tween,
    pub settings_o: Tween,
    pub welcome_o: Tween,
    pub glow: anim::ColorTween,
    pub edge_flash: Option<f64>,
    pub apply: Option<Apply>,
    pub full_image: Option<(String, crate::render::Tex)>,
    pending_full: Option<(String, Vec<(u32, u32, Vec<u8>)>)>,
    pub lock_image: Option<Option<String>>,
    lock_loading: bool,
    // settings / welcome
    pub panel: panel::PanelState,
    // input
    pub hits: Vec<(Rect, Hit, bool)>,
    press: Option<(Hit, Pos2)>,
    pub hovered: Option<Hit>,
    pub pointer: Option<Pos2>,
    wheel_acc: f32,
    wheel_axis_col: bool,
    wheel_last: f64,
    wheel_step: f64,
    pub opened_at: f64,
    open_wait_until: f64,
    open_wait_path: Option<String>,
    pub expanded_attr: bool,
    pub grid_center_pending: bool,
    pub grid_scroll_nearest: bool,
    pub grid_origin: Pos2,
    pub grid_view_h: f32,
    pub first_frame: bool,
    pub ui_attr_hidden: bool,
    pub pk: picker::Extra,
    pub panel_select_rect: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MetaKind {
    Plain,
    Strong,
    Error,
}

pub fn same_path(a: &str, b: &str) -> bool {
    thumbs::key(a) == thumbs::key(b)
}

fn same_opt(a: Option<&str>, b: Option<&str>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if same_path(a, b))
}

pub fn prettify(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if matches!(c, '_' | '-' | '.') { ' ' } else { c }).collect();
    // strip resolutions like 1920x1080 / 3840 × 2160
    let words: Vec<&str> = s.split(' ').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let is_num = |x: &str| (3..=5).contains(&x.len()) && x.chars().all(|c| c.is_ascii_digit());
        let lower = w.to_lowercase();
        // forms: "1920x1080", "1920", "x", "1080"
        if let Some((a, b)) = lower.split_once(['x', '×']) {
            if is_num(a) && is_num(b) {
                i += 1;
                continue;
            }
            if is_num(a) && b.is_empty() && words.get(i + 1).is_some_and(|n| is_num(n)) {
                i += 2;
                continue;
            }
            if a.is_empty() && is_num(b) && out.last().is_some_and(|p| is_num(p)) {
                out.pop();
                i += 1;
                continue;
            }
        }
        if is_num(w) && words.get(i + 1).is_some_and(|n| matches!(*n, "x" | "X" | "×")) && words.get(i + 2).is_some_and(|n| is_num(n)) {
            i += 3;
            continue;
        }
        out.push(w.to_string());
        i += 1;
    }
    s = out.into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        s = name.to_string();
    }
    if s == s.to_lowercase() {
        let mut c = s.chars();
        if let Some(f) = c.next() {
            s = f.to_uppercase().chain(c).collect();
        }
    }
    s
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn natural_cmp_ci(a: &str, b: &str) -> std::cmp::Ordering {
    // localeCompare(numeric, base) ≈ case-insensitive natural compare
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut n1 = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    n1.push(c);
                    ai.next();
                }
                let mut n2 = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    n2.push(c);
                    bi.next();
                }
                let (t1, t2) = (n1.trim_start_matches('0'), n2.trim_start_matches('0'));
                let o = t1.len().cmp(&t2.len()).then_with(|| t1.cmp(t2));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

impl Ui {
    pub fn new(reduce: bool) -> Self {
        Self {
            phase: Phase::Hidden,
            view: View::Picker,
            settings_from_picker: None,
            snapshot: Snapshot::default(),
            settings: Settings::default(),
            items: Vec::new(),
            entries: Vec::new(),
            sections: Vec::new(),
            flat: Vec::new(),
            row: 0,
            col: None,
            gindex: 0,
            filter: String::new(),
            anchor: None,
            status: None,
            busy: false,
            moving: false,
            geo: Geo { w: 560.0, h: 315.0, small: 340.0, gap: 22.0, radius: 4, vw: 1920.0, vh: 1080.0, stage_c: 486.0 },
            cards: Vec::new(),
            order: 0,
            tiles: Vec::new(),
            headings: Vec::new(),
            grid_scroll: Tween::new(0.0),
            grid_content: 0.0,
            steps: Vec::new(),
            token: 0,
            fold_token: 0,
            enter_token: 0,
            now: 0.0,
            reduce,
            capmode_side: false,
            caption_swap: Tween::new(1.0),
            caption_enter: Tween::new(1.0),
            caption_close: Tween::new(1.0),
            titles: Vec::new(),
            title_id: 0,
            last_title: String::new(),
            meta: Vec::new(),
            show_filter_chip: false,
            empty_actions: false,
            empty: false,
            hints: Vec::new(),
            hints_enter: Tween::new(1.0),
            backdrop: Tween::new(0.0),
            picker_o: Tween::new(1.0),
            settings_o: Tween::new(0.0),
            welcome_o: Tween::new(0.0),
            glow: anim::ColorTween::new(style::DEFAULT_GLOW),
            edge_flash: None,
            apply: None,
            full_image: None,
            pending_full: None,
            lock_image: None,
            lock_loading: false,
            panel: panel::PanelState::default(),
            hits: Vec::new(),
            press: None,
            hovered: None,
            pointer: None,
            wheel_acc: 0.0,
            wheel_axis_col: false,
            wheel_last: -1e9,
            wheel_step: -1e9,
            opened_at: 0.0,
            open_wait_until: 0.0,
            open_wait_path: None,
            expanded_attr: false,
            grid_center_pending: false,
            grid_scroll_nearest: false,
            grid_origin: Pos2::ZERO,
            grid_view_h: 0.0,
            first_frame: true,
            ui_attr_hidden: false,
            pk: picker::Extra::default(),
            panel_select_rect: Rect::NOTHING,
        }
    }

    // ---------- small accessors (app.js top) ----------

    fn screens(&self) -> &[crate::core::ScreenInfo] {
        &self.snapshot.screens
    }
    pub fn multi_screen(&self) -> bool {
        self.screens().len() > 1
    }
    pub fn lock_own(&self) -> bool {
        self.settings.lock_mode == "own"
    }
    pub fn editing_lock(&self) -> bool {
        self.lock_own() && self.snapshot.editing_lock
    }
    pub fn map_on(&self) -> bool {
        self.multi_screen() || (self.lock_own() && !self.screens().is_empty())
    }
    fn current_path(&self) -> Option<String> {
        if self.editing_lock() {
            self.settings.lock_wallpaper.as_ref().map(|p| p.to_string_lossy().into_owned())
        } else {
            self.snapshot.current.clone()
        }
    }
    pub fn is_current(&self, item: usize) -> bool {
        let cur = self.current_path();
        same_opt(Some(&self.items[item].path), cur.as_deref())
    }
    fn editing_screen(&self) -> Option<&crate::core::ScreenInfo> {
        self.screens().iter().find(|s| s.editing).or_else(|| self.screens().iter().find(|s| s.here))
    }
    fn screen_name(s: Option<&crate::core::ScreenInfo>) -> String {
        match s {
            None => "this screen".into(),
            Some(s) if s.here => "this screen".into(),
            Some(s) => format!("screen {}", s.number),
        }
    }
    fn other_screen_showing(&self, item: usize) -> Option<usize> {
        let lock = self.editing_lock();
        self.screens()
            .iter()
            .find(|s| (lock || !s.editing) && same_opt(s.current.as_deref(), Some(&self.items[item].path)))
            .map(|s| s.number)
    }
    pub fn layout_grid(&self) -> bool {
        self.settings.layout == "grid"
    }
    fn entry(&self) -> Option<&Entry> {
        self.entries.get(self.row)
    }
    pub fn is_expanded(&self) -> bool {
        !self.layout_grid() && self.col.is_some() && matches!(self.entry(), Some(Entry::Folder { .. }))
    }
    fn folder_items(&self) -> &[usize] {
        match self.entry() {
            Some(Entry::Folder { items, .. }) => items,
            _ => &[],
        }
    }
    pub fn selected_item(&self) -> Option<usize> {
        if self.layout_grid() {
            return self.flat.get(self.gindex).copied();
        }
        match self.entry()? {
            Entry::Wall { item, .. } => Some(*item),
            Entry::Folder { items, .. } => self.col.and_then(|c| items.get(c).copied()),
        }
    }
    fn focus_item(&self) -> Option<usize> {
        if let Some(i) = self.selected_item() {
            return Some(i);
        }
        match self.entry() {
            Some(Entry::Folder { items, .. }) => items.first().copied(),
            _ => None,
        }
    }

    // ---------- model ----------

    fn build_model(&mut self) {
        let q = self.filter.trim().to_lowercase();
        if !q.is_empty() {
            let m: Vec<usize> = (0..self.items.len())
                .filter(|&i| self.items[i].name.to_lowercase().contains(&q) || self.items[i].folder.to_lowercase().contains(&q))
                .collect();
            self.entries = m.iter().map(|&i| Entry::Wall { key: self.items[i].path.clone(), item: i }).collect();
            self.sections = if m.is_empty() { vec![] } else { vec![Section { name: None, items: m.clone() }] };
            self.flat = m;
            return;
        }
        let mut folders: Vec<(String, Vec<usize>)> = Vec::new();
        let mut loose = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            if it.folder.is_empty() {
                loose.push(i);
                continue;
            }
            let top = it.folder.split('/').next().unwrap_or("").to_string();
            match folders.iter_mut().find(|(n, _)| *n == top) {
                Some((_, v)) => v.push(i),
                None => folders.push((top, vec![i])),
            }
        }
        folders.sort_by(|a, b| natural_cmp_ci(&a.0, &b.0));
        self.entries = folders
            .iter()
            .map(|(n, v)| Entry::Folder { key: format!("folder:{n}"), name: n.clone(), items: v.clone() })
            .chain(loose.iter().map(|&i| Entry::Wall { key: self.items[i].path.clone(), item: i }))
            .collect();
        let has_names = !folders.is_empty();
        self.sections = folders.into_iter().map(|(n, v)| Section { name: Some(n), items: v }).collect();
        if !loose.is_empty() {
            self.sections.push(Section { name: if has_names { Some("Unsorted".into()) } else { None }, items: loose });
        }
        self.flat = self.sections.iter().flat_map(|s| s.items.iter().copied()).collect();
    }

    fn locate(&self, path: Option<&str>) -> Option<(usize, Option<usize>)> {
        let path = path?;
        for (r, e) in self.entries.iter().enumerate() {
            match e {
                Entry::Wall { item, .. } if same_path(&self.items[*item].path, path) => return Some((r, None)),
                Entry::Folder { items, .. } => {
                    if let Some(c) = items.iter().position(|&i| same_path(&self.items[i].path, path)) {
                        return Some((r, Some(c)));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn focus_path(&mut self, path: Option<String>, open_folder: bool) {
        let loc = self.locate(path.as_deref());
        self.row = match loc {
            Some((r, _)) => r,
            None => self.row.min(self.entries.len().saturating_sub(1)),
        };
        self.col = if open_folder { loc.and_then(|(_, c)| c) } else { None };
        let g = path.and_then(|p| self.flat.iter().position(|&i| same_path(&self.items[i].path, &p)));
        self.gindex = g.unwrap_or(0);
    }

    fn remember_anchor(&mut self) {
        if let Some(f) = self.focus_item() {
            self.anchor = Some(self.items[f].path.clone());
        }
    }

    pub fn measure(&mut self, vw: f32, vh: f32) {
        let w = (vw * 0.34).max(340.0).min(720.0).min(vh * 0.42 * (16.0 / 9.0)).round();
        self.geo.vw = vw;
        self.geo.vh = vh;
        self.geo.w = w;
        self.geo.h = (w * 9.0 / 16.0).round();
        self.geo.small = (w * 0.6).round();
        self.geo.gap = (18.0f32).max(w * 0.04).round();
        self.geo.stage_c = vh * 0.45;
        let reach = vw / 2.0 - w / 2.0 - self.geo.gap;
        self.geo.radius = ((reach / (self.geo.small + self.geo.gap)).ceil() as i32 + 1).max(2);
    }

    fn slot(&self, d: i32) -> P {
        if d == 0 {
            return P { x: 0.0, y: 0.0, s: 1.0, o: 1.0, b: 1.0 };
        }
        let g = &self.geo;
        let a = d.abs();
        let x = d.signum() as f32 * (g.w / 2.0 + g.gap + g.small / 2.0 + (a - 1) as f32 * (g.small + g.gap));
        P {
            x,
            y: 0.0,
            s: g.small / g.w,
            o: if a >= g.radius { 0.0 } else { 1.0 },
            b: match a {
                1 => 0.58,
                2 => 0.4,
                _ => 0.3,
            },
        }
    }

    fn slot_aside(&self, d: i32) -> P {
        let p = self.slot(d);
        P { x: p.x * 1.35, o: 0.0, b: 0.2, ..p }
    }

    fn slot_col(&self, k: i32) -> P {
        let h0 = self.geo.h * COL_S0;
        if k == 0 {
            return P { x: 0.0, y: 0.0, s: COL_S0, o: 1.0, b: 1.0 };
        }
        let a = k.abs();
        let mut edge = h0 / 2.0;
        let mut sc = COL_S0;
        for i in 1..=a {
            sc = COL_S0 * (1.0 - COL_SHRINK * i as f32);
            edge += h0 * COL_PEEK * 0.82f32.powi(i - 1);
        }
        let offset = edge - (self.geo.h * sc) / 2.0;
        P {
            x: 0.0,
            y: k.signum() as f32 * offset,
            s: sc,
            o: if a > COL_DEPTH { 0.0 } else { 1.0 },
            b: match a {
                1 => 0.5,
                2 => 0.34,
                _ => 0.24,
            },
        }
    }

    // ---------- timers ----------

    fn later(&mut self, ms: f64, step: Step) {
        self.steps.push((self.now + ms, step));
    }
    fn clear_timers(&mut self) {
        self.steps.clear();
    }

    // ---------- glow / caption ----------

    fn update_glow(&mut self) {
        let Some(f) = self.focus_item() else { return };
        let path = self.items[f].path.clone();
        let color = self.thumb_color(&path);
        if let Some(c) = color.and_then(|c| style::glow_for(&c)) {
            let timing = if self.phase == Phase::Hidden { t(0.0, anim::LINEAR) } else { t(900.0, anim::OUT) };
            self.glow.set(c, self.now, timing);
        }
    }

    fn thumb_color(&self, path: &str) -> Option<String> {
        THUMB_COLORS.with(|m| m.borrow().get(&thumbs::key(path)).cloned())
    }

    fn set_title(&mut self, text: String, direction: i32) {
        if text == self.last_title {
            return;
        }
        self.last_title = text.clone();
        let animate = direction != 0 && self.phase == Phase::Open && !self.reduce;
        let now = self.now;
        self.title_id += 1;
        let id = self.title_id;
        let mut line = TitleLine { id, text, y: Tween::new(0.0), o: Tween::new(1.0), leaving: false };
        if animate {
            line.y.jump(if direction > 0 { 0.7 } else { -0.7 });
            line.o.jump(0.0);
            // the double rAF: starts on the next frame
            line.y.set(0.0, now + 16.0, t(520.0, anim::MOVE));
            line.o.set(1.0, now + 16.0, t(380.0, anim::OUT));
        }
        let mut removed = Vec::new();
        for o in self.titles.iter_mut().filter(|l| !l.leaving) {
            if !animate {
                removed.push(o.id);
                continue;
            }
            o.leaving = true;
            o.y.set(if direction > 0 { -0.7 } else { 0.7 }, now, t(520.0, anim::MOVE));
            o.o.set(0.0, now, t(380.0, anim::OUT));
            let oid = o.id;
            self.steps.push((now + 560.0, Step::TitleRemove(oid)));
        }
        self.titles.retain(|l| !removed.contains(&l.id));
        self.titles.push(line);
    }

    fn render_caption(&mut self, direction: i32, swap: bool) {
        let side = self.is_expanded();
        if swap && self.phase == Phase::Open && !self.reduce && self.capmode_side != side {
            self.caption_swap.set(0.0, self.now, t(140.0, anim::OUT));
            self.steps.retain(|(_, s)| *s != Step::CaptionSwap);
            self.later(150.0, Step::CaptionSwap);
            return;
        }
        self.steps.retain(|(_, s)| *s != Step::CaptionSwap);
        self.caption_swap.set(1.0, self.now, t(300.0, anim::OUT));
        self.capmode_side = side;
        self.write_caption(direction);
    }

    fn meta_push(&mut self, text: impl Into<String>, kind: MetaKind) {
        self.meta.push((text.into(), kind));
    }

    fn write_caption(&mut self, direction: i32) {
        self.show_filter_chip = !self.filter.is_empty();
        self.empty_actions = false;
        self.empty = self.items.is_empty();
        self.meta.clear();
        if self.items.is_empty() {
            self.set_title("No wallpapers here yet".into(), 0);
            let folder = self.snapshot.folder.clone().unwrap_or_else(|| "your wallpaper folder".into());
            self.show_filter_chip = false;
            self.meta_push(format!("Add images, or folders of images, to {folder}."), MetaKind::Plain);
            self.empty_actions = true;
            return;
        }
        if self.entries.is_empty() {
            self.set_title(format!("Nothing matches “{}”", self.filter), 0);
            self.meta_push("Backspace to edit, Esc to clear", MetaKind::Plain);
            return;
        }
        if let (false, Some(Entry::Folder { name, items, .. })) = (self.layout_grid(), self.entry().cloned()) {
            if self.col.is_none() {
                self.set_title(name.clone(), direction);
                if let Some(st) = self.status.clone() {
                    self.meta_push(st.text, if st.error { MetaKind::Error } else { MetaKind::Strong });
                } else {
                    if items.iter().any(|&i| self.is_current(i)) {
                        self.meta_push("Has your current wallpaper", MetaKind::Strong);
                    }
                    self.meta_push(plural(items.len(), "wallpaper"), MetaKind::Plain);
                }
                return;
            }
        }
        let Some(item) = self.selected_item() else { return };
        let title = prettify(&self.items[item].name);
        self.set_title(title, direction);
        if let Some(st) = self.status.clone() {
            self.meta_push(st.text, if st.error { MetaKind::Error } else { MetaKind::Strong });
            return;
        }
        let multi = self.multi_screen();
        let lock = self.editing_lock();
        let editing = self.editing_screen().cloned();
        let elsewhere = if multi { self.other_screen_showing(item) } else { None };
        if lock {
            self.meta_push("Editing lock screen", MetaKind::Strong);
        } else if multi {
            if let Some(e) = &editing {
                if !e.here {
                    self.meta_push(format!("Editing screen {}", e.number), MetaKind::Strong);
                }
            }
        }
        if self.is_current(item) {
            let text = if lock {
                "On the lock screen".to_string()
            } else if multi {
                format!("On {}", Self::screen_name(editing.as_ref()))
            } else {
                "On your desktop".to_string()
            };
            self.meta_push(text, MetaKind::Strong);
        } else if let Some(n) = elsewhere {
            self.meta_push(format!("On screen {n}"), MetaKind::Plain);
        }
        let it = self.items[item].clone();
        if it.width > 0 && it.height > 0 {
            self.meta_push(format!("{} × {}", it.width, it.height), MetaKind::Plain);
        }
        if self.is_expanded() {
            let sub: Vec<&str> = it.folder.split('/').skip(1).collect();
            if !sub.is_empty() {
                self.meta_push(sub.join(" / "), MetaKind::Plain);
            }
            let n = self.folder_items().len();
            self.meta_push(format!("{} of {}", self.col.unwrap_or(0) + 1, n), MetaKind::Plain);
        } else if self.layout_grid() {
            if !it.folder.is_empty() && !self.filter.is_empty() {
                self.meta_push(it.folder.split('/').collect::<Vec<_>>().join(" / "), MetaKind::Plain);
            }
            self.meta_push(format!("{} of {}", self.gindex + 1, self.flat.len()), MetaKind::Plain);
        } else if !self.filter.is_empty() && !it.folder.is_empty() {
            self.meta_push(it.folder.split('/').collect::<Vec<_>>().join(" / "), MetaKind::Plain);
        }
    }

    // ---------- row rendering (reconcile) ----------

    fn render_row(&mut self, mode: Option<&str>) {
        let expanded = self.is_expanded();
        let r = self.geo.radius;
        struct Target {
            key: String,
            kind: CardKind,
            p: P,
            z: i32,
            selected: bool,
            delay: f32,
            cls: Cls,
            spawn: P,
            urgent: Option<String>,
        }
        let mut targets: Vec<Target> = Vec::new();
        for d in -r..=r {
            let idx = self.row as i64 + d as i64;
            if idx < 0 {
                continue;
            }
            let Some(e) = self.entries.get(idx as usize).cloned() else { continue };
            let col = self.col.unwrap_or(0) as i32;
            let p = if expanded {
                if d == 0 {
                    self.slot_col(-(col + 1))
                } else {
                    self.slot_aside(d)
                }
            } else {
                self.slot(d)
            };
            let delay = match mode {
                Some("enter") => 70.0 + d.abs() as f32 * 55.0,
                Some("reveal") if d != 0 => 40.0 + (d.abs() - 1) as f32 * REVEAL_STEP,
                _ => 0.0,
            };
            let next = d + if d == 0 { 1 } else { d.signum() };
            let spawn = P { o: 0.0, ..if expanded { self.slot_aside(next) } else { self.slot(next) } };
            let (kind, urgent) = match &e {
                Entry::Folder { key, items, .. } => {
                    (CardKind::Folder { entry: key.clone() }, items.first().map(|&i| self.items[i].path.clone()))
                }
                Entry::Wall { item, .. } => (CardKind::Wall { item: *item }, Some(self.items[*item].path.clone())),
            };
            targets.push(Target {
                key: format!("r:{}", e.key()),
                kind,
                p,
                z: if expanded && d == 0 { 300 - (col + 1) } else { 100 - d.abs() },
                selected: d == 0 && !expanded,
                delay,
                cls: if mode == Some("reveal") { Cls::Slow } else { Cls::Normal },
                spawn,
                urgent: if d == 0 { urgent } else { None },
            });
        }
        if expanded {
            let items = self.folder_items().to_vec();
            let col = self.col.unwrap_or(0) as i64;
            let from = (col - COL_DEPTH as i64 - 1).max(0);
            let to = (items.len() as i64 - 1).min(col + COL_DEPTH as i64 + 1);
            for j in from..=to {
                let k = (j - col) as i32;
                let item = items[j as usize];
                let delay = match mode {
                    Some("drift") => 90.0,
                    Some("enter") => 120.0 + k.abs() as f32 * 60.0,
                    Some("expand") => 200.0 + k.abs() as f32 * 70.0,
                    _ => 0.0,
                };
                let p = self.slot_col(k);
                let spawn = match mode {
                    Some("expand") => P { x: 0.0, y: 0.0, s: COL_S0 * 0.96, o: 0.0, b: 0.7 },
                    Some("drift") => P { y: p.y - (self.geo.h * 0.1).round(), o: 0.0, ..p },
                    _ => P { o: 0.0, ..self.slot_col(k + k.signum()) },
                };
                targets.push(Target {
                    key: format!("c:{}", self.items[item].path),
                    kind: CardKind::Wall { item },
                    p,
                    z: 300 - k.abs(),
                    selected: k == 0,
                    delay,
                    cls: match mode {
                        Some("expand") => Cls::Slow,
                        Some("drift") => Cls::Drift,
                        _ => Cls::Normal,
                    },
                    spawn,
                    urgent: if k == 0 { Some(self.items[item].path.clone()) } else { None },
                });
            }
        }

        // reconcile
        let entering = mode == Some("enter");
        let now = self.now;
        let keys: Vec<&str> = targets.iter().map(|t| t.key.as_str()).collect();
        for c in self.cards.iter_mut() {
            if c.remove_at.is_some() || keys.contains(&c.key.as_str()) {
                continue;
            }
            c.delay = 0.0;
            c.leaving = true;
            c.remove_at = Some(now + 650.0);
            c.key = format!("~{}", c.key); // detached from `nodes`
        }
        let (phase, reduce) = (self.phase, self.reduce);
        for tg in targets {
            if let Some(path) = &tg.urgent {
                URGENT.with(|u| u.borrow_mut().push(path.clone()));
            }
            let pos = self.cards.iter().position(|c| c.key == tg.key && c.remove_at.is_none());
            let idx = match pos {
                Some(i) => i,
                None => {
                    self.order += 1;
                    let mut c = Card::new(tg.key.clone(), tg.kind.clone(), self.order, tg.p, now);
                    c.no_anim = true;
                    c.cls = tg.cls;
                    if entering {
                        // `.enter` pose without transition; released on the next frame
                        c.enter = true;
                    } else {
                        // placed at the spawn point without transition, reflow, then animated to its slot
                        c.tgt = tg.spawn;
                        c.commit(now, phase, reduce);
                        c.no_anim = false;
                    }
                    self.cards.push(c);
                    self.cards.len() - 1
                }
            };
            let c = &mut self.cards[idx];
            c.tgt = tg.p;
            c.cls = tg.cls;
            c.delay = tg.delay;
            c.z = tg.z;
            c.selected = tg.selected;
        }
        self.expanded_attr = expanded;
    }

    fn clear_row(&mut self) {
        self.cards.clear();
    }

    fn on_row_click(&mut self, key: &str, shift: bool) {
        if self.phase != Phase::Open || self.busy {
            return;
        }
        let Some(r) = self.entries.iter().position(|e| e.key() == key) else { return };
        if r != self.row {
            self.select_row(r as i64);
            return;
        }
        match self.entries[r] {
            Entry::Folder { .. } => {
                if self.is_expanded() {
                    self.collapse()
                } else {
                    self.expand()
                }
            }
            Entry::Wall { .. } => self.apply_selected(shift),
        }
    }

    fn on_col_click(&mut self, path: &str, shift: bool) {
        if self.phase != Phase::Open || self.busy {
            return;
        }
        let Some(j) = self.folder_items().iter().position(|&i| self.items[i].path == path) else { return };
        if Some(j) == self.col {
            self.apply_selected(shift);
        } else {
            self.select_col(j as i64);
        }
    }

    fn after_move(&mut self, direction: i32, swap: bool) {
        self.status = None;
        self.remember_anchor();
        self.render_caption(direction, swap);
        self.update_glow();
        self.render_hints();
    }

    fn select_row(&mut self, r: i64) {
        if self.entries.is_empty() || self.busy {
            return;
        }
        let r = r.clamp(0, self.entries.len() as i64 - 1) as usize;
        if r == self.row && self.col.is_none() {
            return;
        }
        let dir = if r > self.row { 1 } else if r < self.row { -1 } else { 1 };
        if self.is_expanded() {
            self.collapse();
            return;
        }
        self.row = r;
        self.col = None;
        self.render_row(None);
        self.after_move(dir, false);
    }

    fn expand(&mut self) {
        if !matches!(self.entry(), Some(Entry::Folder { .. })) || self.is_expanded() || self.busy {
            return;
        }
        self.col = Some(0);
        self.render_row(Some("expand"));
        self.after_move(1, true);
    }

    fn collapse(&mut self) {
        if !self.is_expanded() || self.busy {
            return;
        }
        self.busy = true;
        self.fold_token += 1;
        let token = self.fold_token;
        self.caption_swap.set(0.0, self.now, t(140.0, anim::OUT));
        let lift = (self.geo.h * 0.1).round();
        let col = self.col.unwrap_or(0);
        if col > 0 {
            let items = self.folder_items().to_vec();
            let now = self.now;
            let mut moves = Vec::new();
            for (i, c) in self.cards.iter().enumerate() {
                if c.remove_at.is_some() || !c.key.starts_with("c:") {
                    continue;
                }
                let j = items.iter().position(|&it| format!("c:{}", self.items[it].path) == c.key).unwrap_or(0);
                let p = self.slot_col(j as i32 - col as i32);
                moves.push((i, P { y: p.y + lift, o: 0.0, ..p }));
            }
            for (i, p) in moves {
                let c = &mut self.cards[i];
                c.delay = 0.0;
                c.cls = Cls::Drift;
                c.leaving = true;
                c.tgt = p;
                c.remove_at = Some(now + 500.0);
                c.key = format!("~{}", c.key);
            }
            self.col = Some(0);
            let key = match self.entry() {
                Some(e) => format!("r:{}", e.key()),
                None => String::new(),
            };
            let p = self.slot_col(-1);
            let phase = self.phase;
            let reduce = self.reduce;
            if let Some(c) = self.cards.iter_mut().find(|c| c.key == key) {
                c.no_anim = true;
                c.tgt = P { y: p.y - lift, o: 0.0, ..p };
                c.commit(now, phase, reduce);
                c.no_anim = false;
            }
            self.render_row(Some("drift"));
            self.later(620.0, Step::CollapseFold(token));
        } else {
            self.collapse_fold(token);
        }
    }

    fn collapse_fold(&mut self, token: u64) {
        if token != self.fold_token || self.phase != Phase::Open {
            return;
        }
        let Some(Entry::Folder { key, items, .. }) = self.entry().cloned() else { return };
        let top_key = format!("c:{}", self.items[items[0]].path);
        let folder_key = format!("r:{key}");
        let h = self.geo.h;
        let s0 = self.slot(0);
        for c in self.cards.iter_mut() {
            if c.remove_at.is_some() {
                continue;
            }
            if c.key.starts_with("c:") && c.key != top_key {
                c.cls = Cls::Folding;
                c.delay = 0.0;
                c.z = (c.z - 240).max(1);
                c.tgt = P { x: 0.0, y: h * 0.04, s: COL_S0 * 0.95, o: 0.0, b: 0.5 };
            } else if c.key == folder_key {
                c.cls = Cls::FoldingKeep;
                c.delay = 0.0;
                c.z = 90;
                c.tgt = s0;
            } else if c.key == top_key {
                c.cls = Cls::FoldingKeep;
                c.delay = 0.0;
                c.z = 100;
                c.tgt = P { x: 0.0, y: 0.0, s: 1.0, o: 1.0, b: 1.0 };
            }
        }
        self.later(540.0, Step::CollapseReveal(token));
    }

    fn collapse_reveal(&mut self, token: u64) {
        if token != self.fold_token || self.phase != Phase::Open {
            return;
        }
        let Some(Entry::Folder { key, items, .. }) = self.entry().cloned() else { return };
        let top_key = format!("c:{}", self.items[items[0]].path);
        let folder_key = format!("r:{key}");
        let now = self.now;
        for c in self.cards.iter_mut() {
            if c.key == top_key {
                c.leaving = true;
            }
            if c.key == folder_key {
                c.cls = Cls::Normal;
            }
        }
        self.col = None;
        self.busy = false;
        self.render_row(Some("reveal"));
        let _ = now;
        self.after_move(-1, true);
    }

    fn select_col(&mut self, j: i64) {
        if !self.is_expanded() || self.busy {
            return;
        }
        let n = self.folder_items().len() as i64;
        let j = j.clamp(0, n - 1) as usize;
        let cur = self.col.unwrap_or(0);
        if j == cur {
            return;
        }
        let dir = if j > cur { 1 } else { -1 };
        self.col = Some(j);
        self.render_row(None);
        self.after_move(dir, false);
    }

    // ---------- grid ----------

    fn render_grid(&mut self, entering: bool) {
        self.tiles.clear();
        self.headings.clear();
        let mut delay_index = 0usize;
        let now = self.now;
        let sections = self.sections.clone();
        for sec in &sections {
            if let Some(name) = &sec.name {
                let mut h = Heading { name: name.clone(), count: sec.items.len(), enter: Tween::new(1.0), rect: Rect::NOTHING };
                if entering {
                    h.enter.jump(0.0);
                    h.enter.set(1.0, now + 16.0, t(400.0, anim::OUT).delayed(60.0 + delay_index.min(20) as f32 * 24.0));
                }
                self.headings.push(h);
            }
            for &item in &sec.items {
                let idx = self.tiles.len();
                let mut tile = Tile {
                    item,
                    enter: Tween::new(1.0),
                    enter_delay: 0.0,
                    hover: Tween::new(0.0),
                    sel: Tween::new(if idx == self.gindex { 1.0 } else { 0.0 }),
                    selk: Tween::new(if idx == self.gindex { 1.0 } else { 0.0 }),
                    img_at: None,
                    rect: Rect::NOTHING,
                    created: now,
                };
                if entering {
                    tile.enter.jump(0.0);
                    tile.enter_delay = 60.0 + delay_index.min(20) as f32 * 24.0;
                    tile.enter.set(1.0, now + 32.0, t(420.0, anim::MOVE).delayed(tile.enter_delay));
                }
                delay_index += 1;
                self.tiles.push(tile);
            }
        }
        self.grid_center_pending = true;
    }

    fn select_grid(&mut self, i: i64) {
        if self.flat.is_empty() {
            return;
        }
        let i = i.clamp(0, self.flat.len() as i64 - 1) as usize;
        if i == self.gindex {
            return;
        }
        let dir = if i > self.gindex { 1 } else { -1 };
        self.gindex = i;
        let path = self.items[self.flat[i]].path.clone();
        URGENT.with(|u| u.borrow_mut().push(path));
        self.grid_scroll_nearest = true;
        self.after_move(dir, false);
    }

    fn grid_vertical(&mut self, dir: i32) {
        let Some(cur) = self.tiles.get(self.gindex) else { return };
        let top = cur.rect.min.y;
        let cx = cur.rect.center().x;
        let mut row_top: Option<f32> = None;
        for t in &self.tiles {
            let tt = t.rect.min.y;
            if if dir > 0 { tt > top + 4.0 } else { tt < top - 4.0 } {
                if row_top.map_or(true, |r| if dir > 0 { tt < r } else { tt > r }) {
                    row_top = Some(tt);
                }
            }
        }
        let Some(row_top) = row_top else { return };
        let mut best = None;
        let mut best_dist = f32::INFINITY;
        for (i, t) in self.tiles.iter().enumerate() {
            if (t.rect.min.y - row_top).abs() > 4.0 {
                continue;
            }
            let dist = (t.rect.center().x - cx).abs();
            if dist < best_dist {
                best_dist = dist;
                best = Some(i);
            }
        }
        if let Some(b) = best {
            self.select_grid(b as i64);
        }
    }

    fn render_picker(&mut self, mode: Option<&str>) {
        if !self.layout_grid() {
            self.clear_row();
            self.render_row(if mode == Some("enter") { Some("enter") } else { None });
            if mode != Some("enter") {
                self.fade_in_row();
            }
        } else {
            self.expanded_attr = false;
            self.render_grid(mode == Some("enter"));
        }
        self.render_caption(0, false);
        self.update_glow();
        self.render_hints();
        self.render_minimap();
    }

    fn fade_in_row(&mut self) {
        // `.no-anim` + opacity 0: every card snaps to its slot invisibly, then fades in.
        let (now, phase, reduce) = (self.now, self.phase, self.reduce);
        for c in self.cards.iter_mut() {
            c.no_anim = true;
            c.commit(now, phase, reduce);
            c.no_anim = false;
            c.op.jump(0.0);
        }
    }

    fn apply_filter(&mut self) {
        self.remember_anchor();
        self.build_model();
        if !self.filter.is_empty() {
            let i = self.entries.iter().position(|e| same_opt(Some(e.key()), self.anchor.as_deref()));
            self.row = i.unwrap_or(0);
            self.col = None;
            self.gindex = i.unwrap_or(0);
        } else {
            self.focus_path(self.anchor.clone(), false);
        }
        self.status = None;
        self.render_picker(None);
    }

    // ---------- hints ----------

    fn hint(keys: &[&str], label: &str, optional: bool) -> Hint {
        Hint { keys: keys.iter().map(|s| s.to_string()).collect(), label: label.to_string(), optional }
    }

    fn render_hints(&mut self) {
        let h = Self::hint;
        if self.view == View::Welcome {
            self.hints = vec![h(&["Enter"], if self.items.is_empty() { "Done" } else { "Show wallpapers" }, false), h(&["Esc"], "Close", false)];
            return;
        }
        if self.view == View::Settings {
            self.hints = vec![
                h(&["Ctrl", "Tab"], "Next tab", false),
                h(&["Esc"], if self.settings_from_picker == Some(true) { "Back" } else { "Close" }, false),
            ];
            return;
        }
        if self.items.is_empty() {
            self.hints = vec![h(&["Ctrl", ","], "Settings", false), h(&["Esc"], "Close", false)];
            return;
        }
        let esc = h(
            &["Esc"],
            if !self.filter.is_empty() {
                "Clear filter"
            } else if self.is_expanded() {
                "Close folder"
            } else {
                "Close"
            },
            false,
        );
        let multi = self.multi_screen();
        let one = self.settings.enter_this_screen;
        let editing = self.editing_screen().cloned();
        let lock = self.editing_lock();
        let set_hint = if lock {
            h(&["Enter"], "Set lock screen", false)
        } else if multi {
            h(&["Enter"], &if one { format!("Set on {}", Self::screen_name(editing.as_ref())) } else { "Set on all screens".to_string() }, false)
        } else {
            h(&["Enter"], "Set wallpaper", false)
        };
        let flip = if multi && !lock {
            let name = Self::screen_name(editing.as_ref());
            let cap = name[..1].to_uppercase() + &name[1..];
            Some(h(&["Shift", "Enter"], &if one { "All screens".to_string() } else { format!("{cap} only") }, false))
        } else {
            None
        };
        let screen_hint = if self.map_tiles().len() > 1 {
            let mut keys = vec!["Ctrl".to_string()];
            keys.extend(self.map_arrows());
            Some(Hint { keys, label: if multi { "Other screen".into() } else { "Lock screen".into() }, optional: true })
        } else {
            None
        };
        let settings_hint = h(&["Ctrl", ","], "Settings", false);
        let ui_hint = h(&["Ctrl", "H"], self.ui_hint_label(), false);
        let list: Vec<Option<Hint>> = if self.layout_grid() {
            vec![
                Some(h(&["←", "↑", "↓", "→"], "Browse", false)),
                Some(set_hint),
                flip,
                screen_hint,
                Some(h(&["Tab"], "Row view", true)),
                Some(h(&["A–Z"], "Filter", true)),
                Some(settings_hint),
                Some(ui_hint),
                Some(esc),
            ]
        } else if self.is_expanded() {
            vec![
                Some(h(&["↑", "↓"], "Browse folder", false)),
                Some(set_hint),
                flip,
                screen_hint,
                Some(h(&["Tab"], "Grid view", true)),
                Some(settings_hint),
                Some(ui_hint),
                Some(esc),
            ]
        } else {
            let folder = matches!(self.entry(), Some(Entry::Folder { .. }));
            vec![
                Some(h(&["←", "→"], "Browse", false)),
                Some(if folder { h(&["↓"], "Open folder", false) } else { set_hint }),
                if folder { None } else { flip },
                screen_hint,
                Some(h(&["Tab"], "Grid view", true)),
                Some(h(&["A–Z"], "Filter", true)),
                Some(settings_hint),
                Some(ui_hint),
                Some(esc),
            ]
        };
        self.hints = list.into_iter().flatten().collect();
    }

    fn ui_hint_label(&self) -> &'static str {
        let s = &self.settings;
        if s.ui_hidden {
            "Show UI"
        } else if s.hide_hints && !s.hide_title && !s.hide_details {
            "Hide hints"
        } else {
            "Hide UI"
        }
    }

    // ---------- snapshot / library ----------

    fn apply_snapshot(&mut self, snap: Snapshot) {
        self.settings = snap.settings.clone();
        self.snapshot = snap;
        self.panel.sync(&self.settings, self.multi_screen(), self.now);
    }

    fn load_library(&mut self, host: &mut dyn Host) {
        let snap = host.core().snapshot();
        let items = host.core().list_wallpapers();
        let current = snap.current.clone();
        self.apply_snapshot(snap);
        self.items = items;
        self.build_model();
        let start = if self.locate(current.as_deref()).is_some() { current } else { self.anchor.clone() };
        self.focus_path(start, false);
        if let Some(f) = self.focus_item() {
            URGENT.with(|u| u.borrow_mut().push(self.items[f].path.clone()));
        }
    }

    // ---------- open / close ----------

    pub fn open(&mut self, view: View, host: &mut dyn Host, now: f64) {
        self.now = now;
        if matches!(self.phase, Phase::Open | Phase::Opening) {
            if view == View::Settings && self.view == View::Picker {
                self.show_settings(false);
            }
            return;
        }
        if self.phase == Phase::Applying {
            return;
        }
        self.clear_timers();
        self.busy = false;
        self.phase = Phase::Opening;
        self.filter.clear();
        self.status = None;
        self.settings_from_picker = None;
        self.lock_image = None;
        self.last_title.clear();
        self.titles.clear();
        self.apply = None;
        self.full_image = None;
        self.edge_flash = None;
        self.opened_at = now;
        self.first_frame = true;
        // everything starts from its hidden state with no transitions
        self.backdrop.jump(0.0);
        self.caption_swap.jump(1.0);
        self.caption_close.jump(1.0);
        self.hints_enter.jump(0.0);
        self.caption_enter.jump(0.0);
        self.picker_o.jump(1.0);
        self.settings_o.jump(0.0);
        self.welcome_o.jump(0.0);
        self.cards.clear();
        self.tiles.clear();
        self.headings.clear();

        self.load_library(host);
        let mut view = view;
        if self.snapshot.first_run {
            view = View::Welcome;
        }
        self.view = view;
        self.token += 1;
        let tok = self.token;
        self.open_wait_until = now + 260.0;
        self.open_wait_path = if view != View::Welcome { self.focus_item().map(|f| self.items[f].path.clone()) } else { None };
        self.later(0.0, Step::OpenShow(tok));
    }

    fn open_show(&mut self, tok: u64, thumbs: &Thumbs) {
        crate::log(format!("open_show phase={:?} items={} entries={} geo={:?}", self.phase, self.items.len(), self.entries.len(), self.geo));
        if tok != self.token || self.phase != Phase::Opening {
            return;
        }
        if let Some(p) = &self.open_wait_path {
            if !thumbs.has(p) && self.now < self.open_wait_until {
                self.later(16.0, Step::OpenShow(tok));
                return;
            }
        }
        let (vw, vh) = (self.geo.vw, self.geo.vh);
        self.measure(vw, vh);
        let t280 = t(280.0, anim::OUT);
        self.backdrop.set(1.0, self.now, t280);
        if self.view == View::Welcome {
            self.show_welcome();
            self.later(16.0, Step::OpenDone(tok));
        } else {
            self.enter_picker();
            self.later(34.0, Step::OpenDone(tok));
        }
    }

    fn open_done(&mut self, tok: u64) {
        crate::log(format!("open_done tok={tok}/{} phase={:?} cards={}", self.token, self.phase, self.cards.len()));
        if tok != self.token || self.phase != Phase::Opening {
            return;
        }
        self.phase = Phase::Open;
        self.panel.phase_open(self.view, self.now);
        if self.view == View::Settings {
            self.show_settings(false);
        }
    }

    fn enter_picker(&mut self) {
        self.enter_token += 1;
        let tok = self.enter_token;
        self.caption_enter.jump(0.0);
        self.hints_enter.jump(0.0);
        self.render_picker(Some("enter"));
        self.later(16.0, Step::EnterRelease(tok));
    }

    fn enter_release(&mut self, tok: u64) {
        crate::log(format!("enter_release cards={}", self.cards.len()));
        if tok != self.enter_token || matches!(self.phase, Phase::Hidden | Phase::Closing) {
            return;
        }
        for c in self.cards.iter_mut() {
            c.no_anim = false;
            c.enter = false;
        }
        self.later(140.0, Step::EnterCaption(tok));
        self.later(320.0, Step::EnterHints(tok));
        self.later(520.0, Step::EnterDelays(tok));
    }

    pub fn close(&mut self, host: &mut dyn Host) {
        if matches!(self.phase, Phase::Hidden | Phase::Closing) {
            return;
        }
        let _ = host;
        self.panel.stop_recording(&self.settings);
        self.clear_timers();
        self.busy = false;
        self.moving = false;
        self.remember_anchor();
        self.phase = Phase::Closing;
        let now = self.now;
        self.backdrop.set(0.0, now, t(220.0, anim::IN));
        self.caption_close.set(0.0, now, t(180.0, anim::OUT));
        self.hints_close(now);
        self.settings_o.set(0.0, now, t(180.0, anim::OUT));
        self.welcome_o.set(0.0, now, t(180.0, anim::OUT));
        if let Some(a) = &mut self.apply {
            a.fade.set(0.0, now, t(360.0, anim::OUT));
        }
        for tile in self.tiles.iter_mut() {
            tile.enter.set(-1.0, now, t(180.0, anim::IN)); // -1 = closing pose
        }
        self.later(240.0, Step::CloseDone);
    }

    fn hints_close(&mut self, now: f64) {
        self.hints_enter.set(0.0, now, t(160.0, anim::OUT));
    }

    fn close_done(&mut self, host: &mut dyn Host) {
        self.phase = Phase::Hidden;
        self.view = View::Picker;
        self.clear_row();
        self.tiles.clear();
        self.headings.clear();
        self.apply = None;
        self.titles.clear();
        host.hide();
    }

    // ---------- welcome ----------

    fn show_welcome(&mut self) {
        self.clear_row();
        self.welcome_o.set(1.0, self.now, t(300.0, anim::OUT));
        self.picker_o.set(0.0, self.now, t(260.0, anim::OUT));
        self.panel.welcome_items = self.items.len();
        self.panel.focus = Some(panel::Ctl::WelcomeDone);
        self.panel.focus_visible = false;
        self.render_hints();
    }

    fn finish_welcome(&mut self, host: &mut dyn Host) {
        let on = self.panel.welcome_autostart;
        let snap = host.core().finish_welcome(on);
        self.apply_snapshot(snap);
    }

    fn leave_welcome(&mut self, host: &mut dyn Host) {
        if self.view != View::Welcome {
            return;
        }
        self.finish_welcome(host);
        if self.items.is_empty() {
            self.close(host);
            return;
        }
        self.view = View::Picker;
        self.welcome_o.set(0.0, self.now, t(300.0, anim::OUT));
        self.picker_o.set(1.0, self.now, t(260.0, anim::OUT));
        self.panel.focus = None;
        self.enter_picker();
    }

    fn dismiss_welcome(&mut self, host: &mut dyn Host) {
        if self.view != View::Welcome {
            return;
        }
        self.finish_welcome(host);
        self.close(host);
    }

    // ---------- screens ----------

    pub fn map_tiles(&self) -> Vec<(Option<usize>, [f32; 4])> {
        let list = self.screens();
        let mut tiles: Vec<(Option<usize>, [f32; 4])> =
            list.iter().enumerate().map(|(i, s)| (Some(i), s.rect.map(|v| v as f32))).collect();
        if !self.lock_own() || list.is_empty() {
            return tiles;
        }
        let primary = list.iter().find(|s| s.rect[0] == 0 && s.rect[1] == 0).unwrap_or(&list[0]);
        let pw = (primary.rect[2] - primary.rect[0]) as f32 * LOCK_SCALE;
        let ph = (primary.rect[3] - primary.rect[1]) as f32 * LOCK_SCALE;
        let [left, top, right, bottom] = bounds(&tiles.iter().map(|t| t.1).collect::<Vec<_>>());
        let gap = (right - left).max(bottom - top) * 0.06;
        let rect = if self.settings.lock_spot == "above" {
            [right - pw, top - gap - ph, right, top - gap]
        } else {
            [left - gap - pw, bottom - ph, left - gap, bottom]
        };
        tiles.push((None, rect));
        tiles
    }

    fn editing_tile(&self, tiles: &[(Option<usize>, [f32; 4])]) -> Option<usize> {
        if self.editing_lock() {
            return tiles.iter().position(|t| t.0.is_none());
        }
        let s = self.editing_screen()?.number - 1;
        tiles.iter().position(|t| t.0 == Some(s))
    }

    fn map_arrows(&self) -> Vec<String> {
        let centres: Vec<(f32, f32)> = self.map_tiles().iter().map(|t| middle(t.1)).collect();
        let (mut across, mut stacked) = (false, false);
        for (i, a) in centres.iter().enumerate() {
            for b in &centres[i + 1..] {
                if (a.0 - b.0).abs() >= (a.1 - b.1).abs() {
                    across = true;
                } else {
                    stacked = true;
                }
            }
        }
        let mut v = Vec::new();
        if across {
            v.push("←".to_string());
        }
        if stacked {
            v.push("↑".into());
            v.push("↓".into());
        }
        if across {
            v.push("→".into());
        }
        v
    }

    fn neighbor(&self, dir: &str) -> Option<Option<usize>> {
        let tiles = self.map_tiles();
        let from = self.editing_tile(&tiles)?;
        let origin = middle(tiles[from].1);
        let mut best = None;
        let mut best_score = f32::INFINITY;
        for (i, t) in tiles.iter().enumerate() {
            if i == from {
                continue;
            }
            let p = middle(t.1);
            let (dx, dy) = (p.0 - origin.0, p.1 - origin.1);
            let along = match dir {
                "left" => -dx,
                "right" => dx,
                "up" => -dy,
                _ => dy,
            };
            let across = if dir == "left" || dir == "right" { dy.abs() } else { dx.abs() };
            if along <= 0.0 || across > along * 2.0 {
                continue;
            }
            let score = along + across * 2.0;
            if score < best_score {
                best_score = score;
                best = Some(t.0);
            }
        }
        best
    }

    fn select_tile(&mut self, tile: Option<Option<usize>>, host: &mut dyn Host) {
        match tile {
            Some(None) => self.select_lock(host),
            Some(Some(i)) => self.select_screen(i, host),
            None => {}
        }
    }

    pub fn render_minimap(&mut self) {
        if self.lock_own() && self.lock_image.is_none() && self.phase != Phase::Hidden && !self.lock_loading {
            self.lock_loading = true;
            LOCK_REQ.with(|r| *r.borrow_mut() = true);
        }
    }

    fn select_lock(&mut self, host: &mut dyn Host) {
        if !self.lock_own() || self.editing_lock() || self.moving {
            return;
        }
        if self.phase != Phase::Open || self.view != View::Picker || self.busy {
            return;
        }
        let snap = host.core().select_lock();
        self.apply_snapshot(snap);
        self.refresh_screen_ui();
    }

    fn refresh_screen_ui(&mut self) {
        self.status = None;
        self.render_minimap();
        if !self.layout_grid() {
            self.render_row(None);
        }
        self.render_caption(0, false);
        self.update_glow();
        self.render_hints();
    }

    fn select_screen(&mut self, index: usize, host: &mut dyn Host) {
        let Some(target) = self.screens().get(index).cloned() else { return };
        if (target.editing && !self.editing_lock()) || self.moving {
            return;
        }
        if self.phase != Phase::Open || self.view != View::Picker || self.busy {
            return;
        }
        if !self.settings.follow_screen || target.here {
            let snap = host.select_screen(index);
            self.apply_snapshot(snap);
            self.refresh_screen_ui();
            self.flash_edge();
            return;
        }
        self.moving = true;
        self.remember_anchor();
        let now = self.now;
        self.picker_o.set(0.0, now, t(170.0, anim::IN));
        self.hints_enter.set(0.0, now, t(170.0, anim::IN));
        self.backdrop.set(0.0, now, t(170.0, anim::OUT));
        self.token += 1;
        let tok = self.token;
        self.later(180.0, Step::MoveSelect(index, tok));
    }

    fn move_select(&mut self, index: usize, tok: u64, host: &mut dyn Host) {
        if tok != self.token {
            return;
        }
        let snap = host.select_screen(index);
        self.apply_snapshot(snap);
        self.later(76.0, Step::MoveEnter(tok));
    }

    fn move_enter(&mut self, tok: u64) {
        if tok != self.token {
            return;
        }
        if self.phase == Phase::Open {
            let (vw, vh) = (self.geo.vw, self.geo.vh);
            self.measure(vw, vh);
            self.status = None;
            self.last_title.clear();
            let now = self.now;
            self.picker_o.jump(1.0);
            self.backdrop.set(1.0, now, t(280.0, anim::OUT));
            self.flash_edge();
            self.enter_picker();
        }
        self.moving = false;
    }

    pub fn flash_edge(&mut self) {
        if !self.settings.edit_glow || !self.multi_screen() || self.editing_lock() {
            return;
        }
        if !self.editing_screen().is_some_and(|s| s.here) {
            return;
        }
        self.edge_flash = Some(self.now);
    }

    // ---------- apply ----------

    fn applies_everywhere(&self, shift: bool) -> bool {
        if !self.multi_screen() {
            return true;
        }
        if self.settings.enter_this_screen {
            shift
        } else {
            !shift
        }
    }

    fn apply_selected(&mut self, flip: bool) {
        if self.phase != Phase::Open || self.view != View::Picker || self.busy || self.moving {
            return;
        }
        let Some(item) = self.selected_item() else {
            self.expand();
            return;
        };
        APPLY_REQ.with(|a| *a.borrow_mut() = Some((item, flip)));
    }

    fn apply_start(&mut self, item: usize, flip: bool, host: &mut dyn Host) {
        let path = self.items[item].path.clone();
        let from = self.item_rect(item);
        self.phase = Phase::Applying;
        self.status = None;
        let everywhere = self.applies_everywhere(flip);
        let to_lock = self.editing_lock();
        host.spawn_apply(path.clone(), everywhere);
        let now = self.now;
        self.hints_enter.set(0.0, now, t(160.0, anim::OUT));
        let mut a = Apply {
            path: path.clone(),
            from: Rect::NOTHING,
            from_radius: 10.0,
            start: now,
            reverse_at: None,
            result: None,
            anim_done: true,
            to_lock,
            fade: Tween::new(1.0),
        };
        if let (Some((r, radius)), false) = (from, self.reduce) {
            a.from = r;
            a.from_radius = radius;
            a.anim_done = false;
            host.spawn_full_image(path);
            self.caption_swap.set(0.0, now, t(140.0, anim::OUT));
            self.later(160.0, Step::CaptionSwap);
            self.later(620.0, Step::ApplyAnimDone);
        }
        self.apply = Some(a);
    }

    fn apply_progress(&mut self, host: &mut dyn Host) {
        let Some(a) = &self.apply else { return };
        if !a.anim_done || a.result.is_none() || self.phase != Phase::Applying {
            return;
        }
        let a = self.apply.as_mut().unwrap();
        let result = a.result.take().unwrap();
        let path = a.path.clone();
        let to_lock = a.to_lock;
        match result {
            Err(e) => {
                if a.reverse_at.is_none() && a.from.is_positive() {
                    a.reverse_at = Some(self.now);
                    a.result = Some(Err(e));
                    a.anim_done = false;
                    self.later(620.0, Step::ApplyAnimDone);
                    return;
                }
                self.apply = None;
                self.phase = Phase::Open;
                self.hints_enter.set(1.0, self.now, t(300.0, anim::OUT));
                self.status = Some(Status {
                    error: true,
                    text: format!("{} {e}", if to_lock { "Couldn't set the lock screen." } else { "Couldn't set wallpaper." }),
                });
                self.render_caption(0, false);
                for c in self.cards.iter_mut() {
                    c.op.jump(c.op.get(self.now));
                }
            }
            Ok(res) => {
                host.core().applied(&path, &res);
                if res.scope == "lock" || res.lock_screen == Some(true) {
                    self.settings.lock_wallpaper = Some(path.clone().into());
                    self.lock_image = Some(Some(path.clone()));
                }
                if res.scope == "lock" {
                    self.render_minimap();
                    self.anchor = Some(path);
                    self.status = Some(Status { error: false, text: "Set on the lock screen".into() });
                    self.write_caption(0);
                    self.later(380.0, Step::ApplyClose);
                    return;
                }
                self.snapshot.current = Some(path.clone());
                let all = res.scope == "all";
                for s in self.snapshot.screens.iter_mut() {
                    if all || s.editing {
                        s.current = Some(path.clone());
                    }
                }
                self.render_minimap();
                self.anchor = Some(path);
                let multi = self.multi_screen();
                let mut text = if !multi {
                    "Wallpaper set".to_string()
                } else if all {
                    "Set on all screens".to_string()
                } else {
                    format!("Set on {}", Self::screen_name(self.editing_screen()))
                };
                if res.lock_screen == Some(true) {
                    text = if multi { format!("{text} and the lock screen") } else { "Wallpaper and lock screen set".into() };
                }
                let failed = res.lock_screen == Some(false);
                if failed {
                    text += &format!(
                        " · Lock screen didn't change{}",
                        res.lock_error.as_ref().map(|e| format!(": {e}")).unwrap_or_default()
                    );
                }
                self.status = Some(Status { error: failed, text });
                self.write_caption(0);
                self.later(if failed { 2600.0 } else { 380.0 }, Step::ApplyClose);
            }
        }
    }

    /// Where a selected item is drawn right now (for the apply zoom).
    fn item_rect(&self, item: usize) -> Option<(Rect, f32)> {
        if self.layout_grid() {
            let tile = self.tiles.iter().position(|t| t.item == item)?;
            let r = self.tiles[tile].rect;
            if !r.is_positive() {
                return None;
            }
            let r = r.translate(egui::vec2(0.0, -self.grid_scroll.get(self.now)));
            let r = Rect::from_center_size(r.center(), r.size() * 1.035);
            return Some((r.translate(self.grid_origin.to_vec2()), 10.0));
        }
        let key = if self.is_expanded() { format!("c:{}", self.items[item].path) } else { format!("r:{}", self.items[item].path) };
        let c = self.cards.iter().find(|c| c.key == key)?;
        let r = picker::card_rect(self, c, self.now);
        Some((r, 14.0 * (r.width() / self.geo.w)))
    }

    // ---------- settings ----------

    fn show_settings(&mut self, from_picker: bool) {
        self.panel.stop_recording(&self.settings);
        self.view = View::Settings;
        self.settings_from_picker = Some(from_picker);
        let now = self.now;
        self.settings_o.set(1.0, now, t(260.0, anim::OUT));
        self.picker_o.set(0.0, now, t(260.0, anim::OUT));
        self.welcome_o.set(0.0, now, t(300.0, anim::OUT));
        self.panel.show(from_picker, self.snapshot.shortcut_error.clone(), &self.settings, self.multi_screen(), now);
        self.render_hints();
    }

    fn leave_settings(&mut self, host: &mut dyn Host) {
        self.panel.stop_recording(&self.settings);
        self.panel.close_selects(self.now);
        if self.settings_from_picker == Some(true) {
            self.view = View::Picker;
            let now = self.now;
            self.settings_o.set(0.0, now, t(260.0, anim::OUT));
            self.picker_o.set(1.0, now, t(260.0, anim::OUT));
            self.panel.focus = None;
            self.render_picker(None);
        } else {
            self.close(host);
        }
    }

    fn save_settings(&mut self, host: &mut dyn Host, patch: impl FnOnce(&mut Settings), folder_changed: bool) {
        let mut next = self.settings.clone();
        patch(&mut next);
        match host.save_settings(next) {
            Ok(snap) => {
                self.panel.error.clear();
                self.apply_snapshot(snap);
            }
            Err(e) => {
                self.panel.error = e;
                let snap = host.core().snapshot();
                self.apply_snapshot(snap);
            }
        }
        if folder_changed {
            self.filter.clear();
            self.anchor = None;
            self.load_library(host);
            if self.view == View::Welcome {
                self.show_welcome();
            } else {
                self.render_picker(None);
            }
        }
    }

    fn toggle_ui(&mut self, host: &mut dyn Host) {
        let v = !self.settings.ui_hidden;
        self.settings.ui_hidden = v;
        self.render_hints();
        self.save_settings(host, |s| s.ui_hidden = v, false);
    }

    fn toggle_layout(&mut self, host: &mut dyn Host) {
        self.remember_anchor();
        let next = if self.layout_grid() { "slider" } else { "grid" };
        self.settings.layout = next.into();
        self.focus_path(self.anchor.clone(), true);
        if next == "slider" && !self.filter.is_empty() {
            self.col = None;
        }
        let (vw, vh) = (self.geo.vw, self.geo.vh);
        self.measure(vw, vh);
        self.render_picker(None);
        self.save_settings(host, |s| s.layout = next.into(), false);
    }

    fn open_folder(&mut self, host: &mut dyn Host) {
        if self.view == View::Welcome {
            self.finish_welcome(host);
        }
        match host.core().open_folder() {
            Ok(()) => self.close(host),
            Err(e) => {
                self.status = Some(Status { error: true, text: e });
                if self.view == View::Picker {
                    self.render_caption(0, false);
                }
            }
        }
    }

    // ---------- messages from workers ----------

    pub fn on_msg(&mut self, msg: Msg, host: &mut dyn Host) {
        match msg {
            Msg::Applied(r) => {
                if let Some(a) = &mut self.apply {
                    a.result = Some(r);
                }
                self.apply_progress(host);
            }
            Msg::LockImage(p) => {
                self.lock_loading = false;
                if self.lock_image.is_none() {
                    self.lock_image = Some(p);
                }
            }
            Msg::Picked(p) => {
                if let Some(p) = p {
                    self.save_settings(host, |s| s.folder = Some(p.into()), true);
                }
            }
            Msg::FullImage(path, levels) => {
                if self.apply.as_ref().is_some_and(|a| a.path == path) {
                    self.pending_full = Some((path, levels));
                }
            }
        }
    }

    // ---------- frame ----------

    /// Runs timers, input and drawing for one frame. Returns true while anything animates.
    pub fn frame(&mut self, ui: &mut egui::Ui, fx: &Arc<Fx>, thumbs: &mut Thumbs, host: &mut dyn Host, now: f64) -> bool {
        self.now = now;
        let screen = ui.ctx().content_rect();
        if (screen.width() - self.geo.vw).abs() > 0.5 || (screen.height() - self.geo.vh).abs() > 0.5 {
            self.measure(screen.width(), screen.height());
            if self.phase != Phase::Hidden && self.view == View::Picker && !self.layout_grid() && !self.first_frame {
                self.render_row(None);
            }
        }

        // thumbnails that arrived
        for path in thumbs.pump(fx, now) {
            THUMB_COLORS.with(|m| {
                if let Some(c) = thumbs.colors.get(&thumbs::key(&path)) {
                    m.borrow_mut().insert(thumbs::key(&path), c.clone());
                }
            });
            if self.focus_item().is_some_and(|f| same_path(&self.items[f].path, &path)) {
                self.update_glow();
            }
        }
        if let Some((path, levels)) = self.pending_full.take() {
            if let Some(tex) = fx.upload(&levels) {
                if let Some((_, old)) = self.full_image.replace((path, tex)) {
                    old.destroy();
                }
            }
        }

        // timers
        loop {
            let due: Vec<Step> = {
                let mut due = Vec::new();
                let mut i = 0;
                while i < self.steps.len() {
                    if self.steps[i].0 <= now {
                        due.push(self.steps.remove(i).1);
                    } else {
                        i += 1;
                    }
                }
                due
            };
            if due.is_empty() {
                break;
            }
            for s in due {
                self.run_step(s, thumbs, host);
            }
        }

        // input
        self.handle_input(ui, host);
        if let Some((item, flip)) = APPLY_REQ.with(|a| a.borrow_mut().take()) {
            self.apply_start(item, flip, host);
        }
        if LOCK_REQ.with(|r| std::mem::take(&mut *r.borrow_mut())) {
            host.spawn_lock_image();
        }

        // style recalc for cards
        let (phase, reduce) = (self.phase, self.reduce);
        for c in self.cards.iter_mut() {
            c.commit(now, phase, reduce);
        }
        self.cards.retain(|c| c.remove_at.map_or(true, |t| t > now));

        // draw
        let painter = ui.painter().clone();
        self.hits.clear();
        picker::draw(self, &painter, fx, thumbs, screen, now);
        panel::draw(self, &painter, fx, screen, now);
        picker::draw_top(self, &painter, fx, thumbs, screen, now);

        for p in URGENT.with(|u| std::mem::take(&mut *u.borrow_mut())) {
            thumbs.want(&p, true);
        }
        thumbs.end_frame();
        self.first_frame = false;

        // cursor
        let pointer_cursor = self.hovered.as_ref().is_some_and(|h| {
            matches!(h, Hit::MapScreen(_) | Hit::MapLock | Hit::Button(_) | Hit::Control(_))
        });
        if pointer_cursor {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let animating = self.animating(now);
        if let Some(next) = self.steps.iter().map(|s| s.0).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.min(b)))) {
            let wait = (next - now).max(0.0);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(wait as u64));
        }
        if animating {
            ui.ctx().request_repaint();
        }
        animating
    }

    fn animating(&self, now: f64) -> bool {
        self.cards.iter().any(|c| c.active(now) || c.remove_at.is_some())
            || self.tiles.iter().any(|t| t.enter.active(now) || t.hover.active(now) || t.sel.active(now))
            || self.headings.iter().any(|h| h.enter.active(now))
            || self.titles.iter().any(|l| l.y.active(now) || l.o.active(now) || l.leaving)
            || [&self.backdrop, &self.picker_o, &self.settings_o, &self.welcome_o, &self.caption_swap, &self.caption_enter, &self.caption_close, &self.hints_enter, &self.grid_scroll]
                .iter()
                .any(|t| t.active(now))
            || self.glow.active(now)
            || self.edge_flash.is_some_and(|t| now - t < 620.0)
            || self.apply.is_some()
            || self.panel.animating(now)
            || self.show_filter_chip // caret blink
            || self.tiles.iter().any(|t| t.img_at.is_some_and(|a| now - a < 400.0))
            || self.cards.iter().any(|c| c.img_at.iter().flatten().any(|a| now - a < 400.0))
            || matches!(self.phase, Phase::Opening | Phase::Closing)
    }

    fn run_step(&mut self, s: Step, thumbs: &Thumbs, host: &mut dyn Host) {
        let now = self.now;
        match s {
            Step::CaptionSwap => {
                self.capmode_side = self.is_expanded() && self.phase != Phase::Applying;
                self.last_title.clear();
                self.write_caption(0);
                self.caption_swap.set(1.0, now + 16.0, t(300.0, anim::OUT));
            }
            Step::CollapseFold(tok) => self.collapse_fold(tok),
            Step::CollapseReveal(tok) => self.collapse_reveal(tok),
            Step::EnterRelease(tok) => self.enter_release(tok),
            Step::EnterCaption(tok) => {
                if tok == self.enter_token {
                    self.caption_enter.set(1.0, now, t(420.0, anim::MOVE));
                }
            }
            Step::EnterHints(tok) => {
                if tok == self.enter_token {
                    self.hints_enter.set(1.0, now, t(300.0, anim::OUT));
                }
            }
            Step::EnterDelays(tok) => {
                if tok == self.enter_token {
                    for c in self.cards.iter_mut() {
                        c.delay = 0.0;
                    }
                }
            }
            Step::OpenShow(tok) => self.open_show(tok, thumbs),
            Step::OpenDone(tok) => self.open_done(tok),
            Step::CloseDone => self.close_done(host),
            Step::ApplyAnimDone => {
                if let Some(a) = &mut self.apply {
                    a.anim_done = true;
                }
                self.apply_progress(host);
            }
            Step::ApplyClose => self.close(host),
            Step::MoveSelect(i, tok) => self.move_select(i, tok, host),
            Step::MoveEnter(tok) => self.move_enter(tok),
            Step::TitleRemove(id) => self.titles.retain(|l| l.id != id),
        }
    }

    // ---------- input ----------

    fn hit_at(&self, pos: Pos2) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _, enabled)| *enabled && r.contains(pos)).map(|(_, h, _)| h.clone())
    }

    fn handle_input(&mut self, ui: &mut egui::Ui, host: &mut dyn Host) {
        let events = ui.input(|i| i.events.clone());
        let modifiers = ui.input(|i| i.modifiers);
        self.pointer = ui.input(|i| i.pointer.latest_pos());
        self.hovered = self.pointer.and_then(|p| self.hit_at(p));
        let active = !matches!(self.phase, Phase::Hidden | Phase::Closing);
        for ev in events {
            if !active || matches!(self.phase, Phase::Hidden | Phase::Closing) {
                break;
            }
            match ev {
                egui::Event::Key { key, pressed: true, modifiers, .. } => self.on_key(key, modifiers, host),
                egui::Event::Text(text) => self.on_text(&text, modifiers),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers } => {
                    self.on_pointer(pos, pressed, modifiers, host)
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    let scale = match unit {
                        egui::MouseWheelUnit::Point => 1.0,
                        egui::MouseWheelUnit::Line => 100.0 / 3.0 * 3.0 / 1.0,
                        egui::MouseWheelUnit::Page => self.geo.vh,
                    };
                    // egui: positive y = content moves down (wheel up). DOM deltaY is the opposite.
                    self.on_wheel(-delta.x * scale, -delta.y * scale);
                }
                _ => {}
            }
        }
    }

    fn on_text(&mut self, text: &str, modifiers: Modifiers) {
        if self.phase != Phase::Open || self.view != View::Picker || self.busy || self.moving || self.panel.recording {
            return;
        }
        if modifiers.ctrl || modifiers.alt {
            return;
        }
        for ch in text.chars() {
            if ch.is_control() || (ch == ' ' && self.filter.is_empty()) {
                continue;
            }
            self.filter.push(ch);
            self.apply_filter();
        }
    }

    fn on_key(&mut self, key: Key, m: Modifiers, host: &mut dyn Host) {
        if self.panel.recording {
            if let Some(accel) = panel::record_key(key, m) {
                match accel {
                    Err(msg) => self.panel.record_hint = msg,
                    Ok(None) => self.panel.stop_recording(&self.settings),
                    Ok(Some(accel)) => {
                        self.panel.recording = false;
                        self.save_settings(host, |s| s.shortcut = accel, false);
                    }
                }
            }
            return;
        }
        if self.phase == Phase::Applying {
            return;
        }
        if self.busy && key != Key::Escape {
            return;
        }
        match self.view {
            View::Welcome => {
                if key == Key::Escape {
                    self.dismiss_welcome(host);
                } else if self.panel.handle_key(key, m, View::Welcome, self.now) {
                    let acts = std::mem::take(&mut self.panel.actions);
                    self.run_panel_actions(acts, host);
                } else if key == Key::Enter {
                    self.leave_welcome(host);
                }
                return;
            }
            View::Settings => {
                if key == Key::Escape {
                    if self.panel.select_open {
                        self.panel.close_selects(self.now);
                    } else {
                        self.leave_settings(host);
                    }
                } else if m.ctrl && key == Key::Tab {
                    self.panel.close_selects(self.now);
                    let multi = self.multi_screen();
                    self.panel.step_tab(if m.shift { -1 } else { 1 }, multi, self.now);
                } else if self.panel.handle_key(key, m, View::Settings, self.now) {
                    let acts = std::mem::take(&mut self.panel.actions);
                    self.run_panel_actions(acts, host);
                }
                return;
            }
            View::Picker => {}
        }
        if self.phase != Phase::Open && self.phase != Phase::Opening {
            return;
        }
        if m.ctrl && key == Key::Comma {
            self.show_settings(true);
            return;
        }
        if m.ctrl && key == Key::H {
            self.toggle_ui(host);
            return;
        }
        if m.ctrl {
            let dir = match key {
                Key::ArrowLeft => Some("left"),
                Key::ArrowRight => Some("right"),
                Key::ArrowUp => Some("up"),
                Key::ArrowDown => Some("down"),
                _ => None,
            };
            if let Some(d) = dir {
                let n = self.neighbor(d);
                self.select_tile(n, host);
                return;
            }
            if key == Key::L {
                self.select_lock(host);
                return;
            }
            let digit = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9]
                .iter()
                .position(|k| *k == key);
            if let Some(d) = digit {
                self.select_screen(d, host);
                return;
            }
        }
        if self.moving {
            return;
        }
        let handled = if self.layout_grid() { self.grid_keys(key) } else { self.row_keys(key) };
        if handled {
            return;
        }
        match key {
            Key::Enter => self.apply_selected(m.shift),
            Key::Tab => self.toggle_layout(host),
            Key::Escape => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.apply_filter();
                } else if self.is_expanded() {
                    self.collapse();
                } else if !self.busy {
                    self.close(host);
                }
            }
            Key::Backspace => {
                if !self.filter.is_empty() {
                    if m.ctrl {
                        self.filter.clear();
                    } else {
                        self.filter.pop();
                    }
                    self.apply_filter();
                } else if self.is_expanded() {
                    self.collapse();
                }
            }
            _ => {}
        }
    }

    fn row_keys(&mut self, key: Key) -> bool {
        let expanded = self.is_expanded();
        let is_folder = matches!(self.entry(), Some(Entry::Folder { .. }));
        let row = self.row as i64;
        let col = self.col.unwrap_or(0) as i64;
        match key {
            Key::ArrowRight => self.select_row(row + 1),
            Key::ArrowLeft => self.select_row(row - 1),
            Key::ArrowDown => {
                if expanded {
                    self.select_col(col + 1)
                } else if is_folder {
                    self.expand()
                }
            }
            Key::ArrowUp => {
                if expanded {
                    if col > 0 {
                        self.select_col(col - 1)
                    } else {
                        self.collapse()
                    }
                }
            }
            Key::PageDown => {
                if expanded {
                    self.select_col(col + 5)
                } else {
                    self.select_row(row + 5)
                }
            }
            Key::PageUp => {
                if expanded {
                    self.select_col(col - 5)
                } else {
                    self.select_row(row - 5)
                }
            }
            Key::Home => {
                if expanded {
                    self.select_col(0)
                } else {
                    self.select_row(0)
                }
            }
            Key::End => {
                if expanded {
                    let n = self.folder_items().len() as i64;
                    self.select_col(n - 1)
                } else {
                    let n = self.entries.len() as i64;
                    self.select_row(n - 1)
                }
            }
            _ => return false,
        }
        true
    }

    fn grid_keys(&mut self, key: Key) -> bool {
        let g = self.gindex as i64;
        match key {
            Key::ArrowRight => self.select_grid(g + 1),
            Key::ArrowLeft => self.select_grid(g - 1),
            Key::ArrowDown => self.grid_vertical(1),
            Key::ArrowUp => self.grid_vertical(-1),
            Key::PageDown => {
                for _ in 0..3 {
                    self.grid_vertical(1)
                }
            }
            Key::PageUp => {
                for _ in 0..3 {
                    self.grid_vertical(-1)
                }
            }
            Key::Home => self.select_grid(0),
            Key::End => {
                let n = self.flat.len() as i64;
                self.select_grid(n - 1)
            }
            _ => return false,
        }
        true
    }

    fn on_wheel(&mut self, dx: f32, dy: f32) {
        if self.phase != Phase::Open || self.view == View::Picker && self.layout_grid() {
            if self.phase == Phase::Open && self.view == View::Picker {
                // native scrolling of the grid
                let max = (self.grid_content - self.grid_view_h).max(0.0);
                let target = (self.grid_scroll.target() + dy).clamp(0.0, max);
                self.grid_scroll.set(target, self.now, t(160.0, anim::OUT));
            }
            return;
        }
        if self.view != View::Picker {
            self.panel.scroll(dy, self.now);
            return;
        }
        if self.busy {
            return;
        }
        let now = self.now;
        let horizontal = dx.abs() > dy.abs();
        let axis_col = self.is_expanded() && !horizontal;
        if now - self.wheel_last > 220.0 || axis_col != self.wheel_axis_col {
            self.wheel_acc = 0.0;
        }
        self.wheel_last = now;
        self.wheel_axis_col = axis_col;
        self.wheel_acc += if horizontal { dx } else { dy };
        if self.wheel_acc.abs() >= 60.0 && now - self.wheel_step > 80.0 {
            let step = self.wheel_acc.signum() as i64;
            if axis_col {
                if step < 0 && self.col == Some(0) {
                    self.collapse();
                } else {
                    let c = self.col.unwrap_or(0) as i64;
                    self.select_col(c + step);
                }
            } else {
                let r = self.row as i64;
                self.select_row(r + step);
            }
            self.wheel_acc = 0.0;
            self.wheel_step = now;
        }
    }

    fn on_pointer(&mut self, pos: Pos2, pressed: bool, m: Modifiers, host: &mut dyn Host) {
        let hit = self.hit_at(pos);
        if pressed {
            self.press = hit.clone().map(|h| (h, pos));
            self.on_mousedown(hit.as_ref(), host);
            return;
        }
        // click = press and release on the same element
        let Some((down, _)) = self.press.take() else { return };
        if hit.as_ref() != Some(&down) {
            return;
        }
        self.on_click(down, m.shift, host);
    }

    fn on_mousedown(&mut self, hit: Option<&Hit>, host: &mut dyn Host) {
        if self.phase != Phase::Open {
            return;
        }
        // any mousedown outside an open select closes it
        if self.panel.select_open && !matches!(hit, Some(Hit::Control(panel::Ctl::LockSelect | panel::Ctl::LockOption(_)))) {
            self.panel.close_selects(self.now);
            if self.view == View::Settings {
                return;
            }
        }
        match self.view {
            View::Picker => {
                if matches!(hit, Some(Hit::Card(_) | Hit::Tile(_) | Hit::Caption | Hit::MapScreen(_) | Hit::MapLock | Hit::Hints | Hit::Button(_))) {
                    return;
                }
                if self.is_expanded() {
                    self.collapse();
                } else {
                    self.close(host);
                }
            }
            View::Settings => {
                if hit.is_none() {
                    self.leave_settings(host);
                }
            }
            View::Welcome => {
                if hit.is_none() {
                    self.dismiss_welcome(host);
                }
            }
        }
    }

    fn on_click(&mut self, hit: Hit, shift: bool, host: &mut dyn Host) {
        match hit {
            Hit::Card(key) => {
                if let Some(path) = key.strip_prefix("c:") {
                    let path = path.to_string();
                    self.on_col_click(&path, shift);
                } else if let Some(k) = key.strip_prefix("r:") {
                    let k = k.to_string();
                    self.on_row_click(&k, shift);
                }
            }
            Hit::Tile(idx) => {
                if self.phase != Phase::Open {
                    return;
                }
                if idx == self.gindex {
                    self.apply_selected(shift);
                } else {
                    self.select_grid(idx as i64);
                }
            }
            Hit::MapScreen(i) => self.select_screen(i, host),
            Hit::MapLock => self.select_lock(host),
            Hit::Button(action) => self.run_action(action, host),
            Hit::Control(c) => {
                self.panel.click(c, self.now);
                let acts = std::mem::take(&mut self.panel.actions);
                self.run_panel_actions(acts, host);
            }
            _ => {}
        }
    }

    fn run_action(&mut self, action: &str, host: &mut dyn Host) {
        match action {
            "pick-folder" => host.pick_folder(),
            "open-folder" => self.open_folder(host),
            "finish-welcome" => self.leave_welcome(host),
            "settings-back" => self.leave_settings(host),
            "quit" => host.quit(),
            _ => {}
        }
    }

    fn run_panel_actions(&mut self, acts: Vec<panel::Action>, host: &mut dyn Host) {
        for a in acts {
            match a {
                panel::Action::Run(name) => self.run_action(name, host),
                panel::Action::Set(patch) => {
                    let p = patch.clone();
                    // immediate UI effects
                    p.apply(&mut self.settings);
                    if matches!(p, panel::Patch::MapSize(_) | panel::Patch::LockSpot(_) | panel::Patch::LockMode(_)) {
                        self.render_minimap();
                    }
                    self.save_settings(host, move |s| patch.apply(s), false);
                }
            }
        }
    }

    pub fn next_due(&self) -> Option<f64> {
        self.steps.iter().map(|s| s.0).reduce(f64::min)
    }

    pub fn set_viewport(&mut self, vw: f32, vh: f32) {
        self.measure(vw, vh);
    }
}

pub const LOCK_SCALE: f32 = 0.65;

pub fn middle(r: [f32; 4]) -> (f32, f32) {
    ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0)
}

pub fn bounds(rects: &[[f32; 4]]) -> [f32; 4] {
    let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for r in rects {
        b[0] = b[0].min(r[0]);
        b[1] = b[1].min(r[1]);
        b[2] = b[2].max(r[2]);
        b[3] = b[3].max(r[3]);
    }
    b
}

thread_local! {
    static URGENT: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static APPLY_REQ: std::cell::RefCell<Option<(usize, bool)>> = const { std::cell::RefCell::new(None) };
    static LOCK_REQ: std::cell::RefCell<bool> = const { std::cell::RefCell::new(false) };
    pub static THUMB_COLORS: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(HashMap::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prettify_matches_js() {
        assert_eq!(prettify("mountain_lake-1920x1080"), "Mountain lake");
        assert_eq!(prettify("Aurora.Borealis"), "Aurora Borealis");
        assert_eq!(prettify("vagabond-almond-blossom"), "Vagabond almond blossom");
        assert_eq!(prettify("city 3840 x 2160 night"), "City night");
        assert_eq!(prettify("1920x1080"), "1920x1080");
    }
    #[test]
    fn natural_sort() {
        let mut v = vec!["b10", "B2", "a"];
        v.sort_by(|a, b| natural_cmp_ci(a, b));
        assert_eq!(v, vec!["a", "B2", "b10"]);
    }
}
