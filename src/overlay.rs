//! Индикатор диктовки — нативное Win32-окно (layered, topmost, click-through).
//!
//! Живёт в своём потоке и не зависит от eframe/GL: работает, даже когда основное окно
//! свёрнуто в трей. Показывает три точки, пульсирующие в такт голосу.
//!
//! Здесь только окно, позиционирование и превращение громкости микрофона в амплитуду;
//! сам рисунок кадра — в `hud.rs`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    PeekMessageW, RegisterClassW, ShowWindow, HMENU, MSG, PM_REMOVE, SW_HIDE, SW_SHOWNOACTIVATE,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use crate::hud::{Canvas, H};

/// Насколько громче уровня тишины должен быть пик, чтобы точка вытянулась на всю высоту.
const VOICE_RANGE: f32 = 0.22;

/// Привязывать индикатор к курсору ввода вместо указателя мыши.
/// По умолчанию выключено: в приложениях на Chromium точной каретки нет, и индикатор
/// встаёт к рамке поля ввода, что не всегда там, где ждёшь. Правится в config.json.
static ANCHOR_CARET: AtomicBool = AtomicBool::new(false);

pub fn set_anchor_caret(on: bool) {
    ANCHOR_CARET.store(on, Ordering::Relaxed);
}

struct State {
    visible: AtomicBool,
    level_bits: AtomicU32,
    /// Счётчик уровня прямо из захвата — чтобы брать громкость 30 раз в секунду,
    /// а не с частотой перерисовки окна (в трее это всего 10 раз).
    mic: std::sync::OnceLock<Arc<AtomicU32>>,
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
            mic: std::sync::OnceLock::new(),
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

    /// Подключить счётчик уровня микрофона напрямую.
    pub fn attach_level(&self, mic: Arc<AtomicU32>) {
        let _ = self.state.mic.set(mic);
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe fn run(state: Arc<State>, stop: Arc<AtomicBool>) {
    let Ok(hinst) = GetModuleHandleW(None) else {
        return;
    };
    let class_name = windows::core::w!("TvoiceOverlay");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: HINSTANCE(hinst.0),
        lpszClassName: class_name,
        ..Default::default()
    };
    RegisterClassW(&wc);

    let ex_style =
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let Ok(hwnd) = CreateWindowExW(
        ex_style,
        class_name,
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        crate::hud::W,
        H,
        HWND::default(),
        HMENU::default(),
        HINSTANCE(hinst.0),
        None,
    ) else {
        return;
    };

    let Some(canvas) = Canvas::new() else {
        crate::logln!("overlay: не удалось создать буфер рисования");
        return;
    };

    let t0 = Instant::now();
    let mut smooth = 0.0f32;
    let mut floor = 0.01f32; // плавающий уровень тишины этого микрофона
    let mut shown = false;
    let mut last_source = ""; // прошлый источник позиции (для лога)

    while !stop.load(Ordering::Relaxed) {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, hwnd, 0, 0, PM_REMOVE).as_bool() {
            DispatchMessageW(&msg);
        }

        if state.visible.load(Ordering::Relaxed) {
            // Поиск курсора ввода идёт в своём потоке и только пока индикатор виден:
            // вызовы UI Automation лезут в чужой процесс и стоят десятки миллисекунд.
            if ANCHOR_CARET.load(Ordering::Relaxed) {
                crate::caret::watch();
                crate::caret::set_active(true);
            }
            let raw = match state.mic.get() {
                Some(mic) => f32::from_bits(mic.load(Ordering::Relaxed)),
                None => f32::from_bits(state.level_bits.load(Ordering::Relaxed)),
            };
            let voice = voice_amount(raw.clamp(0.0, 1.0), &mut floor);
            // Атака мгновенная, спад быстрый: точки должны прыгать на каждом слоге,
            // а не переливаться. Сглаживать нарастание нельзя — от этого «вязкость».
            smooth = if voice > smooth {
                voice
            } else {
                smooth * 0.6 + voice * 0.4
            };

            let (pos, source) = anchor();
            if source != last_source {
                crate::logln!("overlay: привязка — {source}, позиция {pos:?}");
                last_source = source;
            }
            canvas.draw(t0.elapsed().as_secs_f32(), smooth);
            canvas.present(hwnd, pos);
            if !shown {
                // Окно создано скрытым, а UpdateLayeredWindow само его не показывает.
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                shown = true;
            }
        } else if shown {
            let _ = ShowWindow(hwnd, SW_HIDE);
            shown = false;
            last_source = "";
            crate::caret::set_active(false);
        }

        thread::sleep(Duration::from_millis(33));
    }

    let _ = DestroyWindow(hwnd);
}

/// Привести пиковый уровень микрофона к 0…1, отсчитывая от уровня тишины.
///
/// Постоянное усиление тут не работает: у пика фона и пика речи разница всего в
/// несколько раз, поэтому любой множитель либо «зажигает» точки в тишине, либо
/// упирается в потолок на первом же слове. Поэтому держим плавающую оценку тишины
/// (вниз быстро, вверх еле-еле) и растягиваем в 0…1 то, что над ней.
fn voice_amount(level: f32, floor: &mut f32) -> f32 {
    *floor = if level < *floor {
        *floor * 0.9 + level * 0.1
    } else {
        (*floor * 1.0005).min(0.08)
    };
    let quiet = (*floor * 2.0).max(0.012);
    let loud = quiet + VOICE_RANGE;
    // Показатель 0.65 вместо корня: чуть меньше сжатия — громкое и тихое различимее.
    ((level - quiet) / (loud - quiet)).clamp(0.0, 1.0).powf(0.65)
}


/// Прогнать последовательность пиковых уровней через ту же математику, что и живой
/// индикатор: приведение к 0…1 и огибающая. Нужно, чтобы проверять резкость отклика
/// числами, а не на глаз.
pub fn simulate_pulse(levels: &[f32]) -> Vec<f32> {
    let mut floor = 0.01f32;
    let mut smooth = 0.0f32;
    levels
        .iter()
        .map(|&l| {
            let voice = voice_amount(l.clamp(0.0, 1.0), &mut floor);
            smooth = if voice > smooth {
                voice
            } else {
                smooth * 0.6 + voice * 0.4
            };
            smooth
        })
        .collect()
}

/// Куда поставить индикатор: к курсору ввода (мигающей палочке), а если его нет —
/// к указателю мыши. `true` во втором элементе — позиция взята от курсора ввода.
unsafe fn anchor() -> ((i32, i32), &'static str) {
    if ANCHOR_CARET.load(Ordering::Relaxed) {
        if let Some((p, source)) = crate::caret::position() {
            // Чуть ниже и правее строки ввода, чтобы не перекрывать сам текст.
            return ((p.0 + 6, p.1 + 4), source);
        }
    }
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    ((pt.x + 14, pt.y - H - 2), "мышь")
}

