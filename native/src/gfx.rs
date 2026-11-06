//! The transparent overlay window: winit + wgpu (Direct3D 12 through a DirectComposition
//! swapchain, the same route WebView2 takes for transparency) + egui.
//! It only exists while the switcher is open; dropping it frees the GPU device and all textures.

use crate::render::Fx;
use std::sync::Arc;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::HasWindowHandle as _;
use winit::window::{Window, WindowAttributes, WindowLevel};

/// Device, queue and compiled pipelines. Slow to make (~300 ms), so it's started on a
/// thread the moment the overlay process launches, in parallel with everything else.
pub struct GpuCore {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    fx: Arc<Fx>,
    format: wgpu::TextureFormat,
}

fn renderer_options() -> egui_wgpu::RendererOptions {
    egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, predictable_texture_filtering: false }
}

impl GpuCore {
    pub fn create() -> Result<Self, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::DX12;
        desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let instance = wgpu::Instance::new(desc);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("No graphics adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("wallswitch"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(|e| format!("No graphics device: {e}"))?;
        // DXGI composition swapchains always accept BGRA8; the two shader compiles run in parallel.
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let (renderer, fx) = std::thread::scope(|s| {
            let fx = s.spawn(|| Arc::new(Fx::new(&device, &queue, format)));
            let renderer = egui_wgpu::Renderer::new(&device, format, renderer_options());
            (renderer, fx.join().expect("fx pipeline"))
        });
        Ok(Self { instance, adapter, device, queue, renderer, fx, format })
    }

    /// Starts on a thread; `done` is called when it finishes (to wake the event loop).
    pub fn start(done: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<Result<Self, String>> {
        std::thread::spawn(move || {
            let r = Self::create();
            done();
            r
        })
    }
}

pub struct GpuWindow {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: egui_wgpu::Renderer,
    pub ctx: egui::Context,
    state: egui_winit::State,
    pub fx: Arc<Fx>,
}

impl GpuWindow {
    /// The bare window. With no redirection bitmap and no swapchain yet it is fully invisible,
    /// so it can be shown right away while the GPU is still initialising.
    pub fn create_window(event_loop: &ActiveEventLoop, rect: [i32; 4]) -> Result<Arc<Window>, String> {
        #[cfg(windows)]
        use winit::platform::windows::WindowAttributesExtWindows as _;
        let size = PhysicalSize::new((rect[2] - rect[0]).max(1) as u32, (rect[3] - rect[1]).max(1) as u32);
        let attrs = WindowAttributes::default()
            .with_title("Wallpaper Switcher")
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(rect[0], rect[1]))
            .with_inner_size(size)
            .with_visible(false);
        #[cfg(windows)]
        let attrs = attrs
            .with_skip_taskbar(true)
            .with_undecorated_shadow(false)
            // DirectComposition gives per-pixel alpha. Don't set `transparent` or a system
            // backdrop here: on Windows 11 both make DWM blur everything behind the window.
            .with_no_redirection_bitmap(true);
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| format!("Couldn't create the window: {e}"))?);
        #[cfg(windows)]
        plain_popup(&window);
        Ok(window)
    }

    pub fn new(event_loop: &ActiveEventLoop, window: Arc<Window>, core: GpuCore) -> Result<Self, String> {
        let size = window.inner_size();

        let GpuCore { instance, adapter, device, queue, mut renderer, mut fx, format: boot_format } = core;
        let surface = instance.create_surface(window.clone()).map_err(|e| format!("Couldn't create a surface: {e}"))?;
        let caps = surface.get_capabilities(&adapter);
        crate::log(format!("adapter {:?} formats {:?} alpha {:?}", adapter.get_info().name, caps.formats, caps.alpha_modes));
        let format = if caps.formats.contains(&boot_format) { boot_format } else { caps.formats[0] };
        if format != boot_format {
            // rare: rebuild the pipelines for the surface's format
            renderer = egui_wgpu::Renderer::new(&device, format, renderer_options());
            fx = Arc::new(Fx::new(&device, &queue, format));
        }
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            crate::log(format!("no premultiplied alpha: {:?}", caps.alpha_modes));
            caps.alpha_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        Ok(Self { window, surface, device, queue, config, renderer, ctx, state, fx })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        // Windows sends several same-size WM_SIZEs on show; rebuilding the swapchain costs ~30 ms each
        let same = size.width == self.config.width && size.height == self.config.height;
        if size.width > 0 && size.height > 0 && !same {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn on_window_event(&mut self, event: &winit::event::WindowEvent) -> egui_winit::EventResponse {
        self.state.on_window_event(&self.window, event)
    }

    /// Runs the UI and presents one frame.
    pub fn paint(&mut self, run_ui: impl FnMut(&mut egui::Ui)) {
        let raw = self.state.take_egui_input(&self.window);
        self.fx.begin_frame();
        let mut out = self.ctx.run_ui(raw, run_ui);
        self.state.handle_platform_output(&self.window, out.platform_output);
        let jobs = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: out.pixels_per_point };

        // textures first, so a skipped frame never loses font atlas updates
        for (id, deltas) in &out.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let free = std::mem::take(&mut out.textures_delta.free);
        out.textures_delta.clear();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => {
                crate::log(format!("surface: {other:?}"));
                self.surface.configure(&self.device, &self.config);
                for id in &free {
                    self.renderer.free_texture(id);
                }
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
        let mut cmds = self.renderer.update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &sd);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &sd);
        }
        cmds.push(encoder.finish());
        self.queue.submit(cmds);
        self.queue.present(frame);
        for id in &free {
            self.renderer.free_texture(id);
        }
    }

    #[cfg(windows)]
    pub fn hwnd(&self) -> isize {
        use winit::raw_window_handle::RawWindowHandle;
        match self.window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => 0,
        }
    }
}

/// winit gives borderless windows WS_CAPTION | WS_SYSMENU (and re-applies them on every state
/// change). Theming tools such as Windhawk's "Translucent Windows" treat that as a normal app
/// window and give it Mica/acrylic plus an extended frame, which turns the overlay into frosted
/// glass with ghost caption buttons. Keep the window a plain popup and undo anything applied.
#[cfg(windows)]
fn plain_popup(window: &Window) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmEnableBlurBehindWindow, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE, DWM_BB_ENABLE,
        DWM_BLURBEHIND,
    };
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::Shell::SetWindowSubclass;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER,
    };
    use winit::raw_window_handle::RawWindowHandle;
    let Ok(RawWindowHandle::Win32(h)) = window.window_handle().map(|h| h.as_raw()) else { return };
    let hwnd = HWND(h.hwnd.get() as _);
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(popup_proc), 0x5753, 0);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        SetWindowLongPtrW(hwnd, GWL_STYLE, popup_bits(style) as isize);
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        // now ineligible, so these reach DWM unmodified
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS::default());
        let auto: u32 = 0; // DWMSBT_AUTO: no backdrop on a window without a title bar
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &auto as *const _ as _, 4);
        // no system open/close animation: the overlay animates itself, like the web version did
        let on: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED, &on as *const _ as _, 4);
        let bb = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE, fEnable: false.into(), ..Default::default() };
        let _ = DwmEnableBlurBehindWindow(hwnd, &bb);
        disable_accent(hwnd);
    }
}

/// SetWindowCompositionAttribute(WCA_ACCENT_POLICY, ACCENT_DISABLED) — undocumented but stable.
#[cfg(windows)]
unsafe fn disable_accent(hwnd: windows::Win32::Foundation::HWND) {
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    #[repr(C)]
    struct Accent {
        state: u32,
        flags: u32,
        color: u32,
        animation: u32,
    }
    #[repr(C)]
    struct Data {
        attrib: u32,
        data: *mut core::ffi::c_void,
        size: usize,
    }
    type SetWca = unsafe extern "system" fn(windows::Win32::Foundation::HWND, *mut Data) -> i32;
    unsafe {
        let Ok(user32) = GetModuleHandleW(windows::core::w!("user32.dll")) else { return };
        let Some(f) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let f: SetWca = std::mem::transmute(f);
        let mut accent = Accent { state: 0, flags: 0, color: 0, animation: 0 };
        let mut data = Data { attrib: 19, data: &mut accent as *mut _ as _, size: std::mem::size_of::<Accent>() };
        f(hwnd, &mut data);
    }
}

#[cfg(windows)]
fn popup_bits(style: u32) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME};
    let strip = (WS_CAPTION | WS_BORDER | WS_DLGFRAME | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0;
    (style & !strip) | WS_POPUP.0
}

#[cfg(windows)]
unsafe extern "system" fn popup_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::{GWL_STYLE, STYLESTRUCT, WM_STYLECHANGING};
    if msg == WM_STYLECHANGING && wparam.0 as i32 == GWL_STYLE.0 {
        let ss = lparam.0 as *mut STYLESTRUCT;
        if !ss.is_null() {
            unsafe { (*ss).styleNew = popup_bits((*ss).styleNew) };
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
