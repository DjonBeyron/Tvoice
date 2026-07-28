//! Отслеживание того, что пользователь сам трогал клавиатуру или мышь.
//!
//! Потоковая диктовка уточняет уже вставленный текст: стирает хвост Backspace'ами и пишет
//! заново. Это безопасно ровно до тех пор, пока между нашими вставками никто не печатает
//! и не переставляет курсор — иначе Backspace съест чужие символы.
//!
//! Низкоуровневые хуки различают настоящий ввод и наш собственный по флагу `LLKHF_INJECTED`,
//! поэтому наши же `SendInput` не считаются «вмешательством».

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN,
};

/// Миллисекунды от старта наблюдения до последнего живого ввода.
static LAST: AtomicU64 = AtomicU64::new(0);
/// Клавиша хоткея диктовки: её нажатие — не «вмешательство» (ей же диктовку и останавливают).
static IGNORE_VK: AtomicU16 = AtomicU16::new(0);

fn epoch() -> &'static Instant {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64
}

/// Запустить наблюдение (идемпотентно).
pub fn watch() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = epoch();
        std::thread::Builder::new()
            .name("tvoice-userinput".into())
            .spawn(|| unsafe {
                let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), None, 0);
                let ms = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
                if kb.is_err() || ms.is_err() {
                    crate::logln!("userinput: хуки не установлены — уточнение текста отключится");
                    return;
                }
                // Хукам нужен цикл сообщений в своём потоке.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .expect("не удалось запустить поток наблюдения за вводом");
    });
}

/// Какую клавишу не считать вмешательством (основная клавиша хоткея).
pub fn ignore_vk(vk: u16) {
    IGNORE_VK.store(vk, Ordering::Relaxed);
}

/// Отметка времени «сейчас» в тех же единицах, что и `touched_since`.
pub fn mark() -> u64 {
    now_ms()
}

/// Трогал ли пользователь клавиатуру/мышь после отметки `since`.
pub fn touched_since(since: u64) -> bool {
    LAST.load(Ordering::Relaxed) > since
}

unsafe extern "system" fn kb_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injected = info.flags.0 & LLKHF_INJECTED.0 != 0u32;
        let ignored = info.vkCode as u16 == IGNORE_VK.load(Ordering::Relaxed);
        if !injected && !ignored {
            LAST.store(now_ms(), Ordering::Relaxed);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Движение мыши игнорируем — курсор ввода переставляют кликом.
    let click = matches!(
        wp.0 as u32,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
    );
    if code >= 0 && click {
        let info = &*(lp.0 as *const MSLLHOOKSTRUCT);
        if info.flags & LLMHF_INJECTED == 0 {
            LAST.store(now_ms(), Ordering::Relaxed);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}
