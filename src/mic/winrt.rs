//! Официальный запрос доступа к микрофону — слой 2.
//!
//! `Windows.Media.Capture.MediaCapture` — рекомендованный Microsoft API. На неупакованном
//! десктопном .exe (Windows 10 1903+) первый вызов `InitializeAsync` показывает системный
//! диалог согласия, а далее уважает сохранённый выбор пользователя. Если доступ запрещён,
//! инициализация падает с UnauthorizedAccessException / E_ACCESSDENIED — это мы и ловим.
//!
//! Вызывается из фонового потока, где уже инициализирован COM в режиме MTA.

use anyhow::Result;
use windows::Media::Capture::{
    MediaCapture, MediaCaptureInitializationSettings, StreamingCaptureMode,
};

/// Пытается получить доступ официальным путём.
/// Возвращает `(доступ_получен, человекочитаемая_деталь)`.
pub fn request_microphone_access() -> Result<(bool, String)> {
    let settings = MediaCaptureInitializationSettings::new()?;
    // Нас интересует только аудио — не трогаем камеру.
    settings.SetStreamingCaptureMode(StreamingCaptureMode::Audio)?;

    let capture = MediaCapture::new()?;

    // Асинхронный вызов блокируем в этом потоке — мы и так в фоне.
    let result = capture.InitializeWithSettingsAsync(&settings)?.get();

    match result {
        Ok(()) => {
            // Освобождаем устройство сразу — доступ подтверждён.
            let _ = capture.Close();
            Ok((
                true,
                "Доступ подтверждён официальным API (WinRT MediaCapture).".to_string(),
            ))
        }
        Err(e) => {
            let code = e.code();
            let msg = e.message();
            Ok((
                false,
                format!("Отказ WinRT: 0x{:08X} — {msg}", code.0 as u32),
            ))
        }
    }
}
