//! Управление диктовкой: обработка событий движка, хоткея и старт/стоп распознавания.
//! Вынесено из app.rs, чтобы файлы оставались небольшими.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::app::TvoiceApp;
use crate::dictation;
use crate::hotkey::HotkeyEvent;
use crate::mic::{LiveCapture, MicCommand, MicEvent};
use crate::models;

impl TvoiceApp {
    pub(crate) fn handle_events(&mut self) {
        for ev in self.engine.drain_events() {
            match ev {
                MicEvent::Log(s) => self.push_log(s),
                MicEvent::Permission(report) => self.permission = Some(report),
                MicEvent::Devices(devs) => {
                    if self.selected.is_none() {
                        self.selected = devs
                            .iter()
                            .find(|d| d.is_default)
                            .or_else(|| devs.first())
                            .map(|d| d.id.clone());
                    }
                    self.devices = devs;
                }
                MicEvent::AccessResult { ok, detail } => {
                    self.push_log(format!("[WinRT] {detail}"));
                    self.last_official = Some((ok, detail));
                }
                MicEvent::CaptureStarted { device, format } => {
                    self.capturing = true;
                    self.push_log(format!("Захват: {device} ({format})"));
                    self.capture_info = Some((device, format));
                }
                MicEvent::CaptureStopped => {
                    self.capturing = false;
                    self.capture_info = None;
                    self.level = 0.0;
                    // Батч-диктовка: по остановке распознаём и вставляем.
                    if let Some(temp) = self.dictation_temp.take() {
                        if let Some(file) = self
                            .selected_model
                            .as_ref()
                            .and_then(|id| models::by_id(id))
                            .map(|m| m.file.to_string())
                        {
                            dictation::transcribe_and_insert(
                                temp,
                                file,
                                self.lang.clone(),
                                self.dictation_insert,
                                self.dictation.clone(),
                                self.ctx.clone(),
                            );
                        }
                    } else {
                        self.push_log("Захват остановлен.");
                    }
                }
                MicEvent::Error(e) => self.push_log(format!("Ошибка: {e}")),
            }
        }
    }

    pub(crate) fn handle_hotkey(&mut self) {
        for ev in self.hotkey.drain() {
            if self.streaming_enabled {
                // Поток — режим-переключатель: нажал начал, нажал закончил
                // (удержание не годится: клавиши хоткея «протекают» в целевое окно).
                if ev == HotkeyEvent::Pressed {
                    if self.dictating {
                        self.stop_dictation();
                    } else {
                        self.begin_dictation(self.insert_enabled);
                    }
                }
            } else {
                // Батч — push-to-talk: держишь пишет, отпустил вставилось.
                match ev {
                    HotkeyEvent::Pressed => self.begin_dictation(self.insert_enabled),
                    HotkeyEvent::Released => self.stop_dictation(),
                }
            }
        }
    }

    pub(crate) fn begin_dictation(&mut self, insert: bool) {
        if self.capturing || self.dictating {
            return;
        }
        crate::logln!("hotkey/кнопка: старт диктовки (insert={insert})");
        self.dictation_insert = insert;
        let err = |app: &Self, msg: &str| {
            if let Ok(mut s) = app.dictation.lock() {
                s.error = Some(msg.to_string());
                s.state = format!("Ошибка: {msg}");
                s.busy = false;
            }
        };
        if models::whisper_exe().is_none() {
            err(self, "whisper.cpp не установлен — вкладка «Модели»");
            return;
        }
        let Some(info) = self.selected_model.as_ref().and_then(|id| models::by_id(id)) else {
            err(self, "Выберите модель во вкладке «Модели»");
            return;
        };
        if !models::is_downloaded(info.file) {
            err(self, "Модель не скачана");
            return;
        }
        let model_file = info.file.to_string();
        if insert {
            // Запоминаем окно СЕЙЧАС: вставлять начнём ещё во время речи, и фокус к тому
            // моменту может оказаться где угодно.
            crate::inject::remember_target();
            // Наблюдение за живым вводом: по нему поток решает, можно ли править
            // уже вставленный черновик. Клавишу хоткея вмешательством не считаем.
            crate::userinput::watch();
            if let Ok(h) = self.hotkey.config.lock() {
                crate::userinput::ignore_vk(h.vk);
            }
        }

        if self.streaming_enabled {
            // Потоковый режим: живой буфер + фоновый распознаватель.
            let live = LiveCapture {
                buf: Arc::new(Mutex::new(Vec::new())),
                rate: Arc::new(AtomicU32::new(16_000)),
            };
            let stop = Arc::new(AtomicBool::new(false));
            self.engine.send(MicCommand::StartCapture {
                device: self.selected.clone(),
                record: None,
                live: Some(live.clone()),
            });
            crate::streaming::run(
                live,
                stop.clone(),
                model_file,
                self.lang.clone(),
                insert,
                self.dictation.clone(),
                self.ctx.clone(),
            );
            self.stream_stop = Some(stop);
            self.dictating = true;
            if let Ok(mut s) = self.dictation.lock() {
                s.busy = true;
                s.error = None;
                s.state = "Слушаю… (говорите)".into();
                s.last_text.clear();
            }
        } else {
            // Батч: пишем во временный WAV, распознаём по отпусканию.
            let temp_dir = models::app_dir().join("temp");
            let _ = std::fs::create_dir_all(&temp_dir);
            let temp = temp_dir.join("tvoice_dictation.wav");
            self.engine.send(MicCommand::StartCapture {
                device: self.selected.clone(),
                record: Some(temp.clone()),
                live: None,
            });
            self.dictating = true;
            self.dictation_temp = Some(temp);
            if let Ok(mut s) = self.dictation.lock() {
                s.busy = true;
                s.error = None;
                s.state = "Запись… (говорите)".into();
            }
        }
    }

    pub(crate) fn stop_dictation(&mut self) {
        if !self.dictating {
            return;
        }
        self.dictating = false;
        if let Some(stop) = self.stream_stop.take() {
            crate::logln!("стоп потоковой диктовки");
            stop.store(true, Ordering::Relaxed); // поток сам зафиксирует хвост
            self.engine.send(MicCommand::StopCapture);
        } else {
            crate::logln!("стоп диктовки → распознавание");
            self.engine.send(MicCommand::StopCapture);
        }
    }
}
