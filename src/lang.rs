//! Язык интерфейса: русский или английский.
//!
//! Строки хранятся не в таблице с ключами, а парами прямо в месте использования:
//! `tr("Диктовка", "Dictation")`. Причина — языков ровно два и добавлять третий не
//! планируется, а таблица ключей на полторы сотни строк дала бы вечную возможность
//! опечататься в ключе или забыть перевод: и то, и другое проявилось бы только на экране.
//! При парах перевод виден рядом с кодом, а пропустить его нельзя — не скомпилируется.
//!
//! Текущий язык лежит в атомике, а не передаётся аргументом: интерфейс `egui` перерисовывает
//! кадр целиком, поэтому переключение применяется сразу, без пересоздания окна.
//!
//! Не путать с языком РАСПОЗНАВАНИЯ (`Config::lang`) — тот про то, на каком языке говорят,
//! и меняется отдельно.

use std::sync::atomic::{AtomicU8, Ordering};

/// Язык интерфейса.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    /// Код для `config.json`.
    pub fn id(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    /// Название на самом этом языке — так в списках принято, и так понятнее тому,
    /// кто не читает текущий язык интерфейса.
    pub fn label(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "ru" => Some(Lang::Ru),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    pub const ALL: [Lang; 2] = [Lang::Ru, Lang::En];
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Установить язык интерфейса.
pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Ru
    }
}

/// Строка интерфейса на текущем языке.
pub fn tr(ru: &'static str, en: &'static str) -> &'static str {
    match current() {
        Lang::Ru => ru,
        Lang::En => en,
    }
}

/// Язык интерфейса Windows, если он нам известен, — им и открываемся при первом запуске.
///
/// Ставить русский всем по умолчанию неправильно: программу скачает и англоязычный
/// пользователь, а меню на незнакомом языке он не разберёт настолько, чтобы найти в нём
/// переключатель языка.
pub fn from_system() -> Lang {
    // Младшие 10 бит LANGID — основной язык; 0x19 — русский.
    const LANG_RUSSIAN: u16 = 0x19;
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if id & 0x3FF == LANG_RUSSIAN {
        Lang::Ru
    } else {
        Lang::En
    }
}
