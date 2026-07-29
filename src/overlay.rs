//! Индикатор диктовки — нативное Win32-окно (layered, topmost, click-through).
//!
//! Живёт в своём потоке и не зависит от eframe/GL: работает, даже когда основное окно
//! свёрнуто в трей. Показывает три точки, пульсирующие в такт голосу.
//!
//! Здесь только окно, позиционирование и превращение громкости микрофона в амплитуду;
//! сам рисунок кадра — в `hud.rs`.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetSystemMetrics, PeekMessageW, RegisterClassW, ShowWindow, HMENU, MSG,
    PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use crate::hud::{self, Canvas};

/// Насколько громче уровня тишины должен быть пик, чтобы точка вытянулась на всю высоту.
const VOICE_RANGE: f32 = 0.22;

/// Куда ставить индикатор во время диктовки.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// За указателем мыши (по умолчанию).
    Cursor,
    /// К курсору ввода — точной каретке, а где её нет, к рамке поля ввода.
    Caret,
    TopLeft,
    TopCenter,
    TopRight,
    LeftCenter,
    RightCenter,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 10] = [
        Anchor::Cursor,
        Anchor::Caret,
        Anchor::TopLeft,
        Anchor::TopCenter,
        Anchor::TopRight,
        Anchor::LeftCenter,
        Anchor::RightCenter,
        Anchor::BottomLeft,
        Anchor::BottomCenter,
        Anchor::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Anchor::Cursor => "У указателя мыши",
            Anchor::Caret => "У курсора ввода",
            Anchor::TopLeft => "Сверху слева",
            Anchor::TopCenter => "Сверху по центру",
            Anchor::TopRight => "Сверху справа",
            Anchor::LeftCenter => "Слева по центру",
            Anchor::RightCenter => "Справа по центру",
            Anchor::BottomLeft => "Снизу слева",
            Anchor::BottomCenter => "Снизу по центру",
            Anchor::BottomRight => "Снизу справа",
        }
    }

    /// Имя для config.json — читаемое, чтобы файл можно было править руками.
    pub fn id(self) -> &'static str {
        match self {
            Anchor::Cursor => "cursor",
            Anchor::Caret => "caret",
            Anchor::TopLeft => "top-left",
            Anchor::TopCenter => "top-center",
            Anchor::TopRight => "top-right",
            Anchor::LeftCenter => "left-center",
            Anchor::RightCenter => "right-center",
            Anchor::BottomLeft => "bottom-left",
            Anchor::BottomCenter => "bottom-center",
            Anchor::BottomRight => "bottom-right",
        }
    }

    pub fn from_id(s: &str) -> Anchor {
        Anchor::ALL
            .into_iter()
            .find(|a| a.id() == s)
            .unwrap_or(Anchor::Cursor)
    }

    fn index(self) -> u8 {
        Anchor::ALL.iter().position(|a| *a == self).unwrap_or(0) as u8
    }

    fn from_index(i: u8) -> Anchor {
        *Anchor::ALL.get(i as usize).unwrap_or(&Anchor::Cursor)
    }
}

/// Отступ индикатора от края экрана.
const EDGE: i32 = 24;

static ANCHOR: AtomicU8 = AtomicU8::new(0);
/// Масштаб индикатора в процентах (100 = базовый размер).
static SCALE_PCT: AtomicU32 = AtomicU32::new(100);

pub fn set_scale(scale: f32) {
    SCALE_PCT.store((scale * 100.0).round().max(1.0) as u32, Ordering::Relaxed);
}

pub fn current_scale() -> f32 {
    SCALE_PCT.load(Ordering::Relaxed) as f32 / 100.0
}

pub fn set_anchor(a: Anchor) {
    ANCHOR.store(a.index(), Ordering::Relaxed);
}

pub fn current_anchor() -> Anchor {
    Anchor::from_index(ANCHOR.load(Ordering::Relaxed))
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
    let (w0, h0) = hud::size_for(current_scale());
    let Ok(hwnd) = CreateWindowExW(
        ex_style,
        class_name,
        PCWSTR::null(),
        WS_POPUP,
        0,
        0,
        w0,
        h0,
        HWND::default(),
        HMENU::default(),
        HINSTANCE(hinst.0),
        None,
    ) else {
        return;
    };

    let mut canvas = match Canvas::new(current_scale()) {
        Some(c) => c,
        None => {
            crate::logln!("overlay: не удалось создать буфер рисования");
            return;
        }
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

        // Размер поменяли в настройках — пересобираем буфер под новый масштаб.
        let scale = current_scale();
        if (scale - canvas.scale).abs() > 0.001 {
            if let Some(c) = Canvas::new(scale) {
                canvas = c;
                crate::logln!("overlay: масштаб {:.0}%", scale * 100.0);
            }
        }

        if state.visible.load(Ordering::Relaxed) {
            // Поиск курсора ввода идёт в своём потоке и только пока индикатор виден:
            // вызовы UI Automation лезут в чужой процесс и стоят десятки миллисекунд.
            if current_anchor() == Anchor::Caret {
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

            let (pos, source) = anchor(canvas.w, canvas.h);
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
pub fn voice_amount(level: f32, floor: &mut f32) -> f32 {
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
unsafe fn anchor(w: i32, h: i32) -> ((i32, i32), &'static str) {
    let mode = current_anchor();
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);

    let (pos, source) = match mode {
        Anchor::Caret => match crate::caret::position() {
            // Чуть ниже и правее строки ввода, чтобы не перекрывать сам текст.
            Some((p, source)) => ((p.0 + 6, p.1 + 4), source),
            None => ((pt.x + 14, pt.y - h - 2), "мышь"),
        },
        Anchor::Cursor => ((pt.x + 14, pt.y - h - 2), "мышь"),
        _ => {
            // Экран выбираем по указателю мыши: пользователь смотрит туда, где мышь,
            // а активное окно может быть на другом мониторе (или его нет вовсе).
            // Рабочая область, а не весь экран, — иначе нижние места уедут под панель задач.
            let r = work_area_at(pt);
            let (cx, cy) = ((r.left + r.right) / 2 - w / 2, (r.top + r.bottom) / 2 - h / 2);
            let (left, right) = (r.left + EDGE, r.right - w - EDGE);
            let (top, bottom) = (r.top + EDGE, r.bottom - h - EDGE);
            let p = match mode {
                Anchor::TopLeft => (left, top),
                Anchor::TopCenter => (cx, top),
                Anchor::TopRight => (right, top),
                Anchor::LeftCenter => (left, cy),
                Anchor::RightCenter => (right, cy),
                Anchor::BottomLeft => (left, bottom),
                Anchor::BottomCenter => (cx, bottom),
                Anchor::BottomRight => (right, bottom),
                _ => (cx, cy),
            };
            (p, "экран")
        }
    };

    // Держим индикатор целиком на том мониторе, куда он попал: у края экрана он иначе
    // разрезается пополам между двумя мониторами.
    let area = work_area_at(POINT {
        x: pos.0 + w / 2,
        y: pos.1 + h / 2,
    });
    let clamped = (
        pos.0.clamp(area.left, (area.right - w).max(area.left)),
        pos.1.clamp(area.top, (area.bottom - h).max(area.top)),
    );
    (clamped, source)
}

/// Рабочая область монитора, на котором лежит точка (ближайшего, если точка вне всех).
unsafe fn work_area_at(pt: POINT) -> RECT {
    let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(monitor, &mut info).as_bool() {
        info.rcWork
    } else {
        // Запасной вариант — весь виртуальный экран.
        RECT {
            left: GetSystemMetrics(SM_XVIRTUALSCREEN),
            top: GetSystemMetrics(SM_YVIRTUALSCREEN),
            right: GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

