//! Проверка доступа к микрофону — слой 1.
//!
//! На Windows 11 доступ гейтится тремя уровнями тумблеров приватности,
//! которые хранятся в реестре (CapabilityAccessManager\ConsentStore\microphone):
//!   * машинный    (HKLM)  — «Microphone access» на уровне устройства;
//!   * пользователя (HKCU) — общий тумблер микрофона для аккаунта;
//!   * NonPackaged (HKCU)  — «Let desktop apps access your microphone» (именно наш случай, .exe).
//!
//! Плюс мы спрашиваем официальный AppCapability API — он агрегирует состояние сам.

use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
use winreg::RegKey;

const CONSENT_MIC: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// Итоговое состояние доступа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    /// Доступ разрешён.
    Allowed,
    /// Доступ запрещён (пользователем или системой).
    Denied,
    /// Требуется системный запрос (пользователь ещё не решал).
    PromptRequired,
    /// Не удалось однозначно определить.
    Unknown,
}

impl PermissionState {
    pub fn label(self) -> &'static str {
        match self {
            PermissionState::Allowed => "Разрешён",
            PermissionState::Denied => "Запрещён",
            PermissionState::PromptRequired => "Требуется запрос",
            PermissionState::Unknown => "Неизвестно",
        }
    }
}

/// Полный отчёт для отображения в UI.
/// Отдельные поля-тумблеры доступны для будущей детализации в UI/логах.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct PermissionReport {
    pub effective: PermissionState,
    /// Результат официального AppCapability.CheckAccess().
    pub app_capability: PermissionState,
    /// HKLM ConsentStore\microphone\Value == "Allow".
    pub machine_allow: Option<bool>,
    /// HKCU ConsentStore\microphone\Value == "Allow".
    pub user_allow: Option<bool>,
    /// HKCU ConsentStore\microphone\NonPackaged\Value == "Allow".
    pub nonpackaged_allow: Option<bool>,
    pub details: Vec<String>,
}

/// Прочитать строковое значение "Value" из ветки ConsentStore и привести к bool (Allow => true).
fn read_allow(root: RegKey, subkey: &str) -> Option<bool> {
    let key = root.open_subkey_with_flags(subkey, KEY_READ).ok()?;
    let val: String = key.get_value("Value").ok()?;
    Some(val.eq_ignore_ascii_case("Allow"))
}

/// Собрать полный отчёт о доступе.
pub fn report() -> PermissionReport {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let machine_allow = read_allow(hklm, CONSENT_MIC);
    let user_allow = read_allow(RegKey::predef(HKEY_CURRENT_USER), CONSENT_MIC);
    let nonpackaged_allow = read_allow(hkcu, &format!(r"{CONSENT_MIC}\NonPackaged"));

    let app_capability = query_app_capability();

    let mut details = Vec::new();
    details.push(format!(
        "AppCapability API: {}",
        app_capability.label()
    ));
    details.push(format!(
        "Машинный тумблер (HKLM): {}",
        fmt_allow(machine_allow)
    ));
    details.push(format!("Тумблер пользователя (HKCU): {}", fmt_allow(user_allow)));
    details.push(format!(
        "Десктоп-приложения (NonPackaged): {}",
        fmt_allow(nonpackaged_allow)
    ));

    // Итог: любой запрещающий тумблер перекрывает всё.
    let effective = if machine_allow == Some(false) {
        details.push("→ Запрещено на уровне системы.".into());
        PermissionState::Denied
    } else if user_allow == Some(false) {
        details.push("→ Пользователь отключил микрофон для аккаунта.".into());
        PermissionState::Denied
    } else if nonpackaged_allow == Some(false) {
        details.push("→ Отключён доступ для десктопных приложений.".into());
        PermissionState::Denied
    } else if app_capability == PermissionState::Allowed
        || (user_allow == Some(true) && nonpackaged_allow != Some(false))
    {
        PermissionState::Allowed
    } else if app_capability == PermissionState::PromptRequired
        || (machine_allow.is_none() && user_allow.is_none())
    {
        PermissionState::PromptRequired
    } else {
        app_capability
    };

    PermissionReport {
        effective,
        app_capability,
        machine_allow,
        user_allow,
        nonpackaged_allow,
        details,
    }
}

fn fmt_allow(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "Allow",
        Some(false) => "Deny",
        None => "нет данных",
    }
}

/// Официальный запрос статуса без вызова диалога — Windows.Security…AppCapability.
fn query_app_capability() -> PermissionState {
    use windows::core::HSTRING;
    use windows::Security::Authorization::AppCapabilityAccess::{
        AppCapability, AppCapabilityAccessStatus,
    };

    let cap = match AppCapability::Create(&HSTRING::from("microphone")) {
        Ok(c) => c,
        Err(_) => return PermissionState::Unknown,
    };
    match cap.CheckAccess() {
        Ok(AppCapabilityAccessStatus::Allowed) => PermissionState::Allowed,
        Ok(AppCapabilityAccessStatus::UserPromptRequired) => PermissionState::PromptRequired,
        Ok(AppCapabilityAccessStatus::DeniedByUser)
        | Ok(AppCapabilityAccessStatus::DeniedBySystem) => PermissionState::Denied,
        Ok(_) => PermissionState::Unknown,
        Err(_) => PermissionState::Unknown,
    }
}
