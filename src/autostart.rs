//! Запуск вместе с Windows.
//!
//! Значение в `HKCU\...\Run` — обычный пользовательский автозапуск: прав администратора не
//! требует, а запись видна во вкладке «Автозагрузка» диспетчера задач, где её можно
//! отключить помимо нас. Поэтому источник истины — сам реестр, а не `config.json`: иначе
//! галочка в настройках расходилась бы с тем, что реально сделает система.

use anyhow::{Context, Result};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "TVOICE";

/// Команда автозапуска: текущий .exe плюс `--tray`.
///
/// Флаг обязателен. При старте системы окно не должно выпрыгивать на экран, а отдельная
/// настройка «запускать свёрнутым» относится к ручному запуску и может быть выключена.
fn command() -> Result<String> {
    let exe = std::env::current_exe().context("не узнать путь к .exe")?;
    Ok(format!("\"{}\" --tray", exe.display()))
}

/// Записан ли автозапуск.
///
/// Проверяем наличие значения, а не совпадение пути: если .exe переехал, честнее показать
/// галочку включённой (запись-то есть) и сказать о расхождении в лог, чем показать
/// выключенной — тогда пользователь не понял бы, почему программа всё равно стартует.
pub fn is_enabled() -> bool {
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ)
    else {
        return false;
    };
    let Ok(have) = key.get_value::<String, _>(VALUE) else {
        return false;
    };
    if let Ok(want) = command() {
        if !have.eq_ignore_ascii_case(&want) {
            crate::logln!("автозапуск: в реестре другой путь ({have}) — перепишется при переключении");
        }
    }
    true
}

/// Включить или выключить автозапуск.
pub fn set(enabled: bool) -> Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey_with_flags(RUN_KEY, KEY_WRITE)
        .context("не открыть ветку автозапуска")?;
    if enabled {
        let cmd = command()?;
        key.set_value(VALUE, &cmd).context("не записать значение")?;
        crate::logln!("автозапуск: включён — {cmd}");
    } else {
        match key.delete_value(VALUE) {
            Ok(()) => crate::logln!("автозапуск: выключен"),
            // Значения могло и не быть — это не ошибка.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("не удалить значение"),
        }
    }
    Ok(())
}
