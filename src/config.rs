//! Сохранение настроек между запусками: `config.json` рядом с .exe.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hotkey::HotkeyConfig;

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u16,
    pub key_name: String,
    pub model: Option<String>,
    pub lang: String,
    pub insert: bool,
    pub streaming: bool,
    /// Способ вставки: 0 — авто, 1 — клавиатура, 2 — буфер обмена (см. `inject`).
    pub paste_mode: u8,
    /// Пауза между символами при печати, мкс. Правится только руками в config.json:
    /// если какое-то приложение теряет/дублирует символы — увеличьте (8000 → 15000).
    pub char_delay_us: u32,
}

impl Default for Config {
    fn default() -> Self {
        let h = HotkeyConfig::default();
        Self {
            ctrl: h.ctrl,
            alt: h.alt,
            shift: h.shift,
            win: h.win,
            vk: h.vk,
            key_name: h.key_name,
            model: None,
            lang: "ru".to_string(),
            insert: true,
            streaming: true,
            paste_mode: crate::inject::MODE_AUTO,
            char_delay_us: crate::inject::DEFAULT_CHAR_DELAY_US,
        }
    }
}

impl Config {
    pub fn hotkey(&self) -> HotkeyConfig {
        HotkeyConfig {
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
            win: self.win,
            vk: self.vk,
            key_name: self.key_name.clone(),
        }
    }
}

fn path() -> PathBuf {
    crate::models::app_dir().join("config.json")
}

pub fn load() -> Config {
    match std::fs::read_to_string(path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) {
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path(), s);
    }
}
