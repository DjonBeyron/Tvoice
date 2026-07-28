//! Глобальный хоткей диктовки (push-to-talk), настраиваемый захватом.
//!
//! Опрашиваем состояние клавиш/кнопок мыши через `GetAsyncKeyState` в фоновом потоке —
//! это ловит и нажатие, и отпускание, работает поверх любого окна, поддерживает и мышь.
//! Комбинация задаётся «захватом»: пользователь зажимает нужные клавиши, и мы их считываем.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, MapVirtualKeyW, MAPVK_VK_TO_VSC, VK_CONTROL, VK_LWIN,
    VK_MENU, VK_RWIN, VK_SHIFT,
};

/// Настройка комбинации.
#[derive(Clone)]
pub struct HotkeyConfig {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Виртуальный код основной клавиши/кнопки (может быть мышью: 0x02/0x04/0x05/0x06).
    pub vk: u16,
    pub key_name: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
            vk: 0x20, // Space
            key_name: "Space".to_string(),
        }
    }
}

impl HotkeyConfig {
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        parts.push(self.key_name.clone());
        parts.join(" + ")
    }

    fn is_down(&self) -> bool {
        if self.vk == 0 {
            return false;
        }
        (!self.ctrl || key_down(VK_CONTROL.0))
            && (!self.alt || key_down(VK_MENU.0))
            && (!self.shift || key_down(VK_SHIFT.0))
            && (!self.win || key_down(VK_LWIN.0) || key_down(VK_RWIN.0))
            && key_down(self.vk)
    }

    /// Признак «опасной» комбинации: обычная печатная клавиша без модификаторов
    /// (будет срабатывать при обычном наборе).
    pub fn is_risky(&self) -> bool {
        let has_mod = self.ctrl || self.alt || self.shift || self.win;
        !has_mod && is_typing_key(self.vk)
    }
}

pub type SharedHotkey = Arc<Mutex<HotkeyConfig>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

pub struct Hotkey {
    rx: Receiver<HotkeyEvent>,
    stop: Arc<AtomicBool>,
    pub config: SharedHotkey,
    capturing: Arc<AtomicBool>,
}

impl Hotkey {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let config: SharedHotkey = Arc::new(Mutex::new(HotkeyConfig::default()));
        let capturing = Arc::new(AtomicBool::new(false));

        let stop_t = Arc::clone(&stop);
        let config_t = Arc::clone(&config);
        let capturing_t = Arc::clone(&capturing);

        thread::Builder::new()
            .name("tvoice-hotkey".into())
            .spawn(move || {
                let mut was_down = false;
                while !stop_t.load(Ordering::Relaxed) {
                    if capturing_t.load(Ordering::Relaxed) {
                        // Режим захвата: ждём нажатие любой не-модификаторной клавиши/кнопки.
                        if let Some(cfg) = try_capture() {
                            if let Ok(mut c) = config_t.lock() {
                                *c = cfg;
                            }
                            capturing_t.store(false, Ordering::Relaxed);
                            was_down = true; // не считаем это же нажатие за старт диктовки
                            ctx.request_repaint();
                        }
                    } else {
                        let down = config_t.lock().map(|c| c.is_down()).unwrap_or(false);
                        if down != was_down {
                            let ev = if down {
                                HotkeyEvent::Pressed
                            } else {
                                HotkeyEvent::Released
                            };
                            if tx.send(ev).is_err() {
                                break;
                            }
                            ctx.request_repaint();
                            was_down = down;
                        }
                    }
                    thread::sleep(Duration::from_millis(15));
                }
            })
            .expect("не удалось запустить поток хоткея");

        Self {
            rx,
            stop,
            config,
            capturing,
        }
    }

    pub fn drain(&self) -> Vec<HotkeyEvent> {
        self.rx.try_iter().collect()
    }

    pub fn label(&self) -> String {
        self.config.lock().map(|c| c.label()).unwrap_or_default()
    }

    pub fn start_capture(&self) {
        self.capturing.store(true, Ordering::Relaxed);
    }

    pub fn cancel_capture(&self) {
        self.capturing.store(false, Ordering::Relaxed);
    }

    pub fn is_capturing(&self) -> bool {
        self.capturing.load(Ordering::Relaxed)
    }

    pub fn is_risky(&self) -> bool {
        self.config.lock().map(|c| c.is_risky()).unwrap_or(false)
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn key_down(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

/// Скан нажатой не-модификаторной клавиши/кнопки для режима захвата.
/// Возвращает готовую конфигурацию (с текущими модификаторами), когда что-то нажато.
fn try_capture() -> Option<HotkeyConfig> {
    for vk in 0x02u16..=0xFE {
        if is_modifier(vk) || vk == 0x01 {
            // модификаторы и левую кнопку мыши (ей кликают по кнопке «Задать») пропускаем
            continue;
        }
        if key_down(vk) {
            return Some(HotkeyConfig {
                ctrl: key_down(VK_CONTROL.0),
                alt: key_down(VK_MENU.0),
                shift: key_down(VK_SHIFT.0),
                win: key_down(VK_LWIN.0) || key_down(VK_RWIN.0),
                vk,
                key_name: key_name(vk),
            });
        }
    }
    None
}

fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5
    )
}

/// Печатная клавиша (буква/цифра/пробел/базовая пунктуация) — опасна без модификатора.
fn is_typing_key(vk: u16) -> bool {
    matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0xBA..=0xC0 | 0xDB..=0xDF)
}

/// Человекочитаемое имя клавиши/кнопки.
///
/// Для распространённых клавиш — свои понятные подписи; для остальных (OEM-пунктуация,
/// numpad-операторы и пр.) спрашиваем настоящее имя у Windows (`GetKeyNameTextW`),
/// что корректно с учётом раскладки; иначе — hex-запас.
pub fn key_name(vk: u16) -> String {
    match vk {
        0x01 => return "Мышь: левая".into(),
        0x02 => return "Мышь: правая".into(),
        0x04 => return "Мышь: средняя".into(),
        0x05 => return "Мышь: X1 (назад)".into(),
        0x06 => return "Мышь: X2 (вперёд)".into(),
        0x08 => return "Backspace".into(),
        0x09 => return "Tab".into(),
        0x0D => return "Enter".into(),
        0x13 => return "Pause".into(),
        0x1B => return "Esc".into(),
        0x20 => return "Space".into(),
        0x21 => return "PageUp".into(),
        0x22 => return "PageDown".into(),
        0x23 => return "End".into(),
        0x24 => return "Home".into(),
        0x25 => return "Влево".into(),
        0x26 => return "Вверх".into(),
        0x27 => return "Вправо".into(),
        0x28 => return "Вниз".into(),
        0x2C => return "PrintScreen".into(),
        0x2D => return "Insert".into(),
        0x2E => return "Delete".into(),
        0x5D => return "Menu".into(),
        0x30..=0x39 => return ((b'0' + (vk - 0x30) as u8) as char).to_string(),
        0x41..=0x5A => return ((b'A' + (vk - 0x41) as u8) as char).to_string(),
        0x60..=0x69 => return format!("Num{}", vk - 0x60),
        0x70..=0x87 => return format!("F{}", vk - 0x6F),
        _ => {}
    }
    os_key_name(vk).unwrap_or_else(|| format!("Клавиша 0x{vk:02X}"))
}

/// Настоящее имя клавиши от Windows (учитывает раскладку).
fn os_key_name(vk: u16) -> Option<String> {
    unsafe {
        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC);
        if scan == 0 {
            return None;
        }
        // Расширенные клавиши (навигация, правые модификаторы, numlock) требуют бит 0x100.
        let extended = matches!(vk, 0x21..=0x2E | 0xA3 | 0xA5 | 0x90 | 0x6F);
        let lparam = ((scan << 16) | (if extended { 1u32 << 24 } else { 0 })) as i32;
        let mut buf = [0u16; 64];
        let n = GetKeyNameTextW(lparam, &mut buf);
        if n <= 0 {
            return None;
        }
        let s = String::from_utf16_lossy(&buf[..n as usize]).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}
