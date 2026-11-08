//! Small custom GPU painter for the things egui can't draw the way the CSS did:
//! rounded images with `brightness()`/`saturate()` filters, blurred box-shadows,
//! inset glows, radial/linear gradients and vertical fade masks.
//! Everything is drawn through egui paint callbacks so it keeps egui's z-order.

use egui::{Color32, Pos2, Rect};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

const SHADER: &str = r#"
struct U {
    screen: vec4<f32>,   // w, h, mode, radius
    quad: vec4<f32>,
    rect: vec4<f32>,
    color: vec4<f32>,
    params: vec4<f32>,   // brightness, saturation, opacity, sigma
    uv: vec4<f32>,
    mask: vec4<f32>,
    stops: vec4<f32>,
    stops2: vec4<f32>,
    clip: vec4<f32>,
    clip_r: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct V { @builtin(position) pos: vec4<f32>, @location(0) px: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> V {
    let corner = vec2<f32>(f32(i & 1u), f32((i >> 1u) & 1u));
    let px = u.quad.xy + corner * u.quad.zw;
    var o: V;
    o.px = px;
    o.pos = vec4<f32>(px.x / u.screen.x * 2.0 - 1.0, 1.0 - px.y / u.screen.y * 2.0, 0.0, 1.0);
    return o;
}

fn sd_round(p: vec2<f32>, half_size: vec2<f32>, r0: f32) -> f32 {
    let r = min(r0, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}
fn gaussian(x: f32, sigma: f32) -> f32 { return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066283 * sigma); }
fn erf2(x0: vec2<f32>) -> vec2<f32> {
    let s = sign(x0);
    let a = abs(x0);
    var x = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    x = x * x;
    return s - s / (x * x);
}
fn shadow_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2<f32>) -> f32 {
    let delta = min(half_size.y - corner - abs(y), 0.0);
    let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    let integral = 0.5 + 0.5 * erf2((x + vec2<f32>(-curved, curved)) * (sqrt(0.5) / sigma));
    return integral.y - integral.x;
}
fn shadow(p: vec2<f32>, half_size: vec2<f32>, corner0: f32, sigma0: f32) -> f32 {
    let sigma = max(sigma0, 0.01);
    let corner = min(corner0, min(half_size.x, half_size.y));
    let low = p.y - half_size.y;
    let high = p.y + half_size.y;
    let start = clamp(-3.0 * sigma, low, high);
    let end = clamp(3.0 * sigma, low, high);
    let step = (end - start) / 4.0;
    var y = start + step * 0.5;
    var value = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        value = value + shadow_x(p.x, p.y - y, sigma, corner, half_size) * gaussian(y, sigma) * step;
        y = y + step;
    }
    return value;
}
fn mask_at(y: f32) -> f32 {
    var m = 1.0;
    if (u.mask.y > u.mask.x) { m = m * clamp((y - u.mask.x) / (u.mask.y - u.mask.x), 0.0, 1.0); }
    if (u.mask.w > u.mask.z) { m = m * clamp((u.mask.w - y) / (u.mask.w - u.mask.z), 0.0, 1.0); }
    return m;
}

@fragment
fn fs(v: V) -> @location(0) vec4<f32> {
    let p = v.px;
    let mode = i32(u.screen.z + 0.5);
    let radius = u.screen.w;
    let half_size = u.rect.zw * 0.5;
    let centre = u.rect.xy + half_size;
    var coverage = 1.0;
    if (u.clip.z > 0.0) {
        let dc = sd_round(p - (u.clip.xy + u.clip.zw * 0.5), u.clip.zw * 0.5, u.clip_r.x);
        coverage = coverage * clamp(0.5 - dc, 0.0, 1.0);
    }
    coverage = coverage * mask_at(p.y);
    // sample unconditionally (uniform control flow)
    let t = clamp((p - u.rect.xy) / max(u.rect.zw, vec2<f32>(1.0)), vec2<f32>(0.0), vec2<f32>(1.0));
    let texel = textureSample(tex, samp, mix(u.uv.xy, u.uv.zw, t));
    var outc = vec4<f32>(0.0);
    if (mode == 0) {
        let d = sd_round(p - centre, half_size, radius);
        let a = clamp(0.5 - d, 0.0, 1.0);
        var c = texel.rgb * u.params.x;
        let s = u.params.y;
        let sat = mat3x3<f32>(
            vec3<f32>(0.213 + 0.787 * s, 0.213 - 0.213 * s, 0.213 - 0.213 * s),
            vec3<f32>(0.715 - 0.715 * s, 0.715 + 0.285 * s, 0.715 - 0.715 * s),
            vec3<f32>(0.072 - 0.072 * s, 0.072 - 0.072 * s, 0.072 + 0.928 * s));
        c = clamp(sat * c, vec3<f32>(0.0), vec3<f32>(1.0));
        let alpha = a * u.params.z;
        outc = vec4<f32>(c * alpha, alpha);
    } else if (mode == 1) {
        outc = u.color * shadow(p - centre, half_size, radius, u.params.w);
    } else if (mode == 2) {
        let d = sd_round(p - centre, half_size, radius);
        let inside = clamp(0.5 - d, 0.0, 1.0);
        let spread = u.params.x;
        let inner = max(half_size - vec2<f32>(spread), vec2<f32>(0.0));
        var s = 0.0;
        if (u.params.w > 0.01) {
            s = shadow(p - centre, inner, max(radius - spread, 0.0), u.params.w);
        } else {
            s = clamp(0.5 - sd_round(p - centre, inner, max(radius - spread, 0.0)), 0.0, 1.0);
        }
        outc = u.color * ((1.0 - s) * inside);
    } else if (mode == 3) {
        let q = (p - u.stops2.zw) / u.rect.zw;
        let tt = length(q);
        var a = 0.0;
        if (tt <= u.stops.x) { a = u.stops.y; }
        else if (tt <= u.stops.z) { a = mix(u.stops.y, u.stops.w, (tt - u.stops.x) / max(u.stops.z - u.stops.x, 1e-5)); }
        else if (tt <= u.stops2.x) { a = mix(u.stops.w, u.stops2.y, (tt - u.stops.z) / max(u.stops2.x - u.stops.z, 1e-5)); }
        else { a = u.stops2.y; }
        outc = u.color * a;
    } else if (mode == 4) {
        let d = sd_round(p - centre, half_size, radius);
        let inside = clamp(0.5 - d, 0.0, 1.0);
        let ty = (p.y - u.rect.y) / u.rect.w;
        let a = mix(u.stops.y, u.stops.w, clamp((ty - u.stops.x) / max(u.stops.z - u.stops.x, 1e-5), 0.0, 1.0));
        outc = u.color * (a * inside);
    } else {
        let d = sd_round(p - centre, half_size, radius);
        let outer = clamp(0.5 - d, 0.0, 1.0);
        let inner = clamp(0.5 - (d + u.params.x), 0.0, 1.0);
        outc = u.color * (outer - inner);
    }
    return outc * coverage;
}
"#;

const SLOT: u64 = 256;
const SLOTS: u64 = 4096;

pub struct Fx {
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    ubind: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    dummy: Arc<wgpu::BindGroup>,
    next: AtomicU32,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Fx {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let ulayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-u"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(176),
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-t"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx"),
            bind_group_layouts: &[Some(&ulayout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-u"),
            size: SLOT * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ubind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-u"),
            layout: &ulayout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &ubuf, offset: 0, size: wgpu::BufferSize::new(176) }),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fx"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let dummy = make_tex(device, queue, &tex_layout, &sampler, &[(1, 1, vec![0, 0, 0, 0])]).expect("dummy texture").bind;
        Self { pipeline, ubuf, ubind, tex_layout, sampler, dummy, next: AtomicU32::new(0), device: device.clone(), queue: queue.clone() }
    }

    /// Call before building a frame's shapes.
    pub fn begin_frame(&self) {
        self.next.store(0, Ordering::Relaxed);
    }

    /// Uploads a thumbnail with its mip chain (largest first).
    pub fn upload(&self, levels: &[(u32, u32, Vec<u8>)]) -> Option<Tex> {
        make_tex(&self.device, &self.queue, &self.tex_layout, &self.sampler, levels)
    }
}

fn make_tex(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    levels: &[(u32, u32, Vec<u8>)],
) -> Option<Tex> {
    let (w, h, _) = levels.first()?;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("thumb"),
        size: wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (lw, lh, px)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * lw), rows_per_image: Some(*lh) },
            wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("thumb"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    Some(Tex { bind: Arc::new(bind), w: *w, h: *h, texture: Arc::new(texture) })
}

/// A GPU texture holding one thumbnail. Dropping the last clone frees it.
#[derive(Clone, Debug)]
pub struct Tex {
    pub bind: Arc<wgpu::BindGroup>,
    pub w: u32,
    pub h: u32,
    texture: Arc<wgpu::Texture>,
}

impl Tex {
    pub fn destroy(&self) {
        self.texture.destroy();
    }

    /// GPU memory used, including the mip chain.
    pub fn bytes(&self) -> u64 {
        let (mut w, mut h, mut total) = (self.w as u64, self.h as u64, 0);
        for _ in 0..self.texture.mip_level_count() {
            total += w * h * 4;
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
        }
        total
    }
}

/// Builds a mip chain on the CPU (wgpu has no glGenerateMipmap). Each level is a 2×2 box
/// average of the one above, like GPU mip generation; about 4× faster than a triangle resize.
pub fn mip_chain(img: image::RgbaImage) -> Vec<(u32, u32, Vec<u8>)> {
    let (mut w, mut h) = img.dimensions();
    let mut levels = vec![(w, h, img.into_raw())];
    while w > 1 || h > 1 {
        let next = half(&levels[levels.len() - 1].2, w as usize, h as usize);
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
        levels.push((w, h, next));
    }
    levels
}

/// 2×2 box downsample of an RGBA image; an odd last row/column is folded into its neighbour.
fn half(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let r0 = &src[(2 * y).min(h - 1) * w * 4..][..w * 4];
        let r1 = &src[(2 * y + 1).min(h - 1) * w * 4..][..w * 4];
        let row = &mut out[y * nw * 4..][..nw * 4];
        for x in 0..nw {
            let (a, b) = ((2 * x).min(w - 1) * 4, (2 * x + 1).min(w - 1) * 4);
            for c in 0..4 {
                let sum = r0[a + c] as u16 + r0[b + c] as u16 + r1[a + c] as u16 + r1[b + c] as u16;
                row[x * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_halves_down_to_one_pixel() {
        let img = image::RgbaImage::from_fn(5, 3, |x, y| image::Rgba([(x * 50) as u8, (y * 100) as u8, 7, 255]));
        let levels = mip_chain(img);
        let sizes: Vec<_> = levels.iter().map(|(w, h, px)| (*w, *h, px.len())).collect();
        assert_eq!(sizes, vec![(5, 3, 60), (2, 1, 8), (1, 1, 4)]);
        // first pixel of level 1 averages (0,0) (1,0) (0,1) (1,1)
        assert_eq!(&levels[1].2[..4], &[25, 50, 7, 255]);
    }
}

/// One custom draw. Coordinates are in egui points; converted to pixels at paint time.
#[derive(Clone, Debug)]
pub enum Draw {
    Image { rect: Rect, radius: f32, tex: Tex, brightness: f32, saturate: f32, opacity: f32 },
    Shadow { rect: Rect, radius: f32, blur: f32, spread: f32, offset: [f32; 2], color: Color32 },
    Inset { rect: Rect, radius: f32, blur: f32, spread: f32, color: Color32 },
    Ring { rect: Rect, radius: f32, width: f32, color: Color32 },
    Radial { area: Rect, centre: Pos2, radii: [f32; 2], stops: [[f32; 2]; 3], color: Color32 },
    Linear { rect: Rect, radius: f32, stops: [[f32; 2]; 2], color: Color32 },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Mask {
    /// Fade in between these two y values (points).
    pub fade_in: Option<[f32; 2]>,
    /// Fade out between these two y values (points).
    pub fade_out: Option<[f32; 2]>,
    /// Extra rounded clip (rect, radius).
    pub clip: Option<(Rect, f32)>,
}

struct Cb {
    fx: Arc<Fx>,
    draw: Draw,
    mask: Mask,
    slot: u32,
}

pub fn shape(fx: &Arc<Fx>, screen: Rect, draw: Draw, mask: Mask) -> egui::Shape {
    let slot = fx.next.fetch_add(1, Ordering::Relaxed);
    if slot as u64 >= SLOTS {
        return egui::Shape::Noop;
    }
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(screen, Cb { fx: fx.clone(), draw, mask, slot }))
}

fn premul(c: Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_array();
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
}

impl egui_wgpu::CallbackTrait for Cb {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        sd: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let u = uniforms(sd.size_in_pixels, sd.pixels_per_point, &self.draw, &self.mask);
        let bytes: Vec<u8> = u.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect();
        queue.write_buffer(&self.fx.ubuf, self.slot as u64 * SLOT, &bytes);
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _res: &egui_wgpu::CallbackResources) {
        pass.set_pipeline(&self.fx.pipeline);
        pass.set_bind_group(0, &self.fx.ubind, &[(self.slot as u64 * SLOT) as u32]);
        match &self.draw {
            Draw::Image { tex, .. } => pass.set_bind_group(1, &*tex.bind, &[]),
            _ => pass.set_bind_group(1, &*self.fx.dummy, &[]),
        }
        pass.draw(0..4, 0..1);
    }
}

fn uniforms(screen_px: [u32; 2], ppp: f32, draw: &Draw, mask: &Mask) -> [[f32; 4]; 11] {
    let px = |r: Rect| [r.min.x * ppp, r.min.y * ppp, r.width() * ppp, r.height() * ppp];
    let mut u = [[0.0f32; 4]; 11];
    // 0 screen, 1 quad, 2 rect, 3 color, 4 params, 5 uv, 6 mask, 7 stops, 8 stops2, 9 clip, 10 clip_r
    let [m0, m1] = mask.fade_in.map(|[a, b]| [a * ppp, b * ppp]).unwrap_or([0.0, 0.0]);
    let [m2, m3] = mask.fade_out.map(|[a, b]| [a * ppp, b * ppp]).unwrap_or([0.0, 0.0]);
    u[6] = [m0, m1, m2, m3];
    if let Some((r, rad)) = mask.clip {
        u[9] = px(r);
        u[10] = [rad * ppp, 0.0, 0.0, 0.0];
    }
    u[5] = [0.0, 0.0, 1.0, 1.0];
    let (mode, radius, rect, quad) = match draw {
        Draw::Image { rect, radius, tex, brightness, saturate, opacity } => {
            let (rw, rh) = (rect.width().max(1.0), rect.height().max(1.0));
            let (tw, th) = (tex.w as f32, tex.h as f32);
            let (mut u0, mut v0, mut u1, mut v1) = (0.0, 0.0, 1.0, 1.0);
            if tw / th > rw / rh {
                let keep = (rw / rh) / (tw / th);
                u0 = (1.0 - keep) / 2.0;
                u1 = 1.0 - u0;
            } else {
                let keep = (tw / th) / (rw / rh);
                v0 = (1.0 - keep) / 2.0;
                v1 = 1.0 - v0;
            }
            u[5] = [u0, v0, u1, v1];
            u[4] = [*brightness, *saturate, *opacity, 0.0];
            (0.0, radius * ppp, px(*rect), px(rect.expand(1.0)))
        }
        Draw::Shadow { rect, radius, blur, spread, offset, color } => {
            let r = rect.expand(*spread).translate(egui::vec2(offset[0], offset[1]));
            u[3] = premul(*color);
            u[4] = [0.0, 0.0, 0.0, (blur / 2.0 * ppp).max(0.01)];
            (1.0, (radius + spread).max(0.0) * ppp, px(r), px(r.expand(blur * 1.6 + 2.0)))
        }
        Draw::Inset { rect, radius, blur, spread, color } => {
            u[3] = premul(*color);
            u[4] = [spread * ppp, 0.0, 0.0, blur / 2.0 * ppp];
            (2.0, radius * ppp, px(*rect), px(*rect))
        }
        Draw::Ring { rect, radius, width, color } => {
            u[3] = premul(*color);
            u[4] = [width * ppp, 0.0, 0.0, 0.0];
            (5.0, radius * ppp, px(*rect), px(*rect))
        }
        Draw::Radial { area, centre, radii, stops, color } => {
            u[3] = premul(*color);
            u[7] = [stops[0][0], stops[0][1], stops[1][0], stops[1][1]];
            u[8] = [stops[2][0], stops[2][1], centre.x * ppp, centre.y * ppp];
            (3.0, 0.0, [0.0, 0.0, radii[0] * ppp, radii[1] * ppp], px(*area))
        }
        Draw::Linear { rect, radius, stops, color } => {
            u[3] = premul(*color);
            u[7] = [stops[0][0], stops[0][1], stops[1][0], stops[1][1]];
            (4.0, radius * ppp, px(*rect), px(*rect))
        }
    };
    u[0] = [screen_px[0] as f32, screen_px[1] as f32, mode, radius];
    u[1] = quad;
    u[2] = rect;
    u
}
