//! Индикатор у курсора — нативное Win32-окно (layered, topmost, click-through).
//!
//! Создаётся один раз в своём потоке и просто двигается за курсором, пульсируя по уровню звука.
//! Не зависит от eframe/GL — надёжно работает, даже когда основное окно в фоне.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, Ellipse, FillRect, GetDC, GetStockObject, ReleaseDC,
    SelectObject, HBRUSH, HGDIOBJ, NULL_PEN,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    PeekMessageW, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
    TranslateMessage, HMENU, HWND_TOPMOST, LWA_COLORKEY, MSG, PM_REMOVE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SW_HIDE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const SIZE: i32 = 60;
/// Цвет-ключ прозрачности (magenta) — заведомо не встречается в индикаторе.
const KEY: u32 = 0x00FF_00FF;

struct State {
    visible: AtomicBool,
    level_bits: AtomicU32,
}

pub struct Overlay {
    state: Arc<State>,
    stop: Arc<AtomicBool>,
}

impl Overlay {
    pub fn spawn() -> Self {
        let state = Arc::new(State {
            visible: AtomicBool::new(false),
            level_bits: AtomicU32::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let st = Arc::clone(&state);
        let sp = Arc::clone(&stop);
        thread::Builder::new()
            .name("tvoice-overlay".into())
            .spawn(move || unsafe { run(st, sp) })
            .expect("overlay thread");
        Self { state, stop }
    }

    pub fn set_visible(&self, v: bool) {
        self.state.visible.store(v, Ordering::Relaxed);
    }
    pub fn set_level(&self, level: f32) {
        self.state.level_bits.store(level.to_bits(), Ordering::Relaxed);
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe fn run(state: Arc<State>, stop: Arc<AtomicBool>) {
    let hinst = match GetModuleHandleW(None) {
        Ok(h) => h,
        Err(_) => return,
    };
    let class_name = windows::core::w!("TvoiceOverlay");

    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(hinst.0),
        lpszClassName: class_name,
        ..Default::default()
    };
    RegisterClassW(&wc);

    let ex_style = WS_EX_LAYERED
        | WS_EX_TRANSPARENT
        | WS_EX_TOPMOST
        | WS_EX_TOOLWINDOW
        | WS_EX_NOACTIVATE;

    let hwnd = match CreateWindowExW(
        ex_style,
        class_name,
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        SIZE,
        SIZE,
        HWND::default(),
        HMENU::default(),
        HINSTANCE(hinst.0),
        None,
    ) {
        Ok(h) => h,
        Err(_) => return,
    };

    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(KEY), 0, LWA_COLORKEY);

    // GDI-объекты создаём ОДИН РАЗ (без churn create/delete в цикле — это и корёжило кучу).
    let key_brush = CreateSolidBrush(COLORREF(KEY));
    let brushes = [
        CreateSolidBrush(rgb(0x37, 0xD6, 0x7A)),
        CreateSolidBrush(rgb(0xE7, 0xB4, 0x16)),
        CreateSolidBrush(rgb(0xE5, 0x5A, 0x5A)),
    ];
    let null_pen = GetStockObject(NULL_PEN);

    let mut shown = false;
    while !stop.load(Ordering::Relaxed) {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if state.visible.load(Ordering::Relaxed) {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    pt.x + 18,
                    pt.y - SIZE - 6,
                    SIZE,
                    SIZE,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                shown = true;
                let level = f32::from_bits(state.level_bits.load(Ordering::Relaxed));
                paint(hwnd, level, key_brush, &brushes, null_pen);
            }
        } else if shown {
            let _ = ShowWindow(hwnd, SW_HIDE);
            shown = false;
        }

        thread::sleep(Duration::from_millis(33));
    }

    let _ = DeleteObject(HGDIOBJ(key_brush.0));
    for b in brushes {
        let _ = DeleteObject(HGDIOBJ(b.0));
    }
    let _ = DestroyWindow(hwnd);
}

unsafe fn paint(hwnd: HWND, level: f32, key_brush: HBRUSH, brushes: &[HBRUSH; 3], null_pen: HGDIOBJ) {
    let hdc = GetDC(hwnd);
    if hdc.is_invalid() {
        return;
    }
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    // Фон — цвет-ключ (станет прозрачным).
    FillRect(hdc, &rc, key_brush);

    let lvl = level.clamp(0.0, 1.0);
    let idx = if lvl < 0.6 {
        0
    } else if lvl < 0.85 {
        1
    } else {
        2
    };
    let old_brush = SelectObject(hdc, HGDIOBJ(brushes[idx].0));
    let old_pen = SelectObject(hdc, null_pen);

    let cx = rc.right / 2;
    let cy = rc.bottom / 2;
    let r = 8 + (lvl * 18.0) as i32;
    let _ = Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);

    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    ReleaseDC(hwnd, hdc);
}
