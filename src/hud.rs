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
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// Размер окна индикатора (с запасом под свечение).
/// Базовый размер индикатора при масштабе 100%.
pub const BASE_W: f32 = 44.0;
pub const BASE_H: f32 = 32.0;
/// Пределы масштаба: мельче не разглядеть, крупнее начинает мешать.
pub const SCALE_MIN: f32 = 0.6;
pub const SCALE_MAX: f32 = 2.2;

/// Размер окна индикатора для заданного масштаба.
pub fn size_for(scale: f32) -> (i32, i32) {
    let s = scale.clamp(SCALE_MIN, SCALE_MAX);
    ((BASE_W * s).round() as i32, (BASE_H * s).round() as i32)
}
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
    pub w: i32,
    pub h: i32,
    pub scale: f32,
}

impl Canvas {
    pub unsafe fn new(scale: f32) -> Option<Self> {
        let (w, h) = size_for(scale);
        let screen = GetDC(HWND::default());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(HWND::default(), screen);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // сверху вниз
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
            w,
            h,
            scale,
        })
    }

    fn pixels(&self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }

    /// Отрисовать кадр: подложка-пилюля и три точки, дышащие под уровень звука.
    pub fn draw(&self, t: f32, level: f32) {
        let (w, h, s) = (self.w, self.h, self.scale);
        draw_frame(self.pixels(), w, h, s, t, level);
    }

    /// Отдать буфер окну с попиксельной прозрачностью.
    pub unsafe fn present(&self, hwnd: HWND, pos: (i32, i32)) {
        let (x, y) = pos;
        let dst = POINT { x, y };
        let src = POINT { x: 0, y: 0 };
        let size = SIZE { cx: self.w, cy: self.h };
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
pub fn draw_frame(px: &mut [u32], w: i32, h: i32, scale: f32, t: f32, voice: f32) {
    px.fill(0);
    let s = scale.clamp(SCALE_MIN, SCALE_MAX);
    let (pad, gap, dot_r, dot_grow) = (PAD * s, DOT_GAP * s, DOT_R * s, DOT_GROW * s);
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    // Подложка: тёмная полупрозрачная пилюля со скруглением по высоте.
    let half = (w as f32 / 2.0 - pad, h as f32 / 2.0 - pad);
    rounded_rect(px, w, h, cx, cy, half, half.1, (14, 16, 15), 0.62);
    rounded_rect(px, w, h, cx, cy, (half.0 - 0.6, half.1 - 0.6), half.1, (255, 255, 255), 0.05);

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
        let half_h = dot_r + dot_grow * amp;
        let x = cx + (i as f32 - (DOTS as f32 - 1.0) / 2.0) * gap;
        // Ореол повторяет форму — тоже вытягивается вверх-вниз.
        rounded_rect(
            px,
            w,
            h,
            x,
            cy,
            (dot_r * 1.9, half_h * 1.25),
            dot_r * 1.9,
            (157, 122, 255),
            0.13 * amp,
        );
        rounded_rect(
            px,
            w,
            h,
            x,
            cy,
            (dot_r, half_h),
            dot_r,
            (207, 189, 255),
            0.78 + 0.22 * voice,
        );
    }
}

/// Размер кадра индикатора.


/// Значок приложения и трея — тот же язык форм, что у индикатора: тёмная подложка
/// со скруглением и три точки-полоски. Квадрат, потому что значки квадратные.
///
/// Возвращает пиксели как BGRA с предумноженной альфой — в таком виде их ждёт
/// `CreateIcon`; для интерфейса есть `icon_rgba`.
pub fn icon_pixels(size: i32) -> Vec<u32> {
    let mut px = vec![0u32; (size * size) as usize];
    let s = size as f32;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let half = (s * 0.44, s * 0.44);
    let round = s * 0.24;
    rounded_rect(&mut px, size, size, cx, cy, half, round, (18, 20, 19), 0.95);
    rounded_rect(
        &mut px,
        size,
        size,
        cx,
        cy,
        (half.0 - s * 0.02, half.1 - s * 0.02),
        round,
        (255, 255, 255),
        0.06,
    );
    // Разная высота полосок читается как «голос» даже в 16 точек.
    let heights = [0.11f32, 0.19, 0.14];
    let r = s * 0.072;
    for (i, hh) in heights.iter().enumerate() {
        let x = cx + (i as f32 - 1.0) * s * 0.21;
        rounded_rect(
            &mut px,
            size,
            size,
            x,
            cy,
            (r, s * hh),
            r,
            (207, 189, 255),
            0.97,
        );
    }
    px
}

/// Тот же значок обычной RGBA-картинкой (альфа не предумножена) — для окна приложения.
pub fn icon_rgba(size: i32) -> Vec<u8> {
    let px = icon_pixels(size);
    let mut out = Vec::with_capacity(px.len() * 4);
    for p in &px {
        let a = ((p >> 24) & 0xFF) as u32;
        let un = |c: u32| if a == 0 { 0 } else { (c * 255 / a).min(255) as u8 };
        out.push(un((p >> 16) & 0xFF));
        out.push(un((p >> 8) & 0xFF));
        out.push(un(p & 0xFF));
        out.push(a as u8);
    }
    out
}

/// Кадр индикатора в виде картинки для интерфейса: тот же рисунок, что видит
/// пользователь на экране, поэтому предпросмотр не может разойтись с настоящим видом.
pub fn preview(scale: f32, t: f32, voice: f32) -> ([usize; 2], Vec<u8>) {
    let (w, h) = size_for(scale);
    let mut px = vec![0u32; (w * h) as usize];
    draw_frame(&mut px, w, h, scale, t, voice);
    // Буфер лежит как BGRA с предумноженной альфой — переставляем в RGBA.
    let mut rgba = Vec::with_capacity(px.len() * 4);
    for p in &px {
        rgba.push(((p >> 16) & 0xFF) as u8);
        rgba.push(((p >> 8) & 0xFF) as u8);
        rgba.push((p & 0xFF) as u8);
        rgba.push(((p >> 24) & 0xFF) as u8);
    }
    ([w as usize, h as usize], rgba)
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
    w: i32,
    h: i32,
    cx: f32,
    cy: f32,
    half: (f32, f32),
    radius: f32,
    rgb: (u8, u8, u8),
    alpha: f32,
) {
    let radius = radius.min(half.0).min(half.1);
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - cx).abs() - (half.0 - radius);
            let dy = (y as f32 + 0.5 - cy).abs() - (half.1 - radius);
            let d = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - radius;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                blend(px, (y * w + x) as usize, rgb, alpha * cover);
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
