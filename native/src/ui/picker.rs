//! Drawing for the picker: backdrop, row of cards, grid, caption, hints, screen map,
//! the apply zoom and the edge flash. Mirrors the rules in style.css.

use super::style::{self, Face, CAPTION_SHADOW};
use super::{Card, CardKind, Entry, Hit, MetaKind, Phase, Ui, View};
use crate::anim::{self, t, Tween};
use crate::render::{self, Draw, Fx, Mask};
use crate::thumbs::Thumbs;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use std::sync::Arc;

/// Extra per-frame visual state that only the drawing code needs.
pub struct Extra {
    pub title_vis: Option<Tween>,
    pub meta_vis: Option<Tween>,
    pub map_vis: Option<Tween>,
    pub hints_vis: Option<Tween>,
    pub map_fx: Vec<(Tween, Tween)>,
    pub map_img: Vec<Option<f64>>,
    pub grid_hover: Option<usize>,
    pub caption_rect: Rect,
    pub map_h: f32,
}

impl Default for Extra {
    fn default() -> Self {
        Self {
            title_vis: None,
            meta_vis: None,
            map_vis: None,
            hints_vis: None,
            map_fx: Vec::new(),
            map_img: Vec::new(),
            grid_hover: None,
            caption_rect: Rect::NOTHING,
            map_h: 0.0,
        }
    }
}

fn vis(t: &mut Option<Tween>, target: f32, now: f64) -> f32 {
    let tw = t.get_or_insert_with(|| Tween::new(target));
    tw.set(target, now, crate::anim::t(260.0, anim::OUT));
    tw.get(now)
}

pub fn gutter(vw: f32) -> f32 {
    (vw * 0.06).max(48.0)
}

fn img_alpha(at: Option<f64>, now: f64, dur: f32) -> f32 {
    match at {
        None => 0.0,
        Some(a) => anim::OUT.at(((now - a) as f32 / dur).clamp(0.0, 1.0)),
    }
}

/// Records when a texture became available, so it fades in like `img.loaded`:
/// instantly if it was ready before the element existed, otherwise from when it arrived.
fn track_img(slot: &mut Option<f64>, born: Option<f64>, created: f64) {
    if let (None, Some(b)) = (*slot, born) {
        *slot = Some(if b <= created { f64::NEG_INFINITY } else { b });
    }
}

pub fn card_rect(ui: &Ui, c: &Card, now: f64) -> Rect {
    let g = &ui.geo;
    let stage_top = g.vh * 0.45 - g.h / 2.0;
    let s = c.s.get(now) * c.k.get(now);
    let cx = g.vw / 2.0 + c.x.get(now);
    let cy = stage_top + g.h / 2.0 + c.y.get(now) + c.ey.get(now);
    Rect::from_center_size(Pos2::new(cx, cy), Vec2::new(g.w * s, g.h * s))
}

pub fn draw(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, thumbs: &mut Thumbs, screen: Rect, now: f64) {
    draw_backdrop(ui, p, fx, screen, now);
    if ui.phase == Phase::Applying {
        draw_apply(ui, p, fx, thumbs, screen, now);
    }
    let alpha = ui.picker_o.get(now);
    if alpha <= 0.001 || ui.phase == Phase::Hidden {
        return;
    }
    if ui.layout_grid() {
        draw_grid(ui, p, fx, thumbs, screen, now, alpha);
        draw_caption(ui, p, fx, screen, now, alpha);
        draw_minimap(ui, p, fx, thumbs, screen, now, alpha);
    } else {
        draw_caption(ui, p, fx, screen, now, alpha);
        draw_minimap(ui, p, fx, thumbs, screen, now, alpha);
        draw_cards(ui, p, fx, thumbs, screen, now, alpha);
    }
}

pub fn draw_top(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, thumbs: &mut Thumbs, screen: Rect, now: f64) {
    draw_hints(ui, p, fx, screen, now);
    if ui.phase != Phase::Applying {
        draw_apply(ui, p, fx, thumbs, screen, now);
    }
    draw_edge(ui, p, fx, screen, now);
}

fn draw_backdrop(ui: &Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64) {
    let o = ui.backdrop.get(now);
    if o <= 0.001 || std::env::var_os("WALLSWITCH_NOBACKDROP").is_some() {
        return;
    }
    let (vw, vh) = (screen.width(), screen.height());
    let shade = style::rgba(style::SHADE, o);
    let (centre, radii, stops) = match ui.settings.dim.as_str() {
        "clear" => (Pos2::new(vw * 0.5, vh * 0.56), [vw * 0.58, vh * 0.52], [[0.0, 0.5], [0.6, 0.2], [0.85, 0.0]]),
        "deep" => (Pos2::new(vw * 0.5, vh * 0.5), [vw, vh], [[0.0, 0.9], [1.0, 0.9], [2.0, 0.9]]),
        _ => (Pos2::new(vw * 0.5, vh * 0.52), [vw * 1.1, vh * 0.85], [[0.0, 0.86], [0.48, 0.74], [1.0, 0.5]]),
    };
    p.add(render::shape(fx, screen, Draw::Radial { area: screen, centre, radii, stops, color: shade }, Mask::default()));
    let glow = style::glow_alpha(ui.glow.get(now), o);
    p.add(render::shape(
        fx,
        screen,
        Draw::Radial {
            area: screen,
            centre: Pos2::new(vw * 0.5, vh * 0.45),
            radii: [vw * 0.48, vh * 0.42],
            stops: [[0.0, 0.13], [0.72, 0.0], [0.72, 0.0]],
            color: glow,
        },
        Mask::default(),
    ));
}

// ---------- cards ----------

fn draw_cards(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, thumbs: &mut Thumbs, screen: Rect, now: f64, alpha: f32) {
    let mut order: Vec<usize> = (0..ui.cards.len()).collect();
    order.sort_by_key(|&i| (ui.cards[i].z, ui.cards[i].order));
    let glow = ui.glow.get(now);
    for i in order {
        let rect = card_rect(ui, &ui.cards[i], now);
        let c = &ui.cards[i];
        let o = c.op.get(now) * alpha;
        if o <= 0.002 || !rect.is_positive() {
            continue;
        }
        let scale = rect.width() / ui.geo.w;
        let b = c.b.get(now);
        let sel = c.sel.get(now);
        let fsel = c.fsel.get(now);
        let key = c.key.clone();
        let kind = c.kind.clone();
        let pointer = !c.pointer_none();
        let mut img_at = c.img_at;
        match kind {
            CardKind::Wall { item } => {
                let path = ui.items[item].path.clone();
                let tex = thumbs.tex(&path);
                track_img(&mut img_at[0], tex.as_ref().map(|t| t.1), c.created);
                let current = ui.is_current(item);
                let error = thumbs.is_error(&path);
                draw_sheet(
                    p,
                    fx,
                    screen,
                    rect,
                    14.0 * scale,
                    scale,
                    b,
                    o,
                    sel,
                    glow,
                    tex.map(|t| t.0),
                    img_alpha(img_at[0], now, 360.0),
                    current,
                    if error { Some(ui.items[item].file_name.clone()) } else { None },
                    [0.75, 24.0, 60.0, -28.0],
                );
                if !thumbs.has(&path) {
                    thumbs.want(&path, false);
                }
            }
            CardKind::Folder { entry } => {
                let Some(Entry::Folder { items, .. }) = ui.entries.iter().find(|e| matches!(e, Entry::Folder { key, .. } if *key == entry)).cloned()
                else {
                    continue;
                };
                let any_current = items.iter().any(|&it| ui.is_current(it));
                // DOM order s3, s2, s1 → s1 on top
                for (si, (ty0, ty1, sc, bright)) in [(0.19, 0.23, 0.84, 0.38), (0.10, 0.12, 0.92, 0.6), (0.0, 0.0, 1.0, 1.0)].into_iter().enumerate() {
                    let sheet = 2 - si;
                    let Some(&it) = items.get(sheet) else { continue };
                    let path = ui.items[it].path.clone();
                    let tex = thumbs.tex(&path);
                    track_img(&mut img_at[sheet], tex.as_ref().map(|t| t.1), c.created);
                    let ty = (ty0 + (ty1 - ty0) * fsel) * rect.height();
                    let r = Rect::from_center_size(rect.center() + Vec2::new(0.0, ty), rect.size() * sc);
                    let is_top = sheet == 0;
                    let error = thumbs.is_error(&path);
                    draw_sheet(
                        p,
                        fx,
                        screen,
                        r,
                        14.0 * scale * sc,
                        scale * sc,
                        b * bright,
                        o,
                        if is_top { sel } else { 0.0 },
                        glow,
                        tex.map(|t| t.0),
                        img_alpha(img_at[sheet], now, 360.0),
                        is_top && any_current,
                        if error { Some(ui.items[it].file_name.clone()) } else { None },
                        [0.8, 24.0, 60.0, -28.0],
                    );
                    if !thumbs.has(&path) {
                        thumbs.want(&path, false);
                    }
                }
                // count badge: right 14 bottom 12, padding 3 10, 13px 600
                let g = style::text(&items.len().to_string(), 13.0 * scale, Face::Sans600, style::ink(1.0)).galley(p, None);
                let pad = Vec2::new(10.0, 3.0) * scale;
                let line_h = 13.0 * 1.45 * scale;
                let size = Vec2::new(g.size().x + pad.x * 2.0, line_h + pad.y * 2.0);
                let br = Rect::from_min_size(rect.max - Vec2::new(14.0, 12.0) * scale - size, size);
                p.rect_filled(br, br.height() / 2.0, style::rgba([10, 11, 14], 0.62 * o).gamma_multiply(1.0));
                let pos = Pos2::new(br.center().x - g.size().x / 2.0, br.center().y - g.size().y / 2.0);
                p.galley_with_override_text_color(pos, g, style::fade(style::lerp_color(Color32::BLACK, style::ink(1.0), b), o));
            }
        }
        ui.cards[i].img_at = img_at;
        if pointer {
            ui.hits.push((rect, Hit::Card(key), true));
        }
    }
}

/// One rounded image surface with shadow, ring, current dot and optional fallback name.
#[allow(clippy::too_many_arguments)]
fn draw_sheet(
    p: &Painter,
    fx: &Arc<Fx>,
    screen: Rect,
    r: Rect,
    radius: f32,
    scale: f32,
    b: f32,
    o: f32,
    sel: f32,
    glow: [f32; 3],
    tex: Option<render::Tex>,
    img_a: f32,
    current: bool,
    fallback: Option<String>,
    base_shadow: [f32; 4],
) {
    // box-shadow (filter: brightness applies to it as well)
    let k = scale;
    if sel < 0.999 {
        style::shadow(p, fx, screen, r, radius, base_shadow[1] * k, base_shadow[2] * k, base_shadow[3] * k, style::black(base_shadow[0] * o * (1.0 - sel)));
    }
    if sel > 0.001 {
        style::shadow(p, fx, screen, r, radius, 50.0 * k, 110.0 * k, -40.0 * k, style::black(0.9 * o * sel));
        style::shadow(p, fx, screen, r, radius, 0.0, 160.0 * k, -40.0 * k, style::glow_alpha(glow.map(|c| c * b), 0.28 * o * sel));
    }
    // background + image
    let bg = style::CARD_BG.map(|c| (c as f32 * b) as u8);
    p.rect_filled(r, radius, style::rgba(bg, o));
    if let Some(tex) = tex {
        if img_a > 0.0 {
            p.add(render::shape(
                fx,
                screen,
                Draw::Image { rect: r, radius, tex, brightness: b, saturate: 0.55 + b * 0.45, opacity: o * img_a },
                Mask::default(),
            ));
        }
    } else if let Some(name) = fallback {
        let g = style::text(&name, 13.0 * k, Face::Sans400, style::ink(0.42 * o)).galley(p, Some((r.width() - 32.0 * k).max(1.0)));
        p.galley(Pos2::new(r.min.x + 16.0 * k, r.max.y - 14.0 * k - g.size().y), g, Color32::WHITE);
    }
    // ::after ring
    let ring_c = style::lerp_color(style::white(0.1), style::mix(glow, 0.16, [255, 255, 255], 0.7), sel);
    let ring_c = style::fade(scale_rgb(ring_c, b), o);
    style::ring(p, fx, screen, r, radius, (1.0 + 0.5 * sel) * k, ring_c);
    if current {
        let c = r.min + Vec2::new(14.0 + 3.5, 14.0 + 3.5) * k;
        dot(p, fx, screen, c, 3.5 * k, b, o, true);
    }
}

fn scale_rgb(c: Color32, b: f32) -> Color32 {
    let [r, g, bl, a] = c.to_array();
    Color32::from_rgba_premultiplied((r as f32 * b) as u8, (g as f32 * b) as u8, (bl as f32 * b) as u8, a)
}

/// `.current-dot`: ink circle, 3px dark ring, 12px white glow.
fn dot(p: &Painter, fx: &Arc<Fx>, screen: Rect, c: Pos2, r: f32, b: f32, o: f32, ringed: bool) {
    let k = r / 3.5;
    let rect = Rect::from_center_size(c, Vec2::splat(r * 2.0));
    style::shadow(p, fx, screen, rect, r, 0.0, if ringed { 12.0 } else { 10.0 } * k, 0.0, style::white(0.6 * o * b));
    if ringed {
        p.circle_filled(c, r + 3.0 * k, style::black(0.25 * o));
    }
    p.circle_filled(c, r, style::fade(scale_rgb(style::ink(1.0), b), o));
}

// ---------- grid ----------

fn grid_stage(ui: &Ui) -> Rect {
    let g = gutter(ui.geo.vw);
    let bottom = if ui.map_on() { ui.pk.map_h.max(0.0) + 110.0 } else { 96.0 };
    Rect::from_min_max(Pos2::new(g, 204.0), Pos2::new(ui.geo.vw - g, (ui.geo.vh - bottom).max(204.0 + 50.0)))
}

fn layout_grid(ui: &mut Ui, stage: Rect, p: &Painter) {
    let inner_w = stage.width() - 24.0;
    let cols = (((inner_w + 20.0) / (280.0f32.min(inner_w) + 20.0)).floor() as usize).max(1);
    let col_w = (inner_w - 20.0 * (cols as f32 - 1.0)) / cols as f32;
    let tile_h = col_w * 9.0 / 16.0;
    let mut y = 12.0;
    let mut tile_i = 0;
    let mut head_i = 0;
    let mut first = true;
    let mut any_row = false;
    let sections = ui.sections.clone();
    for sec in &sections {
        if sec.name.is_some() {
            if any_row {
                y += 20.0;
            }
            let top = y + if first { 0.0 } else { 18.0 };
            let h = 26.0 * 1.2;
            if let Some(hd) = ui.headings.get_mut(head_i) {
                hd.rect = Rect::from_min_size(Pos2::new(12.0 + 2.0, top), Vec2::new(inner_w - 4.0, h));
            }
            head_i += 1;
            y = top + h - 6.0;
            any_row = true;
        }
        first = false;
        for (j, _) in sec.items.iter().enumerate() {
            let c = j % cols;
            if c == 0 {
                if any_row {
                    y += 20.0;
                }
                any_row = true;
            }
            let x = 12.0 + c as f32 * (col_w + 20.0);
            if let Some(t) = ui.tiles.get_mut(tile_i) {
                t.rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(col_w, tile_h));
            }
            tile_i += 1;
            if c == cols - 1 || j == sec.items.len() - 1 {
                y += tile_h;
            }
        }
    }
    let _ = p;
    ui.grid_content = y + 40.0;
}

#[allow(clippy::too_many_arguments)]
fn draw_grid(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, thumbs: &mut Thumbs, screen: Rect, now: f64, alpha: f32) {
    let stage = grid_stage(ui);
    layout_grid(ui, stage, p);
    ui.grid_origin = stage.min;
    ui.grid_view_h = stage.height();
    let max_scroll = (ui.grid_content - stage.height()).max(0.0);
    if ui.grid_center_pending {
        ui.grid_center_pending = false;
        if let Some(tile) = ui.tiles.get(ui.gindex) {
            let target = (tile.rect.center().y - stage.height() / 2.0).clamp(0.0, max_scroll);
            ui.grid_scroll.jump(target);
        }
    }
    if ui.grid_scroll_nearest {
        ui.grid_scroll_nearest = false;
        if let Some(tile) = ui.tiles.get(ui.gindex) {
            let cur = ui.grid_scroll.target();
            let target = if tile.rect.min.y < cur {
                tile.rect.min.y
            } else if tile.rect.max.y > cur + stage.height() {
                tile.rect.max.y - stage.height()
            } else {
                cur
            }
            .clamp(0.0, max_scroll);
            if ui.reduce {
                ui.grid_scroll.jump(target);
            } else {
                ui.grid_scroll.set(target, now, t(380.0, anim::EASE));
            }
        }
    }
    if ui.grid_scroll.target() > max_scroll {
        ui.grid_scroll.jump(max_scroll);
    }
    let scroll = ui.grid_scroll.get(now);
    let off = stage.min.to_vec2() - Vec2::new(0.0, scroll);
    let clip = p.with_clip_rect(stage);
    let mask = Mask { fade_in: Some([stage.min.y, stage.min.y + 12.0]), fade_out: Some([stage.max.y - 48.0, stage.max.y]), clip: None };
    let mask_at = |y: f32| ((y - stage.min.y) / 12.0).clamp(0.0, 1.0) * ((stage.max.y - y) / 48.0).clamp(0.0, 1.0);
    let glow = ui.glow.get(now);
    let closing = ui.phase == Phase::Closing;
    let applying = ui.apply.as_ref().map(|a| ((now - a.start) as f32 / 240.0).clamp(0.0, 1.0));

    // headings
    for h in &ui.headings {
        let r = h.rect.translate(off);
        if r.max.y < stage.min.y - 40.0 || r.min.y > stage.max.y + 40.0 {
            continue;
        }
        let mut a = h.enter.get(now).max(0.0) * alpha * mask_at(r.center().y);
        if closing {
            a *= ((h.enter.get(now) + 1.0) / 2.0).clamp(0.0, 1.0);
        }
        let name = style::text(&h.name, 26.0, Face::Serif, style::ink(1.0)).line(26.0 * 1.2).galley(p, None);
        let count = style::text(&super::plural(h.count, "wallpaper"), 13.0, Face::Sans500, style::ink(0.42)).galley(p, None);
        let base_y = r.min.y;
        let sh = style::TextShadow { offset: 0.0, sharp_alpha: 0.0, blur: 18.0, blur_alpha: 0.5, blur_offset: 2.0 };
        style::draw_galley(&clip, fx, screen, Pos2::new(r.min.x, base_y), &name, a, Some(sh));
        // baseline alignment: serif baseline vs sans baseline
        let name_base = name.rows.first().map(|r| r.row.glyphs.first().map(|g| g.pos.y).unwrap_or(r.rect().max.y * 0.8)).unwrap_or(0.0);
        let count_base = count.rows.first().map(|r| r.row.glyphs.first().map(|g| g.pos.y).unwrap_or(r.rect().max.y * 0.8)).unwrap_or(0.0);
        style::draw_galley(&clip, fx, screen, Pos2::new(r.min.x + name.size().x + 12.0, base_y + name_base - count_base), &count, a, None);
    }

    // tiles
    let hover = ui.hovered.clone();
    let mut order: Vec<usize> = (0..ui.tiles.len()).collect();
    order.sort_by_key(|&i| if i == ui.gindex { 1 } else { 0 });
    for i in order {
        let tile_rect = ui.tiles[i].rect.translate(off);
        let visible = tile_rect.max.y > stage.min.y - 300.0 && tile_rect.min.y < stage.max.y + 300.0;
        let item = ui.tiles[i].item;
        let path = ui.items[item].path.clone();
        if visible && !thumbs.has(&path) {
            thumbs.want(&path, false);
        }
        if tile_rect.max.y < stage.min.y || tile_rect.min.y > stage.max.y {
            continue;
        }
        let selected = i == ui.gindex;
        let hovered = matches!(hover, Some(Hit::Tile(h)) if h == i);
        let tile = &mut ui.tiles[i];
        tile.sel.set(if selected { 1.0 } else { 0.0 }, now, t(260.0, anim::OUT));
        tile.selk.set(if selected { 1.0 } else { 0.0 }, now, t(if ui.reduce { 0.0 } else { 420.0 }, anim::MOVE));
        tile.hover.set(if hovered { 1.0 } else { 0.0 }, now, t(260.0, anim::OUT));
        let e = tile.enter.get(now);
        let (mut o, mut ty, mut sc) = if closing {
            let k = ((e + 1.0) / 2.0).clamp(0.0, 1.0);
            (k, 14.0 * (1.0 - k), 1.0)
        } else {
            let e = e.clamp(0.0, 1.0);
            let red = if ui.reduce { 0.0 } else { 1.0 };
            (e, 26.0 * (1.0 - e) * red, 1.0 - 0.03 * (1.0 - e) * red)
        };
        if let Some(a) = applying {
            o *= 1.0 - anim::OUT.at(a);
        }
        o *= alpha;
        if o <= 0.002 {
            continue;
        }
        let sel = tile.sel.get(now);
        let hv = tile.hover.get(now) * (1.0 - sel);
        let b = 0.56 + (0.82 - 0.56) * hv + (1.0 - 0.56) * sel;
        let sat = 0.8 + (0.95 - 0.8) * hv + 0.2 * sel;
        sc *= 1.0 + 0.035 * tile.selk.get(now);
        ty += 0.0;
        let r = Rect::from_center_size(tile_rect.center() + Vec2::new(0.0, ty), tile_rect.size() * sc);
        let tex = thumbs.tex(&path);
        track_img(&mut tile.img_at, tex.as_ref().map(|t| t.1), tile.created);
        let img_a = img_alpha(tile.img_at, now, 320.0);
        let shadow_mask = mask;
        let clip_shape = |d: Draw| render::shape(fx, screen, d, shadow_mask);
        // shadows
        if sel < 0.999 {
            clip.add(clip_shape(Draw::Shadow { rect: r, radius: 10.0, blur: 40.0, spread: -26.0, offset: [0.0, 18.0], color: style::black(0.8 * o * (1.0 - sel)) }));
        }
        if sel > 0.001 {
            clip.add(clip_shape(Draw::Shadow { rect: r, radius: 10.0, blur: 70.0, spread: -30.0, offset: [0.0, 30.0], color: style::black(0.9 * o * sel) }));
            clip.add(clip_shape(Draw::Shadow { rect: r, radius: 10.0, blur: 100.0, spread: -30.0, offset: [0.0, 0.0], color: style::glow_alpha(glow, 0.28 * o * sel) }));
        }
        let bg = style::CARD_BG.map(|c| (c as f32 * b) as u8);
        let m = mask_at(r.center().y).max(0.0);
        clip.add(clip_shape(Draw::Linear { rect: r, radius: 10.0, stops: [[0.0, 1.0], [1.0, 1.0]], color: style::rgba(bg, o) }));
        if let Some((tex, _)) = tex {
            clip.add(clip_shape(Draw::Image { rect: r, radius: 10.0, tex, brightness: b, saturate: sat, opacity: o * img_a }));
        } else if thumbs.is_error(&path) {
            let g = style::text(&ui.items[item].file_name, 12.0, Face::Sans400, style::ink(0.42 * o * m)).galley(p, Some((r.width() - 24.0).max(1.0)));
            clip.galley(Pos2::new(r.min.x + 12.0, r.max.y - 10.0 - g.size().y), g, Color32::WHITE);
        }
        let ring_c = style::lerp_color(style::white(0.08), style::mix(glow, 0.16, [255, 255, 255], 0.7), sel);
        clip.add(clip_shape(Draw::Ring { rect: r, radius: 10.0, width: 1.0 + 0.5 * sel, color: style::fade(scale_rgb(ring_c, b), o) }));
        if ui.is_current(item) {
            let c = r.min + Vec2::new(13.0, 13.0);
            let rect = Rect::from_center_size(c, Vec2::splat(6.0));
            clip.add(clip_shape(Draw::Shadow { rect, radius: 3.0, blur: 10.0, spread: 0.0, offset: [0.0, 0.0], color: style::white(0.6 * o * b * m) }));
            clip.circle_filled(c, 3.0, style::fade(scale_rgb(style::ink(1.0), b), o * m));
        }
        let hit_rect = r.intersect(stage);
        if hit_rect.is_positive() {
            ui.hits.push((hit_rect, Hit::Tile(i), true));
        }
    }

    // thin scrollbar
    if max_scroll > 0.5 {
        let track_h = stage.height();
        let thumb_h = (track_h * track_h / ui.grid_content).max(24.0);
        let y = stage.min.y + (track_h - thumb_h) * (scroll / max_scroll);
        let r = Rect::from_min_size(Pos2::new(stage.max.x - 8.0, y), Vec2::new(6.0, thumb_h));
        clip.add(render::shape(fx, screen, Draw::Linear { rect: r, radius: 3.0, stops: [[0.0, 1.0], [1.0, 1.0]], color: style::white(0.18 * alpha) }, mask));
    }
}

// ---------- caption ----------

fn clampf(lo: f32, v: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

fn draw_caption(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64, alpha: f32) {
    let g = ui.geo;
    let gut = gutter(g.vw);
    let grid = ui.layout_grid();
    let side = ui.capmode_side && !grid;
    let (left, right, top, center, title_size) = if grid {
        (gut + 12.0, g.vw - gut, 64.0, false, clampf(36.0, g.vw * 0.032, 60.0))
    } else if side {
        (g.vw / 2.0 + (g.w * 0.9) / 2.0 + 40.0, g.vw - 32.0, g.stage_c, false, clampf(30.0, g.vw * 0.026, 54.0))
    } else {
        (gut, g.vw - gut, g.stage_c + g.h / 2.0 + 52.0, true, clampf(44.0, g.vw * 0.048, 96.0))
    };
    let top = if ui.empty { g.vh * 0.38 } else { top };
    let width = (right - left).max(10.0);

    let s = &ui.settings;
    let hidden = s.ui_hidden;
    let title_t = if hidden && s.hide_title && !ui.empty { 0.0 } else { 1.0 };
    let meta_t = if hidden && s.hide_details && !ui.empty && ui.filter.is_empty() && ui.status.is_none() { 0.0 } else { 1.0 };
    let title_vis = vis(&mut ui.pk.title_vis, title_t, now);
    let meta_vis = vis(&mut ui.pk.meta_vis, meta_t, now);

    let enter = ui.caption_enter.get(now);
    let close = ui.caption_close.get(now);
    let a = alpha * ui.caption_swap.get(now) * enter * close;
    let dy = 14.0 * (1.0 - enter) + 8.0 * (1.0 - close);

    // layout meta first to know the caption height
    let title_h = title_size * 1.12;
    let meta_size = 14.0;
    let meta_line = meta_size * 1.45;
    let mut parts: Vec<(Arc<egui::Galley>, f32, bool)> = Vec::new(); // galley, width, is_chip
    let chip_text = ui.filter.clone();
    if ui.show_filter_chip {
        let gt = style::text(&chip_text, meta_size, Face::Sans450, style::ink(1.0)).letter(0.01).galley(p, None);
        let w = 10.0 + 13.0 + 6.0 + gt.size().x + 6.0 + 2.0 + 1.5 + 12.0;
        parts.push((gt, w, true));
    }
    for (text, kind) in &ui.meta {
        let color = match kind {
            MetaKind::Plain => style::ink(0.66),
            MetaKind::Strong => style::ink(1.0),
            MetaKind::Error => style::rgba(style::ERROR, 1.0),
        };
        let gt = style::text(text, meta_size, Face::Sans450, color).letter(0.01).galley(p, Some(width));
        let w = gt.size().x;
        parts.push((gt, w, false));
    }
    // flex-wrap rows, gap 6px 28px
    let mut rows: Vec<Vec<usize>> = vec![vec![]];
    let mut row_w = 0.0;
    for (i, (_, w, _)) in parts.iter().enumerate() {
        let need = if rows.last().unwrap().is_empty() { *w } else { row_w + 28.0 + w };
        if need > width && !rows.last().unwrap().is_empty() {
            rows.push(vec![i]);
            row_w = *w;
        } else {
            rows.last_mut().unwrap().push(i);
            row_w = need;
        }
    }
    let meta_h = (rows.len() as f32 * meta_line + (rows.len() as f32 - 1.0) * 6.0).max(meta_line);
    let actions_h = if ui.empty_actions { 26.0 + 14.0 * 1.45 + 16.0 + 2.0 } else { 0.0 };
    let total_h = title_h + 14.0 + meta_h + actions_h;
    let top = if side { top - total_h / 2.0 } else { top } + dy;
    let cap_rect = Rect::from_min_size(Pos2::new(left, top), Vec2::new(width, total_h));
    ui.pk.caption_rect = cap_rect;
    if a > 0.01 {
        ui.hits.push((cap_rect, Hit::Caption, true));
    }
    if a <= 0.002 {
        return;
    }

    // title lines (clipped to the title box)
    let title_box = Rect::from_min_size(Pos2::new(left, top), Vec2::new(width, title_h));
    let tp = p.with_clip_rect(title_box.expand2(Vec2::new(40.0, 0.0)));
    for line in &ui.titles {
        let ly = line.y.get(now) * title_h;
        let lo = line.o.get(now);
        let gt = style::text(&line.text, title_size, Face::Serif, style::ink(1.0)).letter(-0.012).line(title_h).galley(p, Some(width));
        let x = if center { left + (width - gt.size().x) / 2.0 } else { left };
        style::draw_galley(&tp, fx, screen, Pos2::new(x, top + ly), &gt, a * lo * title_vis, Some(CAPTION_SHADOW));
    }

    // meta
    let mut y = top + title_h + 14.0;
    let ma = a * meta_vis;
    for row in &rows {
        let rw: f32 = row.iter().map(|&i| parts[i].1).sum::<f32>() + 28.0 * (row.len() as f32 - 1.0).max(0.0);
        let mut x = if center { left + (width - rw) / 2.0 } else { left };
        for &i in row {
            let (gt, w, chip) = &parts[i];
            if *chip {
                let chip_r = Rect::from_min_size(Pos2::new(x, y - 3.0 + 3.0 - 3.0), Vec2::new(*w, meta_line + 6.0));
                let chip_r = chip_r.translate(Vec2::new(0.0, 0.0));
                p.rect_filled(chip_r, chip_r.height() / 2.0, style::white(0.12 * ma));
                // magnifier icon 13x13
                let ic = Pos2::new(x + 10.0, chip_r.center().y - 6.5);
                let k = 13.0 / 16.0;
                let stroke = Stroke::new(1.6 * k, style::ink(0.66 * ma));
                p.circle_stroke(ic + Vec2::new(7.0, 7.0) * k, 4.6 * k, stroke);
                p.line_segment([ic + Vec2::new(10.5, 10.5) * k, ic + Vec2::new(14.0, 14.0) * k], stroke);
                let tx = x + 10.0 + 13.0 + 6.0;
                let ty = chip_r.center().y - gt.size().y / 2.0;
                style::draw_galley(p, fx, screen, Pos2::new(tx, ty), gt, ma, None);
                // caret: 1.5px × 1.05em, blinking steps(1) 1s
                let blink = ((now / 1000.0).fract() < 0.5) as i32 as f32;
                let cx = tx + gt.size().x + 2.0;
                let ch = meta_size * 1.05;
                p.rect_filled(Rect::from_min_size(Pos2::new(cx, chip_r.center().y - ch / 2.0 + 1.0), Vec2::new(1.5, ch)), 0.0, style::ink(ma * blink));
            } else {
                style::draw_galley(p, fx, screen, Pos2::new(x, y + (meta_line - gt.size().y) / 2.0), gt, ma, Some(CAPTION_SHADOW));
            }
            x += w + 28.0;
        }
        y += meta_line + 6.0;
    }

    // empty actions
    if ui.empty_actions {
        let by = top + title_h + 14.0 + meta_h + 26.0;
        let labels = [("Open folder", true, "open-folder"), ("Use another folder", false, "pick-folder")];
        let gals: Vec<_> = labels.iter().map(|(l, _, _)| style::text(l, 14.0, Face::Sans500, style::ink(1.0)).galley(p, None)).collect();
        let bh = 14.0 * 1.45 + 16.0 + 2.0;
        let widths: Vec<f32> = gals.iter().map(|g| g.size().x + 28.0 + 2.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 10.0;
        let mut x = left + (width - total) / 2.0;
        for (i, (_, primary, action)) in labels.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, by), Vec2::new(widths[i], bh));
            let hov = ui.hovered == Some(Hit::Button(action));
            super::panel::paint_button(p, r, &gals[i], *primary, false, hov, a, 9.0);
            ui.hits.push((r, Hit::Button(action), true));
            x += widths[i] + 10.0;
        }
    }
}

// ---------- hints ----------

fn draw_hints(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64) {
    if ui.phase == Phase::Hidden {
        return;
    }
    let hide = ui.settings.ui_hidden && ui.settings.hide_hints && ui.view != View::Welcome;
    let v = vis(&mut ui.pk.hints_vis, if hide { 0.0 } else { 1.0 }, now);
    let mut a = ui.hints_enter.get(now) * v;
    if ui.view == View::Picker {
        a *= ui.picker_o.get(now).max(if ui.view == View::Picker { 0.0 } else { 1.0 });
    }
    if a <= 0.002 {
        return;
    }
    let gut = gutter(ui.geo.vw);
    let max_w = ui.geo.vw - gut * 2.0;
    let narrow = ui.geo.vw < 760.0;
    let items: Vec<_> = ui
        .hints
        .iter()
        .filter(|h| !(narrow && h.optional))
        .map(|h| {
            let label = style::text(&h.label, 12.5, Face::Sans500, style::ink(0.42)).galley(p, None);
            let kw = style::kbd_width(p, &h.keys, &style::KBD);
            (h.keys.clone(), label.clone(), kw + 8.0 + label.size().x)
        })
        .collect();
    let mut rows: Vec<Vec<usize>> = vec![vec![]];
    let mut row_w = 0.0;
    for (i, it) in items.iter().enumerate() {
        let need = if rows.last().unwrap().is_empty() { it.2 } else { row_w + 26.0 + it.2 };
        if need > max_w && !rows.last().unwrap().is_empty() {
            rows.push(vec![i]);
            row_w = it.2;
        } else {
            rows.last_mut().unwrap().push(i);
            row_w = need;
        }
    }
    let row_h = 22.0;
    let total_h = rows.len() as f32 * row_h + (rows.len() as f32 - 1.0) * 10.0;
    let mut y = ui.geo.vh - 34.0 - total_h;
    let block = Rect::from_min_size(Pos2::new(gut, y), Vec2::new(max_w, total_h));
    ui.hits.push((block, Hit::Hints, true));
    for row in rows {
        let w: f32 = row.iter().map(|&i| items[i].2).sum::<f32>() + 26.0 * (row.len() as f32 - 1.0).max(0.0);
        let mut x = (ui.geo.vw - w) / 2.0;
        let cy = y + row_h / 2.0;
        for i in row {
            let (keys, label, total) = &items[i];
            let kw = style::draw_kbds(p, keys, &style::KBD, x, cy, a);
            let lx = x + kw + 8.0;
            let sh = style::TextShadow { offset: 0.0, sharp_alpha: 0.0, blur: 12.0, blur_alpha: 0.5, blur_offset: 1.0 };
            style::draw_galley(p, fx, screen, Pos2::new(lx, cy - label.size().y / 2.0), label, a, Some(sh));
            x += total + 26.0;
        }
        y += row_h + 10.0;
    }
}

// ---------- minimap ----------

const MAP_SIZES: [(&str, f32, f32); 3] = [("small", 0.125, 0.12), ("medium", 0.18, 0.17), ("large", 0.25, 0.24)];

#[allow(clippy::too_many_arguments)]
fn draw_minimap(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, thumbs: &mut Thumbs, screen: Rect, now: f64, alpha: f32) {
    if !ui.map_on() {
        ui.pk.map_h = 0.0;
        return;
    }
    let tiles = ui.map_tiles();
    let screens: Vec<[f32; 4]> = ui.snapshot.screens.iter().map(|s| s.rect.map(|v| v as f32)).collect();
    let [left, top, right, bottom] = super::bounds(&tiles.iter().map(|t| t.1).collect::<Vec<_>>());
    let [bl, bt, br, bb] = super::bounds(&screens);
    let (width, height) = (right - left, bottom - top);
    let size = MAP_SIZES.iter().find(|s| s.0 == ui.settings.map_size).unwrap_or(&MAP_SIZES[1]);
    let large = size.0 == "large";
    let mut box_w = ui.geo.vw * size.1;
    let mut box_h = ui.geo.vh * size.2;
    if ui.lock_own() {
        if ui.settings.lock_spot == "above" {
            box_h *= (height / (bb - bt)).min(1.6);
        } else {
            box_w *= (width / (br - bl)).min(1.6);
        }
    }
    let scale = (box_w / width).min(box_h / height);
    let map_w = (width * scale).round();
    let map_h = (height * scale).round();
    ui.pk.map_h = map_h;
    let origin = Pos2::new(ui.geo.vw - 32.0 - map_w, ui.geo.vh - 84.0 - map_h);
    let hide = ui.settings.ui_hidden && ui.settings.hide_map;
    let v = vis(&mut ui.pk.map_vis, if hide { 0.0 } else { 1.0 }, now);
    let a = alpha * v * ui.caption_close.get(now);
    if ui.pk.map_fx.len() != tiles.len() {
        ui.pk.map_fx = tiles.iter().map(|_| (Tween::new(0.55), Tween::new(0.8))).collect();
        ui.pk.map_img = vec![None; tiles.len()];
    }
    let lock = ui.editing_lock();
    let radius = if large { 7.0 } else { 5.0 };
    let lock_path = ui
        .lock_image
        .clone()
        .flatten()
        .or_else(|| ui.settings.lock_wallpaper.as_ref().map(|p| p.to_string_lossy().into_owned()));
    for (ti, (screen_idx, rect)) in tiles.iter().enumerate() {
        let r = Rect::from_min_size(
            origin + Vec2::new(((rect[0] - left) * scale).round() + 2.0, ((rect[1] - top) * scale).round() + 2.0),
            Vec2::new(((rect[2] - rect[0]) * scale).round() - 4.0, ((rect[3] - rect[1]) * scale).round() - 4.0),
        );
        let (current, path, hit) = match screen_idx {
            Some(i) => {
                let s = &ui.snapshot.screens[*i];
                (s.editing && !lock, s.current.clone(), Hit::MapScreen(*i))
            }
            None => (lock, lock_path.clone(), Hit::MapLock),
        };
        let hovered = ui.hovered.as_ref() == Some(&hit);
        let (tb, ts) = if current { (1.0, 1.0) } else if hovered { (0.85, 0.95) } else { (0.55, 0.8) };
        let fxs = &mut ui.pk.map_fx[ti];
        fxs.0.set(tb, now, t(220.0, anim::OUT));
        fxs.1.set(ts, now, t(220.0, anim::OUT));
        let (b, s) = (fxs.0.get(now), fxs.1.get(now));
        if a > 0.002 {
            style::shadow(p, fx, screen, r, radius, 12.0, 30.0, -14.0, style::black(0.85 * a));
            p.rect_filled(r, radius, style::rgba(style::CARD_BG.map(|c| (c as f32 * b) as u8), a));
            let tex = path.as_ref().and_then(|pp| {
                if !thumbs.has(pp) {
                    thumbs.want(pp, true);
                }
                thumbs.tex(pp)
            });
            track_img(&mut ui.pk.map_img[ti], tex.as_ref().map(|t| t.1), ui.opened_at);
            if let Some((tex, _)) = tex {
                let ia = img_alpha(ui.pk.map_img[ti], now, 300.0);
                p.add(render::shape(fx, screen, Draw::Image { rect: r, radius, tex, brightness: b, saturate: s, opacity: a * ia }, Mask::default()));
            } else if screen_idx.is_none() {
                // .map-lock:not(:has(img.loaded))::before — linear-gradient(160deg, white .07, transparent 70%)
                p.add(render::shape(fx, screen, Draw::Linear { rect: r, radius, stops: [[0.0, 0.07 * b], [0.7, 0.0]], color: style::white(a) }, Mask::default()));
            }
            let ring_c = if current { style::white(0.85) } else { style::white(0.12) };
            style::ring(p, fx, screen, r, radius, if current { 1.5 } else { 1.0 }, style::fade(ring_c, a));
            // label
            let (lx, lb, fs) = if large { (9.0, 5.0, 13.0) } else { (6.0, 3.0, 11.0) };
            let label_color = style::fade(scale_rgb(style::ink(1.0), b), a);
            match screen_idx {
                Some(i) => {
                    let g = style::text(&ui.snapshot.screens[*i].number.to_string(), fs, Face::Sans600, label_color).galley(p, None);
                    let pos = Pos2::new(r.min.x + lx, r.max.y - lb - fs * 1.45);
                    let sh = style::TextShadow { offset: 1.0, sharp_alpha: 0.9, blur: 4.0, blur_alpha: 0.9, blur_offset: 1.0 };
                    style::draw_galley(p, fx, screen, pos, &g, 1.0, Some(sh));
                }
                None => {
                    let (w, h) = if large { (12.0, 13.0) } else { (10.0, 11.0) };
                    let line = fs * 1.45;
                    let pos = Pos2::new(r.min.x + lx, r.max.y - lb - line + (line - h) / 2.0);
                    lock_icon(p, fx, screen, Rect::from_min_size(pos, Vec2::new(w, h)), label_color, a);
                }
            }
        }
        ui.hits.push((r, hit, a > 0.01));
    }
}

/// The padlock SVG from app.js (viewBox 0 0 10 12), drawn with shapes.
fn lock_icon(p: &Painter, fx: &Arc<Fx>, screen: Rect, r: Rect, color: Color32, a: f32) {
    let sx = r.width() / 10.0;
    let sy = r.height() / 12.0;
    let pt = |x: f32, y: f32| Pos2::new(r.min.x + x * sx, r.min.y + y * sy);
    style::shadow(p, fx, screen, Rect::from_min_max(pt(1.0, 5.0), pt(9.0, 12.0)), 1.0, 1.0, 3.0, 0.0, style::black(0.9 * a));
    // body
    p.rect_filled(Rect::from_min_max(pt(1.0, 5.0), pt(9.0, 12.0)), 1.1 * sx, color);
    // shackle: outer radius 2.5, inner 1.2 → a ring arc of width 1.3 centred at (5, 3.6)
    let c = pt(5.0, 3.6);
    let rr = 1.85 * sx;
    let mut pts = Vec::new();
    for i in 0..=16 {
        let ang = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 16.0;
        pts.push(Pos2::new(c.x + rr * ang.cos(), c.y + rr * ang.sin() * (sy / sx)));
    }
    pts.insert(0, Pos2::new(c.x - rr, pt(0.0, 5.2).y));
    pts.push(Pos2::new(c.x + rr, pt(0.0, 5.2).y));
    p.add(Shape::line(pts, Stroke::new(1.3 * sx, color)));
}

// ---------- apply ----------

fn draw_apply(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, thumbs: &mut Thumbs, screen: Rect, now: f64) {
    let Some(a) = &ui.apply else { return };
    if !a.from.is_positive() {
        return;
    }
    let raw = match a.reverse_at {
        None => ((now - a.start) as f32 / 620.0).clamp(0.0, 1.0),
        Some(r) => (1.0 - (now - r) as f32 / 620.0).clamp(0.0, 1.0),
    };
    let k = anim::MOVE.at(raw);
    let lerp = |x: f32, y: f32| x + (y - x) * k;
    let r = Rect::from_min_max(
        Pos2::new(lerp(a.from.min.x, 0.0), lerp(a.from.min.y, 0.0)),
        Pos2::new(lerp(a.from.max.x, screen.width()), lerp(a.from.max.y, screen.height())),
    );
    let radius = lerp(a.from_radius, 0.0);
    let o = a.fade.get(now);
    if o <= 0.002 {
        return;
    }
    style::shadow(p, fx, screen, r, radius, 50.0, 120.0, -40.0, style::black(0.9 * o));
    let path = a.path.clone();
    let full = ui.full_image.as_ref().filter(|(fp, _)| *fp == path).map(|(_, t)| t.clone());
    let tex = full.or_else(|| thumbs.tex(&path).map(|t| t.0));
    p.rect_filled(r, radius, style::rgba(style::CARD_BG, o));
    if let Some(tex) = tex {
        p.add(render::shape(fx, screen, Draw::Image { rect: r, radius, tex, brightness: 1.0, saturate: 1.0, opacity: o }, Mask::default()));
    }
    p.add(render::shape(fx, screen, Draw::Linear { rect: r, radius, stops: [[0.45, 0.0], [1.0, 0.5]], color: style::black(o) }, Mask::default()));
}

// ---------- edge flash ----------

fn draw_edge(ui: &mut Ui, p: &Painter, fx: &Arc<Fx>, screen: Rect, now: f64) {
    let Some(start) = ui.edge_flash else { return };
    let tt = ((now - start) as f32 / 600.0).clamp(0.0, 1.0);
    if tt >= 1.0 {
        ui.edge_flash = None;
        return;
    }
    let e = anim::OUT;
    let o = if tt < 0.2 {
        e.at(tt / 0.2)
    } else if tt < 0.4 {
        1.0
    } else {
        1.0 - e.at((tt - 0.4) / 0.6)
    };
    p.add(render::shape(fx, screen, Draw::Inset { rect: screen, radius: 0.0, blur: 80.0, spread: -16.0, color: style::white(0.26 * o) }, Mask::default()));
    style::ring(p, fx, screen, screen, 0.0, 1.5, style::white(0.3 * o));
}
