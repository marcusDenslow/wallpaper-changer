//! Colours, fonts and painting helpers that mirror style.css.

use crate::render::{self, Draw, Fx, Mask};
use egui::{text::LayoutJob, Align2, Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect, Stroke, Vec2};
use std::sync::Arc;

pub const INK: [u8; 3] = [244, 242, 238];
pub const SHADE: [u8; 3] = [6, 7, 10];
pub const ERROR: [u8; 3] = [255, 157, 140];
pub const DARK_TEXT: [u8; 3] = [22, 23, 28];
pub const CARD_BG: [u8; 3] = [30, 31, 38];

pub fn rgba(c: [u8; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}
pub fn ink(a: f32) -> Color32 {
    rgba(INK, a)
}
pub fn white(a: f32) -> Color32 {
    rgba([255, 255, 255], a)
}
pub fn black(a: f32) -> Color32 {
    rgba([0, 0, 0], a)
}
pub fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// `color-mix(in oklab, glow P%, other)` approximated in premultiplied sRGB.
pub fn mix(glow: [f32; 3], p: f32, other: [u8; 3], other_a: f32) -> Color32 {
    let a = p + (1.0 - p) * other_a;
    if a <= 0.0 {
        return Color32::TRANSPARENT;
    }
    let ch = |g: f32, o: u8| ((g * p + o as f32 / 255.0 * (1.0 - p) * other_a) / a * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(ch(glow[0], other[0]), ch(glow[1], other[1]), ch(glow[2], other[2]), (a * 255.0).round() as u8)
}
pub fn glow_alpha(glow: [f32; 3], a: f32) -> Color32 {
    let c = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(c(glow[0]), c(glow[1]), c(glow[2]), (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// glowFor() from app.js: thumbnail ambient colour → softened HSL glow.
pub fn glow_for(hex: &str) -> Option<[f32; 3]> {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    let (r, g, b) = (((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let (mut h, mut s) = (0.0, 0.0);
    if max != min {
        let d = max - min;
        s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h /= 6.0;
    }
    let big_l = l.clamp(0.5, 0.66);
    let big_s = if s < 0.06 { s } else { (s * 0.8).clamp(0.2, 0.5) };
    // the CSS rounds to whole degrees / percents
    let h = (h * 360.0).round() / 360.0;
    let big_s = (big_s * 100.0).round() / 100.0;
    let big_l = (big_l * 100.0).round() / 100.0;
    Some(hsl(h, big_s, big_l))
}

fn hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0
        }
        if t > 1.0 {
            t -= 1.0
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0)]
}

pub const DEFAULT_GLOW: [f32; 3] = [0x7d as f32 / 255.0, 0x8a as f32 / 255.0, 0xa3 as f32 / 255.0];

// ---------- fonts ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Sans400,
    Sans450,
    Sans500,
    Sans600,
    /// Tabular figures (`font-variant-numeric: tabular-nums` in style.css).
    Sans450Tnum,
    Sans500Tnum,
    Sans600Tnum,
    /// Bodoni at the optical size closest to the rendered size.
    Serif,
}

pub fn family(face: Face, size: f32) -> FontFamily {
    FontFamily::Name(
        match face {
            Face::Sans400 => "sans400",
            Face::Sans450 => "sans450",
            Face::Sans500 => "sans500",
            Face::Sans600 => "sans600",
            Face::Sans450Tnum => "sans450tnum",
            Face::Sans500Tnum => "sans500tnum",
            Face::Sans600Tnum => "sans600tnum",
            Face::Serif if size < 34.0 => "serif26",
            Face::Serif if size < 64.0 => "serif44",
            Face::Serif => "serif96",
        }
        .into(),
    )
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    let fonts: [(&str, &'static [u8]); 10] = [
        ("sans400", include_bytes!("../../assets/sans-400.ttf")),
        ("sans450", include_bytes!("../../assets/sans-450.ttf")),
        ("sans500", include_bytes!("../../assets/sans-500.ttf")),
        ("sans600", include_bytes!("../../assets/sans-600.ttf")),
        // tools/freeze_tnum.py
        ("sans450tnum", include_bytes!("../../assets/sans-450-tnum.ttf")),
        ("sans500tnum", include_bytes!("../../assets/sans-500-tnum.ttf")),
        ("sans600tnum", include_bytes!("../../assets/sans-600-tnum.ttf")),
        ("serif26", include_bytes!("../../assets/serif-26.ttf")),
        ("serif44", include_bytes!("../../assets/serif-44.ttf")),
        ("serif96", include_bytes!("../../assets/serif-96.ttf")),
    ];
    // The browser fell back to Segoe UI for glyphs the subset lacks (← →, other scripts).
    let windir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    let segoe = |file: &str| std::fs::read(windir.join("Fonts").join(file)).ok();
    let mut fallbacks: Vec<String> = Vec::new();
    for (name, file) in [("segoe", "segoeui.ttf"), ("segoe-sym", "seguisym.ttf")] {
        if let Some(bytes) = segoe(file) {
            defs.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            fallbacks.push(name.into());
        }
    }
    let defaults = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in fonts {
        defs.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
        let mut list = vec![name.to_string()];
        list.extend(fallbacks.iter().cloned());
        list.extend(defaults.iter().cloned());
        defs.families.insert(FontFamily::Name(name.into()), list);
    }
    ctx.set_fonts(defs);
}

pub struct Text<'a> {
    pub text: &'a str,
    pub size: f32,
    pub face: Face,
    pub color: Color32,
    pub letter: f32,
    pub line_height: Option<f32>,
}

pub fn text(text: &str, size: f32, face: Face, color: Color32) -> Text<'_> {
    Text { text, size, face, color, letter: 0.0, line_height: None }
}

impl Text<'_> {
    pub fn letter(mut self, em: f32) -> Self {
        self.letter = em * self.size;
        self
    }
    pub fn line(mut self, lh: f32) -> Self {
        self.line_height = Some(lh);
        self
    }
    pub fn job(&self, max_width: Option<f32>) -> LayoutJob {
        let mut job = LayoutJob::default();
        let format = egui::TextFormat {
            font_id: FontId::new(self.size, family(self.face, self.size)),
            color: self.color,
            extra_letter_spacing: self.letter,
            line_height: self.line_height,
            ..Default::default()
        };
        job.append(self.text, 0.0, format);
        if let Some(w) = max_width {
            job.wrap.max_width = w;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
        }
        job
    }
    pub fn galley(&self, painter: &Painter, max_width: Option<f32>) -> Arc<Galley> {
        painter.layout_job(self.job(max_width))
    }
}

/// Greedy word wrap like a CSS block of `max_width`: one galley per line, so each line
/// can be centred on its own (`text-align: center`).
pub fn wrap_lines(painter: &Painter, t: Text, max_width: f32) -> Vec<Arc<Galley>> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in t.text.split(' ') {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        let width = Text { text: &candidate, ..t }.galley(painter, None).size().x;
        if !line.is_empty() && width > max_width {
            lines.push(Text { text: &line, ..t }.galley(painter, None));
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    lines.push(Text { text: &line, ..t }.galley(painter, None));
    lines
}

/// Lays out text truncated at the *start* (CSS `direction: rtl` + ellipsis on a path).
pub fn galley_truncate_start(painter: &Painter, t: Text, max_width: f32) -> Arc<Galley> {
    let full = t.galley(painter, None);
    if full.size().x <= max_width {
        return full;
    }
    let chars: Vec<char> = t.text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        let s: String = std::iter::once('…').chain(chars[mid..].iter().copied()).collect();
        let g = Text { text: &s, ..t }.galley(painter, None);
        if g.size().x <= max_width {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let s: String = std::iter::once('…').chain(chars[lo.min(chars.len())..].iter().copied()).collect();
    Text { text: &s, ..t }.galley(painter, None)
}

/// CSS-like text-shadow: a soft blurred cloud plus an optional crisp offset copy.
#[derive(Clone, Copy)]
pub struct TextShadow {
    pub offset: f32,
    pub sharp_alpha: f32,
    pub blur: f32,
    pub blur_alpha: f32,
    pub blur_offset: f32,
}

pub const CAPTION_SHADOW: TextShadow = TextShadow { offset: 1.0, sharp_alpha: 0.35, blur: 24.0, blur_alpha: 0.55, blur_offset: 2.0 };

pub fn draw_galley(painter: &Painter, fx: &Arc<Fx>, screen: Rect, pos: Pos2, g: &Arc<Galley>, alpha: f32, shadow: Option<TextShadow>) {
    if alpha <= 0.002 {
        return;
    }
    if let Some(s) = shadow {
        if s.blur_alpha > 0.0 && !g.is_empty() {
            // ink bounds: roughly the glyph box inside the line box
            let r = g.rect.translate(pos.to_vec2());
            let rows = g.rows.len().max(1) as f32;
            let row_h = r.height() / rows;
            let ink = Rect::from_min_max(
                Pos2::new(r.min.x + 2.0, r.min.y + row_h * 0.22),
                Pos2::new(r.min.x + g.rows.iter().map(|row| row.rect().width()).fold(0.0, f32::max) - 2.0, r.max.y - row_h * 0.18),
            );
            if ink.is_positive() {
                painter.add(render::shape(
                    fx,
                    screen,
                    Draw::Shadow {
                        rect: ink,
                        radius: row_h * 0.3,
                        blur: s.blur,
                        spread: 0.0,
                        offset: [0.0, s.blur_offset],
                        color: black(s.blur_alpha * 0.42 * alpha),
                    },
                    Mask::default(),
                ));
            }
        }
        if s.sharp_alpha > 0.0 {
            painter.galley_with_override_text_color(pos + Vec2::new(0.0, s.offset), g.clone(), black(s.sharp_alpha * 0.55 * alpha));
        }
    }
    if alpha >= 0.999 {
        painter.galley(pos, g.clone(), Color32::WHITE);
    } else {
        // multiply every section colour by alpha
        let mut job = (*g.job).clone();
        for s in &mut job.sections {
            s.format.color = fade(s.format.color, alpha);
        }
        let faded = painter.layout_job(job);
        painter.galley(pos, faded, Color32::WHITE);
    }
}

pub fn shadow(painter: &Painter, fx: &Arc<Fx>, screen: Rect, rect: Rect, radius: f32, y: f32, blur: f32, spread: f32, color: Color32) {
    if color.a() == 0 {
        return;
    }
    painter.add(render::shape(fx, screen, Draw::Shadow { rect, radius, blur, spread, offset: [0.0, y], color }, Mask::default()));
}

pub fn ring(painter: &Painter, fx: &Arc<Fx>, screen: Rect, rect: Rect, radius: f32, width: f32, color: Color32) {
    if color.a() == 0 {
        return;
    }
    painter.add(render::shape(fx, screen, Draw::Ring { rect, radius, width, color }, Mask::default()));
}

pub fn bordered(painter: &Painter, rect: Rect, radius: f32, fill: Color32, border: Color32) {
    painter.rect(rect, radius, fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);
}

/// `:focus-visible { outline: 2px solid color-mix(glow 18%, white); outline-offset: 2px }`
pub fn focus_ring(painter: &Painter, rect: Rect, radius: f32, glow: [f32; 3], offset: f32) {
    let c = mix(glow, 0.18, [255, 255, 255], 1.0);
    painter.rect_stroke(rect.expand(offset + 1.0), radius + offset + 1.0, Stroke::new(2.0, c), egui::StrokeKind::Middle);
}

/// A row of `<kbd>` keys. Returns the width used.
pub struct KbdStyle {
    pub height: f32,
    pub min_w: f32,
    pub pad: f32,
    pub size: f32,
    pub radius: f32,
    pub gap: f32,
}
pub const KBD: KbdStyle = KbdStyle { height: 22.0, min_w: 22.0, pad: 6.0, size: 11.5, radius: 6.0, gap: 4.0 };
pub const KBD_LARGE: KbdStyle = KbdStyle { height: 30.0, min_w: 32.0, pad: 9.0, size: 13.0, radius: 8.0, gap: 5.0 };

pub fn kbd_width(painter: &Painter, keys: &[String], st: &KbdStyle) -> f32 {
    let mut w = 0.0;
    for (i, k) in keys.iter().enumerate() {
        let g = text(k, st.size, Face::Sans600, ink(1.0)).galley(painter, None);
        w += (g.size().x + st.pad * 2.0 + 2.0).max(st.min_w);
        if i > 0 {
            w += st.gap;
        }
    }
    w
}

pub fn draw_kbds(painter: &Painter, keys: &[String], st: &KbdStyle, left: f32, centre_y: f32, alpha: f32) -> f32 {
    let mut x = left;
    for k in keys {
        let g = text(k, st.size, Face::Sans600, ink(1.0)).galley(painter, None);
        let w = (g.size().x + st.pad * 2.0 + 2.0).max(st.min_w);
        let r = Rect::from_min_size(Pos2::new(x, centre_y - st.height / 2.0), Vec2::new(w, st.height));
        bordered(painter, r, st.radius, white(0.07 * alpha), white(0.2 * alpha));
        let pos = Pos2::new(r.center().x - g.size().x / 2.0, r.center().y - g.size().y / 2.0);
        painter.galley_with_override_text_color(pos, g, ink(alpha));
        x += w + st.gap;
    }
    x - st.gap - left
}

pub fn pretty_shortcut(accel: &str) -> Vec<String> {
    accel
        .split('+')
        .filter_map(|t| {
            let k = t.trim();
            if k.is_empty() {
                return None;
            }
            let low = k.to_lowercase();
            Some(match low.as_str() {
                "super" | "cmd" | "command" => "Win".into(),
                "control" | "ctrl" => "Ctrl".into(),
                "alt" | "option" => "Alt".into(),
                "shift" => "Shift".into(),
                _ if k.chars().count() == 1 => k.to_uppercase(),
                _ => k.to_string(),
            })
        })
        .collect()
}

pub fn centered_text(painter: &Painter, rect: Rect, g: Arc<Galley>, color: Color32) {
    let pos = Align2::CENTER_CENTER.align_size_within_rect(g.size(), rect).min;
    painter.galley_with_override_text_color(pos, g, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glow_matches_js() {
        // #ff0000 → hsl(0 50% 50%) → rgb(191,64,64)
        let c = glow_for("#ff0000").unwrap();
        assert!((c[0] * 255.0 - 191.0).abs() < 1.5 && (c[1] * 255.0 - 64.0).abs() < 1.5);
        // grey stays grey, lightness clamped to 50%
        let g = glow_for("#101010").unwrap();
        assert!((g[0] - 0.5).abs() < 0.01 && (g[0] - g[2]).abs() < 1e-4);
    }
    #[test]
    fn shortcut_pretty() {
        assert_eq!(pretty_shortcut("Ctrl+Alt+w"), vec!["Ctrl", "Alt", "W"]);
        assert_eq!(pretty_shortcut("super+shift+F5"), vec!["Win", "Shift", "F5"]);
    }
}
