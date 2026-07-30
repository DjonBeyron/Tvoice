//! Значок в области уведомлений (трей): нативное Win32-окно без рамки и `Shell_NotifyIconW`.
//!
//! Живёт в своём потоке с собственным циклом сообщений — как и оверлей, не зависит от
//! состояния окна приложения. Само окно можно прятать: глобальный хоткей опрашивает
//! состояние клавиш отдельным потоком и работает, даже когда показывать нечего.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, GetWindowLongPtrW,
    GetWindowThreadProcessId, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SendMessageW, SetWindowLongPtrW, ShowWindow, TrackPopupMenu, TranslateMessage, GWL_EXSTYLE,
    HMENU, ICON_BIG, ICON_SMALL, WM_SETICON,
    MF_SEPARATOR, MF_STRING, MSG, SW_HIDE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN, TPM_RIGHTALIGN,
    WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_OVERLAPPED,
};

/// Сообщение от значка в наше окно.
const WM_TRAY: u32 = WM_APP + 1;

const ID_SHOW: usize = 1;
const ID_DICTATE: usize = 2;
const ID_QUIT: usize = 3;

/// Что пользователь выбрал в трее.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayEvent {
    /// Показать окно приложения.
    Show,
    /// Включить/выключить диктовку.
    Dictate,
    /// Выйти из приложения.
    Quit,
}

static EVENTS: OnceLock<Mutex<Sender<TrayEvent>>> = OnceLock::new();
/// Контекст egui: со спрятанным окном событий нет и цикл UI спит, поэтому выбор
/// в меню нужно не только положить в канал, но и разбудить приложение.
static CTX: OnceLock<egui::Context> = OnceLock::new();
/// Идёт ли диктовка — от этого зависит подпись в меню.
static DICTATING: AtomicBool = AtomicBool::new(false);

pub struct Tray {
    rx: Receiver<TrayEvent>,
    hwnd: Arc<Mutex<isize>>,
}

impl Tray {
    /// Создать значок и запустить его цикл сообщений.
    pub fn spawn(tip: &str, ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let _ = EVENTS.set(Mutex::new(tx));
        let _ = CTX.set(ctx);
        let hwnd = Arc::new(Mutex::new(0isize));
        let hwnd_t = Arc::clone(&hwnd);
        let tip = tip.to_string();
        thread::Builder::new()
            .name("tvoice-tray".into())
            .spawn(move || unsafe { run(&tip, hwnd_t) })
            .expect("не удалось запустить поток трея");
        Self { rx, hwnd }
    }

    pub fn drain(&self) -> Vec<TrayEvent> {
        self.rx.try_iter().collect()
    }

    /// Сообщить трею состояние диктовки (подпись пункта меню).
    pub fn set_dictating(&self, on: bool) {
        DICTATING.store(on, Ordering::Relaxed);
    }

    /// Обновить подсказку значка (в ней держим текущий хоткей).
    pub fn set_tip(&self, tip: &str) {
        let h = self.hwnd.lock().map(|v| *v).unwrap_or(0);
        if h == 0 {
            return;
        }
        let mut data = icon_data(HWND(h as *mut core::ffi::c_void));
        data.uFlags = NIF_TIP;
        write_tip(&mut data, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let h = self.hwnd.lock().map(|v| *v).unwrap_or(0);
        if h != 0 {
            let data = icon_data(HWND(h as *mut core::ffi::c_void));
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                let _ = DestroyWindow(HWND(h as *mut core::ffi::c_void));
            }
        }
    }
}

/// Показать или убрать кнопку приложения на панели задач (и в Alt+Tab).
///
/// Нужно, потому что «спрятать» окно по-настоящему нельзя: eframe выполняет кадры только
/// пока окно показано, а без кадров перестают работать и хоткей, и меню трея (проверено:
/// с `Visible(false)` и `Minimized(true)` цикл встаёт). Поэтому окно уезжает за экран —
/// и тогда его кнопку на панели задач нужно убрать руками, иначе по ней будут кликать.
pub fn set_taskbar(show: bool) {
    unsafe {
        let Some(hwnd) = main_window() else {
            crate::logln!("tray: главное окно не найдено — панель задач не тронута");
            return;
        };
        // Флаг применяется только к скрытому окну, поэтому прячем его на мгновение.
        let _ = ShowWindow(hwnd, SW_HIDE);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let tool = WS_EX_TOOLWINDOW.0 as isize;
        let ex = if show { ex & !tool } else { ex | tool };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

/// Главное окно приложения (по заголовку, с проверкой процесса).
unsafe fn main_window() -> Option<HWND> {
    let hwnd = FindWindowW(PCWSTR::null(), windows::core::w!("TVOICE")).ok()?;
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    (pid == GetCurrentProcessId()).then_some(hwnd)
}

fn send(ev: TrayEvent) {
    crate::logln!("tray: выбрано {ev:?}");
    if let Some(tx) = EVENTS.get() {
        if let Ok(tx) = tx.lock() {
            if tx.send(ev).is_err() {
                crate::logln!("tray: очередь событий закрыта");
            }
        }
    }
    // Разбудить приложение: иначе со спрятанным окном событие пролежит в канале вечно.
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    } else {
        crate::logln!("tray: контекст egui неизвестен — приложение не разбудить");
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

fn write_tip(data: &mut NOTIFYICONDATAW, tip: &str) {
    let mut wide: Vec<u16> = tip.encode_utf16().take(data.szTip.len() - 1).collect();
    wide.push(0);
    data.szTip[..wide.len()].copy_from_slice(&wide);
}

unsafe fn run(tip: &str, slot: Arc<Mutex<isize>>) {
    let Ok(hinst) = GetModuleHandleW(None) else {
        return;
    };
    let class = windows::core::w!("TvoiceTray");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassW(&wc);

    // Окно нужно только как адресат сообщений значка — на экране его нет.
    let Ok(hwnd) = CreateWindowExW(
        Default::default(),
        class,
        PCWSTR::null(),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        None,
        HMENU::default(),
        hinst,
        None,
    ) else {
        return;
    };
    if let Ok(mut s) = slot.lock() {
        *s = hwnd.0 as isize;
    }

    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = make_icon(32);
    write_tip(&mut data, tip);
    if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
        crate::logln!("tray: значок создан");
    } else {
        crate::logln!("tray: не удалось создать значок");
    }

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                match lp.0 as u32 {
                    WM_LBUTTONDBLCLK => send(TrayEvent::Show),
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                match (wp.0 & 0xFFFF) as usize {
                    ID_SHOW => send(TrayEvent::Show),
                    ID_DICTATE => send(TrayEvent::Dictate),
                    ID_QUIT => send(TrayEvent::Quit),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let dictate = if DICTATING.load(Ordering::Relaxed) {
        windows::core::w!("Остановить диктовку")
    } else {
        windows::core::w!("Начать диктовку")
    };
    let _ = AppendMenuW(menu, MF_STRING, ID_SHOW, windows::core::w!("Открыть TVOICE"));
    let _ = AppendMenuW(menu, MF_STRING, ID_DICTATE, dictate);
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, windows::core::w!("Выход"));

    // Без этого меню не закроется по клику мимо него (документированная особенность).
    let _ = SetForegroundWindow(hwnd);
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = TrackPopupMenu(
        menu,
        TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
}

/// Значок трея — тот же рисунок, что у индикатора диктовки (см. `hud::icon_pixels`),
/// чтобы приложение узнавалось по одной и той же форме и в трее, и на экране.
unsafe fn make_icon(size: i32) -> windows::Win32::UI::WindowsAndMessaging::HICON {
    let s = size;
    let px = crate::hud::icon_pixels(s);
    let mut bgra = Vec::with_capacity(px.len() * 4);
    for p in &px {
        bgra.extend_from_slice(&p.to_le_bytes());
    }
    let and_mask = vec![0u8; (s * s / 8) as usize];
    let hinst: windows::Win32::Foundation::HINSTANCE =
        GetModuleHandleW(None).map(Into::into).unwrap_or_default();
    CreateIcon(hinst, s, s, 1, 32, and_mask.as_ptr(), bgra.as_ptr()).unwrap_or_default()
}

/// Поставить окну приложения тот же значок, что и в трее.
///
/// eframe задаёт значок при создании окна, но до заголовка и панели задач он доходит
/// не всегда. `WM_SETICON` работает надёжно: отдельно мелкий значок для заголовка и
/// крупный для Alt+Tab и панели задач.
pub fn set_window_icon() {
    unsafe {
        let Some(hwnd) = main_window() else {
            crate::logln!("tray: окно не найдено — значок приложения не поставлен");
            return;
        };
        let big = make_icon(32);
        let small = make_icon(16);
        SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(big.0 as isize));
        SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_SMALL as usize),
            LPARAM(small.0 as isize),
        );
        crate::logln!("tray: значок приложения установлен");
    }
}
