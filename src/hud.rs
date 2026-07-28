//! Рисование индикатора диктовки: подложка-пилюля и три точки, вытягивающиеся под голос.
//!
//! Кадр собирается в 32-битный буфер с предумноженной альфой — именно в таком виде его
//! ждёт `UpdateLayeredWindow`. Формы заливаются по покрытию пикселя, поэтому края
//! выходят сглаженными без всякой графической библиотеки.

use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, UpdateLayeredWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, ULW_ALPHA,
};

/// Размер окна индикатора (с запасом под свечение).
pub const W: i32 = 44;
pub const H: i32 = 32;
/// Пилюля-подложка внутри окна.
const PAD: f32 = 5.0;
/// Точки: ширина постоянна, растут только вверх-вниз — из круга в капсулу.
const DOTS: usize = 3;
const DOT_GAP: f32 = 8.0;
const DOT_R: f32 = 2.6;
const DOT_GROW: f32 = 5.0;

/// Буфер 32-бит BGRA с предумноженной альфой — формат, который ждёт `UpdateLayeredWindow`.
pub struct Canvas {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
}

impl Canvas {
    pub unsafe fn new() -> Option<Self> {
        let screen = GetDC(HWND::default());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(HWND::default(), screen);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: W,
                biHeight: -H, // сверху вниз
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &mut info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            return None;
        }
        let old = SelectObject(dc, HGDIOBJ(bmp.0));
        Some(Self {
            dc,
            bmp,
            old,
            bits: bits as *mut u32,
        })
    }

    fn pixels(&self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (W * H) as usize) }
    }

    /// Отрисовать кадр: подложка-пилюля и три точки, дышащие под уровень звука.
    pub fn draw(&self, t: f32, level: f32) {
        draw_frame(self.pixels(), t, level);
    }

    /// Отдать буфер окну с попиксельной прозрачностью.
    pub unsafe fn present(&self, hwnd: HWND, pos: (i32, i32)) {
        let (x, y) = clamp_to_screen(pos);
        let dst = POINT { x, y };
        let src = POINT { x: 0, y: 0 };
        let size = SIZE { cx: W, cy: H };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let screen = GetDC(HWND::default());
        let _ = UpdateLayeredWindow(
            hwnd,
            screen,
            Some(&dst),
            Some(&size),
            self.dc,
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        ReleaseDC(HWND::default(), screen);
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Кадр индикатора: подложка-пилюля и три точки, вытягивающиеся под голос.
/// `voice` — уже приведённая к 0…1 громкость (см. `voice_amount`).
/// Вынесено из `Canvas`, чтобы тот же рисунок можно было отрисовать в файл для проверки.
pub fn draw_frame(px: &mut [u32], t: f32, voice: f32) {
    px.fill(0);
    let cx = W as f32 / 2.0;
    let cy = H as f32 / 2.0;
    // Подложка: тёмная полупрозрачная пилюля со скруглением по высоте.
    let half = (W as f32 / 2.0 - PAD, H as f32 / 2.0 - PAD);
    rounded_rect(px, cx, cy, half, half.1, (14, 16, 15), 0.62);
    rounded_rect(px, cx, cy, (half.0 - 0.6, half.1 - 0.6), half.1, (255, 255, 255), 0.05);

    let voice = voice.clamp(0.0, 1.0);
    // Точки прыгают вместе с голосом — каждая со своей долей, чтобы не выглядеть одним
    // блоком. Прежняя медленная синусоида поверх громкости делала движение «плавающим».
    const SHARE: [f32; DOTS] = [0.82, 1.0, 0.9];
    for i in 0..DOTS {
        // Хаос появляется только с голосом: в тишине точкам дёргаться незачем.
        // У каждой свой сдвиг по шуму, поэтому они не повторяют друг друга.
        let chaos = (noise(t * 7.0 + i as f32 * 31.7) - 0.5) * 0.55 * voice;
        let idle = 0.025 * (0.5 + 0.5 * (t * 1.6 - i as f32 * 0.8).sin());
        let amp = (idle + voice * SHARE[i] * (1.0 + chaos)).clamp(0.0, 1.0);
        // Ширина постоянна, тянется только высота: круг превращается в капсулу.
        let half_h = DOT_R + DOT_GROW * amp;
        let x = cx + (i as f32 - (DOTS as f32 - 1.0) / 2.0) * DOT_GAP;
        // Ореол повторяет форму — тоже вытягивается вверх-вниз.
        rounded_rect(
            px,
            x,
            cy,
            (DOT_R * 1.9, half_h * 1.25),
            DOT_R * 1.9,
            (157, 122, 255),
            0.13 * amp,
        );
        rounded_rect(
            px,
            x,
            cy,
            (DOT_R, half_h),
            DOT_R,
            (207, 189, 255),
            0.78 + 0.22 * voice,
        );
    }
}

/// Размер кадра индикатора.
pub fn size() -> (i32, i32) {
    (W, H)
}

/// Плавный шум: в целых точках — псевдослучайные значения, между ними мягкая
/// интерполяция. Даёт непредсказуемое, но не дёрганое движение и не требует ни
/// генератора случайных чисел, ни состояния.
fn noise(x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let a = hash(i);
    let b = hash(i + 1.0);
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}

fn hash(n: f32) -> f32 {
    let s = (n * 127.1).sin() * 43758.545;
    s - s.floor()
}
/// Залить прямоугольник со скруглёнными углами, сглаживая края по покрытию пикселя.
/// При `half.0 == half.1 == radius` это круг — на нём и построены точки индикатора.
fn rounded_rect(
    px: &mut [u32],
    cx: f32,
    cy: f32,
    half: (f32, f32),
    radius: f32,
    rgb: (u8, u8, u8),
    alpha: f32,
) {
    let radius = radius.min(half.0).min(half.1);
    for y in 0..H {
        for x in 0..W {
            let dx = (x as f32 + 0.5 - cx).abs() - (half.0 - radius);
            let dy = (y as f32 + 0.5 - cy).abs() - (half.1 - radius);
            let d = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - radius;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                blend(px, (y * W + x) as usize, rgb, alpha * cover);
            }
        }
    }
}

/// Наложение «источник поверх» с предумноженной альфой.
fn blend(px: &mut [u32], i: usize, rgb: (u8, u8, u8), a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let dst = px[i];
    let (db, dg, dr, da) = (
        (dst & 0xFF) as f32,
        ((dst >> 8) & 0xFF) as f32,
        ((dst >> 16) & 0xFF) as f32,
        ((dst >> 24) & 0xFF) as f32,
    );
    let inv = 1.0 - a;
    let b = rgb.2 as f32 * a + db * inv;
    let g = rgb.1 as f32 * a + dg * inv;
    let r = rgb.0 as f32 * a + dr * inv;
    let out_a = a * 255.0 + da * inv;
    px[i] = ((out_a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
}

/// Не дать индикатору уехать за край рабочего стола.
///
/// Границы берём по всему виртуальному экрану, а не по основному монитору: иначе на
/// системе с несколькими мониторами индикатор утаскивало бы на главный, за километр
/// от места, где человек печатает.
unsafe fn clamp_to_screen((x, y): (i32, i32)) -> (i32, i32) {
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    (
        x.clamp(vx, (vx + vw - W).max(vx)),
        y.clamp(vy, (vy + vh - H).max(vy)),
    )
}
