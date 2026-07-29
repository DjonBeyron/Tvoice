//! Вставка текста в активное окно.
//!
//! Два способа:
//! * **Буфер обмена + Ctrl+V** — основной. Вставка атомарна: сколько бы ни было текста,
//!   это одно событие, которое приложение обрабатывает целиком. Прежний буфер возвращаем.
//! * **Клавиатура** (`SendInput` + `KEYEVENTF_UNICODE`) — запасной. Не трогает буфер обмена
//!   и работает там, где Ctrl+V занят (терминалы), но приложения на WinUI/TSF (новый
//!   Блокнот) разбирают поток `VK_PACKET` асинхронно и путают символы: замер на Блокноте
//!   Windows 11 показал брак при паузе <5мс и один испорченный символ из 51 даже при 8мс.
//!
//! Режим выбирается в настройках: «Авто» (буфер, при отказе — клавиатура) либо принудительно.
//!
//! Вставка идёт в отдельном потоке-очереди (`queue_text`), чтобы распознавание
//! не ждало клавиатуру: за время вставки успевает накопиться следующая фраза.

use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    SetForegroundWindow,
};

const INPUT_SIZE: i32 = std::mem::size_of::<INPUT>() as i32;
const VK_V: u16 = 0x56;

/// Способ вставки.
pub const MODE_AUTO: u8 = 0;
pub const MODE_KEYS: u8 = 1;
pub const MODE_CLIPBOARD: u8 = 2;

/// Пауза между символами в клавиатурном режиме.
///
/// Отправлять кусок одним пакетом нельзя: приложения на WinUI/TSF (новый Блокнот)
/// разбирают пачку `VK_PACKET` асинхронно и подставляют символ ПОСЛЕДНЕГО события во все —
/// вместо «понять, что не так с» получается «понять,ссссссссс».
/// Замер на Блокноте Windows 11: 0/1/2мс — сплошной брак, 5–8мс — редкие одиночные ошибки.
/// Поэтому режим и запасной: 8мс — компромисс между скоростью и числом ошибок.
pub const DEFAULT_CHAR_DELAY_US: u32 = 8000;
static CHAR_DELAY_US: AtomicU32 = AtomicU32::new(DEFAULT_CHAR_DELAY_US);

pub fn set_char_delay_us(us: u32) {
    CHAR_DELAY_US.store(us, Ordering::Relaxed);
}
/// Сколько ждём физического отпускания модификаторов хоткея перед вставкой.
const MODIFIERS_WAIT: Duration = Duration::from_millis(400);

static MODE: AtomicU8 = AtomicU8::new(MODE_AUTO);

pub fn set_mode(mode: u8) {
    MODE.store(mode, Ordering::Relaxed);
}

pub fn mode_name(mode: u8) -> &'static str {
    match mode {
        MODE_KEYS => "Клавиатура (для терминалов)",
        MODE_CLIPBOARD => "Буфер обмена (Ctrl+V)",
        _ => "Авто — буфер обмена",
    }
}

/// Поставить текст в очередь на вставку и сразу вернуть управление.
pub fn queue_text(text: &str) {
    queue_replace(0, text);
}

/// Стереть `back` символов и вставить `text` — уточнение уже вставленного черновика.
/// Возвращает управление сразу; работа идёт в потоке вставки.
pub fn queue_replace(back: usize, text: &str) {
    let _ = queue().send((back, text.to_string()));
}

/// Сколько символов позволяем стереть за одно уточнение: если расхождение больше,
/// это уже не «поправить хвост», а признак того, что курсор не там, где мы думаем.
const MAX_BACKSPACES: usize = 120;

fn queue() -> &'static Sender<(usize, String)> {
    static Q: OnceLock<Sender<(usize, String)>> = OnceLock::new();
    Q.get_or_init(|| {
        let (tx, rx) = channel::<(usize, String)>();
        std::thread::Builder::new()
            .name("tvoice-inject".into())
            .spawn(move || {
                while let Ok((back, text)) = rx.recv() {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        replace_text(back, &text)
                    }));
                    if r.is_err() {
                        crate::logln!("inject: ПАНИКА в потоке вставки (поймана)");
                    }
                }
            })
            .expect("не удалось запустить поток вставки");
        tx
    })
}

/// Стереть хвост и напечатать новый (синхронно).
pub fn replace_text(back: usize, text: &str) {
    if back == 0 {
        type_text(text);
        return;
    }
    if back > MAX_BACKSPACES {
        crate::logln!("inject: запрошено стереть {back} символов — отказ (слишком много)");
        return;
    }
    focus_target();
    prepare_modifiers();
    let t0 = Instant::now();
    // Backspace — обычная клавиша (не VK_PACKET), но шлём с зазором: приложение должно
    // успеть обработать каждое удаление, иначе стираем меньше, чем просили.
    for _ in 0..back {
        if !send_batch(&[key_vk(VK_BACK.0, false), key_vk(VK_BACK.0, true)]) {
            break;
        }
        spin(Duration::from_millis(3));
    }
    crate::logln!(
        "inject: стёрто {back} симв. за {:.0}мс",
        t0.elapsed().as_secs_f32() * 1000.0
    );
    if !text.is_empty() {
        type_text(text);
    }
}

/// Окно, в которое диктуем (запомнено на старте диктовки). 0 — «куда попадёт».
static TARGET: AtomicIsize = AtomicIsize::new(0);

/// Запомнить активное окно как цель вставки — вызывается при старте диктовки.
///
/// Потоковая вставка идёт, пока пользователь ещё говорит, и любой перехват фокуса
/// (наше же окно, всплывающая подсказка, что угодно) уводил бы слова в никуда.
pub fn remember_target() {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() || is_ours(hwnd) {
        TARGET.store(0, Ordering::Relaxed);
        crate::logln!("inject: цель не задана (активно наше окно) — вставка пойдёт в фокус");
        return;
    }
    TARGET.store(hwnd.0 as isize, Ordering::Relaxed);
    crate::logln!("inject: цель вставки — {}", window_info(hwnd));
}

/// Напечатать/вставить текст в окно, которое сейчас в фокусе (синхронно).
pub fn type_text(text: &str) {
    if text.is_empty() {
        return;
    }
    focus_target();
    prepare_modifiers();

    let mode = MODE.load(Ordering::Relaxed);
    let len = text.chars().count();
    let clipboard_first = mode != MODE_KEYS;

    let t0 = Instant::now();
    let (ok, way) = if clipboard_first {
        if paste_via_clipboard(text) {
            (true, "буфер")
        } else {
            (send_unicode(text), "буфер→клавиатура")
        }
    } else if send_unicode(text) {
        (true, "клавиатура")
    } else {
        (paste_via_clipboard(text), "клавиатура→буфер")
    };
    crate::logln!(
        "inject: {len} симв. через {way} за {:.0}мс{} → {}",
        t0.elapsed().as_secs_f32() * 1000.0,
        if ok { "" } else { " — НЕ УДАЛОСЬ" },
        window_info(unsafe { GetForegroundWindow() })
    );
}

/// Вернуть фокус в окно, где начали диктовать (если он куда-то ушёл).
fn focus_target() {
    let raw = TARGET.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let target = HWND(raw as *mut core::ffi::c_void);
    if !unsafe { IsWindow(target) }.as_bool() {
        TARGET.store(0, Ordering::Relaxed);
        crate::logln!("inject: целевое окно закрыто — вставка пойдёт в текущий фокус");
        return;
    }
    let fg = unsafe { GetForegroundWindow() };
    if fg == target {
        return;
    }

    // Windows не даёт фоновому процессу просто так менять активное окно; штатный обход —
    // на момент вызова присоединиться к очереди ввода текущего активного потока.
    let ours = unsafe { GetCurrentThreadId() };
    let theirs = unsafe { GetWindowThreadProcessId(fg, None) };
    let attached = theirs != 0 && theirs != ours && unsafe { AttachThreadInput(ours, theirs, true) }.as_bool();
    let ok = unsafe { SetForegroundWindow(target) }.as_bool();
    if attached {
        unsafe {
            let _ = AttachThreadInput(ours, theirs, false);
        }
    }
    crate::logln!(
        "inject: фокус был в {} → возврат в целевое окно {}",
        window_info(fg),
        if ok { "ок" } else { "НЕ УДАЛСЯ" }
    );
    sleep(Duration::from_millis(40)); // дать окну принять фокус
}

fn is_ours(hwnd: HWND) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}

/// Описание активного окна (для логов и автотеста).
pub fn foreground() -> String {
    window_info(unsafe { GetForegroundWindow() })
}

/// Короткое описание окна для лога: заголовок, класс, свой/чужой процесс.
fn window_info(hwnd: HWND) -> String {
    if hwnd.0.is_null() {
        return "<нет активного окна>".into();
    }
    let text = |f: unsafe fn(HWND, &mut [u16]) -> i32| {
        let mut buf = [0u16; 96];
        let n = unsafe { f(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    };
    let title = text(|h, b| unsafe { GetWindowTextW(h, b) });
    let class = text(|h, b| unsafe { GetClassNameW(h, b) });
    format!(
        "{title:?} [{class}]{}",
        if is_ours(hwnd) { " (наше окно!)" } else { "" }
    )
}

/// Хоткей мог остаться зажатым: Ctrl/Alt превращают ввод в сочетания клавиш.
/// Ждём отпускания недолго, а если пользователь держит клавиши — снимаем их логически.
fn prepare_modifiers() {
    let down = |vk: u16| (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0;
    let any = || down(VK_CONTROL.0) || down(VK_MENU.0) || down(VK_LWIN.0) || down(VK_RWIN.0);
    if !any() {
        return;
    }
    let start = Instant::now();
    while start.elapsed() < MODIFIERS_WAIT && any() {
        sleep(Duration::from_millis(10));
    }
    if any() {
        let vks = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN];
        let inputs: Vec<INPUT> = vks.iter().map(|vk| key_vk(vk.0, true)).collect();
        unsafe {
            SendInput(&inputs, INPUT_SIZE);
        }
        crate::logln!("inject: модификаторы всё ещё зажаты — сняты принудительно");
    }
    sleep(Duration::from_millis(10));
}

/// «Печать» текста событиями Unicode.
///
/// Символы отправляются по одному с микропаузой: приложение должно успеть разобрать
/// каждое событие. Пауза выдерживается активным ожиданием — обычный `sleep` на Windows
/// округляется до ~15 мс, что превратило бы фразу в полсекунды.
fn send_unicode(text: &str) -> bool {
    let delay = Duration::from_micros(CHAR_DELAY_US.load(Ordering::Relaxed) as u64);
    // Перевод строки как Unicode-символ игнорируется многими полями ввода — шлём Enter.
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 && !send_batch(&[key_vk(VK_RETURN.0, false), key_vk(VK_RETURN.0, true)]) {
            return false;
        }
        let units: Vec<u16> = line.encode_utf16().collect();
        if delay.is_zero() {
            let mut inputs: Vec<INPUT> = Vec::with_capacity(units.len() * 2);
            for &unit in &units {
                inputs.push(unicode_event(unit, false));
                inputs.push(unicode_event(unit, true));
            }
            if !send_batch(&inputs) {
                return false;
            }
            continue;
        }
        // Суррогатная пара — это один символ, её половинки нельзя разносить по пакетам.
        let mut idx = 0;
        while idx < units.len() {
            let take = if (0xD800..0xDC00).contains(&units[idx]) { 2 } else { 1 };
            let end = (idx + take).min(units.len());
            let mut inputs: Vec<INPUT> = Vec::with_capacity((end - idx) * 2);
            for &unit in &units[idx..end] {
                inputs.push(unicode_event(unit, false));
                inputs.push(unicode_event(unit, true));
            }
            if !send_batch(&inputs) {
                return false;
            }
            idx = end;
            if idx < units.len() {
                spin(delay);
            }
        }
    }
    true
}

/// Активное ожидание: нужна точность в единицы миллисекунд, которой `sleep` не даёт.
fn spin(d: Duration) {
    let t0 = Instant::now();
    while t0.elapsed() < d {
        std::hint::spin_loop();
    }
}

/// Отправить сочетание Ctrl+клавиша (нужно автотесту вставки: Ctrl+S в Блокноте).
pub fn ctrl_combo(vk: u16) {
    send_batch(&[
        key_vk(VK_CONTROL.0, false),
        key_vk(vk, false),
        key_vk(vk, true),
        key_vk(VK_CONTROL.0, true),
    ]);
}

fn send_batch(inputs: &[INPUT]) -> bool {
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe { SendInput(inputs, INPUT_SIZE) } as usize;
    if sent != inputs.len() {
        crate::logln!("inject: SendInput отправил {sent} из {}", inputs.len());
        return false;
    }
    true
}

/// Положить текст в буфер и отправить Ctrl+V; затем вернуть прежний буфер.
fn paste_via_clipboard(text: &str) -> bool {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            crate::logln!("inject: clipboard недоступен: {e}");
            return false;
        }
    };
    let previous = clipboard.get_text().ok();

    if let Err(e) = clipboard.set_text(text) {
        crate::logln!("inject: set_text ошибка: {e}");
        return false;
    }
    sleep(Duration::from_millis(30)); // дать буферу примениться

    let ok = send_batch(&[
        key_vk(VK_CONTROL.0, false),
        key_vk(VK_V, false),
        key_vk(VK_V, true),
        key_vk(VK_CONTROL.0, true),
    ]);

    // Дать приложению вставить до восстановления буфера.
    sleep(Duration::from_millis(120));
    if let Some(prev) = previous {
        let _ = clipboard.set_text(prev);
    }
    ok
}

fn key_vk(vk: u16, key_up: bool) -> INPUT {
    let flags = if key_up {
        KEYEVENTF_KEYUP
    } else {
        Default::default()
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn unicode_event(unit: u16, key_up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
