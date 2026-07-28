//! Конвейер диктовки: записанный WAV → 16 кГц → распознавание → вставка в курсор.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::{audio, inject};

/// Разделяемый статус диктовки для отображения в UI.
#[derive(Default, Clone)]
pub struct DictationStatus {
    pub busy: bool,
    pub state: String,
    pub last_text: String,
    pub error: Option<String>,
}

pub type SharedDictation = Arc<Mutex<DictationStatus>>;

pub fn new_shared() -> SharedDictation {
    Arc::new(Mutex::new(DictationStatus::default()))
}

/// Асинхронно: привести к 16 кГц, распознать выбранной моделью и напечатать в активное окно.
pub fn transcribe_and_insert(
    recorded_wav: PathBuf,
    model_file: String,
    lang: String,
    insert: bool,
    status: SharedDictation,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        crate::logln!(
            "dictation: старт конвейера (insert={insert}, model={model_file}, lang={lang})"
        );
        set(&status, &ctx, |s| {
            s.busy = true;
            s.error = None;
            s.state = "Готовлю аудио…".into();
        });

        let wav16 = recorded_wav.with_file_name("tvoice_dictation_16k.wav");
        if let Err(e) = audio::wav_to_16k_mono(&recorded_wav, &wav16) {
            crate::logln!("dictation: ресемпл ОШИБКА: {e}");
            fail(&status, &ctx, format!("Аудио: {e}"));
            let _ = std::fs::remove_file(&recorded_wav);
            return;
        }
        crate::logln!("dictation: ресемпл ок → 16кГц");

        set(&status, &ctx, |s| s.state = "Распознаю…".into());

        let result = crate::server::transcribe(&wav16, &model_file, &lang);
        let _ = std::fs::remove_file(&recorded_wav);
        let _ = std::fs::remove_file(&wav16);

        match result {
            Ok(text) if !text.is_empty() => {
                crate::logln!("dictation: распознано ({} симв.): {:?}", text.chars().count(), text);
                if insert {
                    crate::logln!("inject: старт (len={})", text.len());
                    let text_for_inject = text.clone();
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        inject::type_text(&text_for_inject)
                    }));
                    match r {
                        Ok(()) => crate::logln!("inject: готово"),
                        Err(_) => {
                            crate::logln!("inject: ПАНИКА поймана (см. строку PANIC выше)");
                            fail(&status, &ctx, "сбой вставки (см. tvoice.log)".into());
                            return;
                        }
                    }
                }
                set(&status, &ctx, |s| {
                    s.busy = false;
                    s.last_text = text;
                    s.state = if insert { "Вставлено ✓".into() } else { "Готово ✓".into() };
                });
            }
            Ok(_) => {
                crate::logln!("dictation: пусто");
                set(&status, &ctx, |s| {
                    s.busy = false;
                    s.state = "Пусто (речь не распознана)".into();
                });
            }
            Err(e) => {
                crate::logln!("dictation: транскрибация ОШИБКА: {e}");
                fail(&status, &ctx, e.to_string());
            }
        }
    });
}

fn set(status: &SharedDictation, ctx: &egui::Context, f: impl FnOnce(&mut DictationStatus)) {
    if let Ok(mut s) = status.lock() {
        f(&mut s);
    }
    ctx.request_repaint();
}

fn fail(status: &SharedDictation, ctx: &egui::Context, msg: String) {
    set(status, ctx, |s| {
        s.busy = false;
        s.error = Some(msg.clone());
        s.state = format!("Ошибка: {msg}");
    });
}
