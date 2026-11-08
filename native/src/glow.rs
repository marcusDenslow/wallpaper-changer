pub fn flash(screen: [i32; 4], current: impl Fn() -> bool + Send + 'static) {
    imp::flash(screen, current)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Once;
    use std::time::{Duration, Instant};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA,
        AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW, ShowWindow,
        TranslateMessage, UpdateLayeredWindow, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    const CLASS: PCWSTR = w!("WallpaperSwitcherEdge");
    const LENGTH_MS: f32 = 600.0;

    unsafe extern "system" fn edge_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    fn blend(alpha: u8) -> BLENDFUNCTION {
        BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        }
    }

    fn level(t: f32) -> f32 {
        if t < 0.2 {
            1.0 - (1.0 - t / 0.2).powi(3)
        } else if t < 0.4 {
            1.0
        } else {
            let x = (t - 0.4) / 0.6;
            1.0 - x * x * (3.0 - 2.0 * x)
        }
    }

    fn profile(thick: usize, line: f32) -> Vec<u32> {
        (0..thick)
            .map(|d| {
                let d = d as f32;
                let soft = 0.26 * (1.0 - d / thick as f32).powf(2.4);
                let core = (line + 0.5 - d).clamp(0.0, 1.0) * 0.32;
                let a = ((core + soft * (1.0 - core)) * 255.0).round() as u32;
                (a << 24) | (a << 16) | (a << 8) | a
            })
            .collect()
    }

    unsafe fn pump() {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    struct Strip {
        hwnd: HWND,
        memory: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
        at: POINT,
        size: SIZE,
    }

    impl Strip {
        unsafe fn show(&self, alpha: u8) -> bool {
            let origin = POINT::default();
            let step = blend(alpha);
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&self.at as *const _),
                Some(&self.size as *const _),
                Some(self.memory),
                Some(&origin as *const _),
                COLORREF(0),
                Some(&step as *const _),
                ULW_ALPHA,
            )
            .is_ok()
        }

        unsafe fn close(self) {
            let _ = DestroyWindow(self.hwnd);
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
        }
    }

    unsafe fn strip(instance: HINSTANCE, screen: [i32; 4], part: [i32; 4], table: &[u32]) -> Option<Strip> {
        let [left, top, right, bottom] = part;
        let (width, height) = (right - left, bottom - top);
        if width <= 0 || height <= 0 {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let screen_dc = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen_dc));
        ReleaseDC(None, screen_dc);
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => bitmap,
            _ => {
                let _ = DeleteDC(memory);
                return None;
            }
        };
        let previous = SelectObject(memory, bitmap.into());
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize);
        let [screen_left, screen_top, screen_right, screen_bottom] = screen;
        for (y, row) in pixels.chunks_mut(width as usize).enumerate() {
            let at_y = top + y as i32;
            let from_y = (at_y - screen_top).min(screen_bottom - 1 - at_y);
            for (x, pixel) in row.iter_mut().enumerate() {
                let at_x = left + x as i32;
                let edge = (at_x - screen_left).min(screen_right - 1 - at_x).min(from_y).max(0) as usize;
                *pixel = table.get(edge).copied().unwrap_or(0);
            }
        }

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!(""),
            WS_POPUP,
            left,
            top,
            width,
            height,
            None,
            None,
            Some(instance),
            None,
        );
        let strip = Strip {
            hwnd: hwnd.unwrap_or_default(),
            memory,
            bitmap,
            previous,
            at: POINT { x: left, y: top },
            size: SIZE { cx: width, cy: height },
        };
        if strip.hwnd.is_invalid() || !strip.show(0) {
            strip.close();
            return None;
        }
        let _ = ShowWindow(strip.hwnd, SW_SHOWNOACTIVATE);
        Some(strip)
    }

    pub fn flash(screen: [i32; 4], current: impl Fn() -> bool + Send + 'static) {
        std::thread::spawn(move || unsafe {
            let Ok(module) = GetModuleHandleW(None) else { return };
            let instance = HINSTANCE::from(module);
            static REGISTER: Once = Once::new();
            REGISTER.call_once(|| {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(edge_proc),
                    hInstance: instance,
                    lpszClassName: CLASS,
                    ..Default::default()
                };
                RegisterClassW(&class);
            });

            let [left, top, right, bottom] = screen;
            let scale = ((bottom - top) as f32 / 1080.0).max(1.0);
            let thick = ((84.0 * scale).round() as i32).min((right - left) / 3).min((bottom - top) / 3).max(8);
            let table = profile(thick as usize, 1.5 * scale);
            let parts = [
                [left, top, right, top + thick],
                [left, bottom - thick, right, bottom],
                [left, top + thick, left + thick, bottom - thick],
                [right - thick, top + thick, right, bottom - thick],
            ];
            let strips: Vec<Strip> = parts.iter().filter_map(|&part| strip(instance, screen, part, &table)).collect();

            let start = Instant::now();
            loop {
                pump();
                let elapsed = start.elapsed().as_secs_f32() * 1000.0;
                if elapsed >= LENGTH_MS || !current() {
                    break;
                }
                let alpha = (level(elapsed / LENGTH_MS) * 255.0).round() as u8;
                for strip in &strips {
                    strip.show(alpha);
                }
                std::thread::sleep(Duration::from_millis(12));
            }
            for strip in strips {
                strip.close();
            }
            pump();
        });
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn flash(_screen: [i32; 4], _current: impl Fn() -> bool + Send + 'static) {}
}
