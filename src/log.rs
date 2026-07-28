//! Простой файловый лог + перехват паник. Файл `tvoice.log` рядом с .exe.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

static LOCK: Mutex<()> = Mutex::new(());

fn log_path() -> PathBuf {
    crate::models::app_dir().join("tvoice.log")
}

fn now() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// Записать строку в лог (и продублировать в stderr для debug-консоли).
pub fn line(msg: &str) {
    let entry = format!("[{}] {}\n", now(), msg);
    eprint!("{entry}");
    let _g = LOCK.lock();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = f.write_all(entry.as_bytes());
    }
}

/// Установить перехватчик паник, пишущий в лог (срабатывает и при panic=abort).
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "?".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "—".to_string());
        line(&format!("!!! PANIC в {loc}: {payload}"));
        prev(info);
    }));
}

/// Удобный макрос: `logln!("текст {}", x)`.
#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}
