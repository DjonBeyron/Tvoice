//! Где сейчас стоит курсор ввода — к нему привязывается индикатор диктовки.
//!
//! Два источника, по убыванию точности:
//! 1. `GetGUIThreadInfo` — настоящий Win32-курсор. Быстро и точно, но есть только в
//!    приложениях, которые его заводят (Блокнот, поля ввода, редакторы).
//! 2. UI Automation — путь для Chromium/Electron/UWP (Claude, браузеры, VS Code):
//!    у сфокусированного элемента спрашиваем каретку через `TextPattern2`, а если его
//!    нет — берём рамку самого поля ввода. Это уже «где текст», а не «где мышь».
//!
//! UIA работает через COM и лезет в чужой процесс: один вызов легко занимает десятки
//! миллисекунд. Поэтому опрос живёт в своём потоке, а рисование берёт готовый результат.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationTextPattern2, UIA_TextPattern2Id,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;

/// Как часто опрашиваем позицию, пока индикатор виден.
const POLL: Duration = Duration::from_millis(180);
/// Сколько результат считается свежим (курсор мог исчезнуть вместе с фокусом).
const FRESH: Duration = Duration::from_millis(1500);

struct Found {
    /// Низ курсора ввода в экранных координатах.
    pos: (i32, i32),
    at: Instant,
    /// Откуда взято — для лога.
    source: &'static str,
}

static FOUND: Mutex<Option<Found>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Включить/выключить опрос (обычно — на время показа индикатора).
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

/// Последняя известная позиция курсора ввода и её источник.
pub fn position() -> Option<((i32, i32), &'static str)> {
    let g = FOUND.lock().ok()?;
    let f = g.as_ref()?;
    (f.at.elapsed() < FRESH).then_some((f.pos, f.source))
}

/// Запустить фоновый поиск курсора ввода (идемпотентно).
pub fn watch() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        std::thread::Builder::new()
            .name("tvoice-caret".into())
            .spawn(|| unsafe { run() })
            .expect("не удалось запустить поток поиска курсора ввода");
    });
}

unsafe fn run() {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let uia: Option<IUIAutomation> = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| crate::logln!("caret: UI Automation недоступна ({e}) — только Win32-курсор"))
        .ok();

    loop {
        if !ACTIVE.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }
        let found = win32_caret()
            .map(|p| (p, "win32"))
            .or_else(|| uia.as_ref().and_then(|u| uia_caret(u)));
        if let Ok(mut g) = FOUND.lock() {
            match found {
                Some((pos, source)) => {
                    *g = Some(Found {
                        pos,
                        at: Instant::now(),
                        source,
                    })
                }
                // Ничего не нашли — старое значение просто протухнет само.
                None => {}
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Настоящий Win32-курсор ввода активного окна.
unsafe fn win32_caret() -> Option<(i32, i32)> {
    let fg = GetForegroundWindow();
    if fg.0.is_null() {
        return None;
    }
    let tid = GetWindowThreadProcessId(fg, None);
    if tid == 0 {
        return None;
    }
    let mut gi = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    GetGUIThreadInfo(tid, &mut gi).ok()?;
    let r: RECT = gi.rcCaret;
    if gi.hwndCaret.0.is_null() || r.bottom - r.top <= 0 {
        return None;
    }
    let mut p = POINT {
        x: r.left,
        y: r.bottom,
    };
    let _ = ClientToScreen(gi.hwndCaret, &mut p);
    Some((p.x, p.y))
}

/// Каретка (или хотя бы поле ввода) через UI Automation.
unsafe fn uia_caret(uia: &IUIAutomation) -> Option<((i32, i32), &'static str)> {
    let el = uia.GetFocusedElement().ok()?;

    // Точный путь: каретка внутри текстового шаблона.
    if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) {
        let mut active = Default::default();
        if let Ok(range) = tp.GetCaretRange(&mut active) {
            if let Ok(sa) = range.GetBoundingRectangles() {
                let rects = read_doubles(sa);
                // Каждый прямоугольник — четвёрка: left, top, width, height.
                if rects.len() >= 4 {
                    let (l, t, h) = (rects[0], rects[1], rects[3]);
                    return Some(((l as i32, (t + h) as i32), "uia-caret"));
                }
            }
        }
    }

    // Запасной путь: рамка самого поля ввода — уже лучше, чем позиция мыши.
    let r = el.CurrentBoundingRectangle().ok()?;
    if r.right - r.left <= 0 || r.bottom - r.top <= 0 {
        return None;
    }
    Some(((r.left, r.bottom), "uia-поле"))
}

/// Забрать значения из SAFEARRAY с double и освободить его.
unsafe fn read_doubles(sa: *mut SAFEARRAY) -> Vec<f64> {
    let mut out = Vec::new();
    if let (Ok(lo), Ok(hi)) = (SafeArrayGetLBound(sa, 1), SafeArrayGetUBound(sa, 1)) {
        for i in lo..=hi {
            let mut v = 0f64;
            if SafeArrayGetElement(sa, &i, &mut v as *mut f64 as *mut core::ffi::c_void).is_ok() {
                out.push(v);
            }
        }
    }
    let _ = SafeArrayDestroy(sa);
    out
}
