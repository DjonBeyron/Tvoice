//! Управление диктовкой: обработка событий движка, хоткея и старт/стоп распознавания.
//! Вынесено из app.rs, чтобы файлы оставались небольшими.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::app::TvoiceApp;
use crate::dictation;
use crate::hotkey::HotkeyEvent;
use crate::mic::{LiveCapture, MicCommand, MicEvent};
use crate::tray::TrayEvent;
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
                s.auto_stop = false; // от прошлого сеанса флаг остаться не должен
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
        // Единственная воронка выхода из захвата: сюда приходят и повторное нажатие, и
        // отпускание в батч-режиме, и меню трея, и остановка по молчанию.
        crate::sound::play_exit();
        if let Some(stop) = self.stream_stop.take() {
            crate::logln!("стоп потоковой диктовки");
            stop.store(true, Ordering::Relaxed); // поток сам зафиксирует хвост
            self.engine.send(MicCommand::StopCapture);
        } else {
            crate::logln!("стоп диктовки → распознавание");
            self.engine.send(MicCommand::StopCapture);
        }
    }

    /// Меню трея, закрытие окна в трей и восстановление из трея.
    ///
    /// Хоткей опрашивается отдельным потоком и работает со спрятанным окном; чтобы
    /// приложение не «засыпало» без событий окна, просим перерисовку по таймеру.
    pub(crate) fn handle_tray(&mut self, ctx: &egui::Context) {
        if self.first_frame {
            self.first_frame = false;
            crate::tray::set_window_icon();
            // Запуск сразу в трей: окно уже стоит за экраном, осталось убрать его кнопку.
            if self.hidden {
                crate::tray::set_taskbar(false);
            }
        }
        // Запоминаем положение окна, пока оно на виду, — вернём туда же.
        if !self.hidden {
            if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
                self.window_pos = rect.min;
            }
        }
        for ev in self.tray.drain() {
            match ev {
                TrayEvent::Show => self.show_window(ctx),
                TrayEvent::Dictate => {
                    if self.dictating {
                        self.stop_dictation();
                    } else {
                        self.begin_dictation(self.insert_enabled);
                    }
                }
                TrayEvent::Quit => {
                    crate::logln!("выход по команде из трея");
                    self.quitting = true;
                    if self.dictating {
                        self.stop_dictation();
                    }
                    self.persist();
                    crate::server::shutdown();
                    // Окно может быть спрятано — Close по невидимому окну не сработает.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        self.tray.set_dictating(self.dictating);

        // Крестик не закрывает приложение, а прячет его в трей — иначе диктовка
        // по глобальному хоткею умирала бы вместе с окном. Но выход из трея —
        // это настоящий выход, его перехватывать нельзя.
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.quitting {
                crate::logln!("окно закрывается, приложение завершается");
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide_window(ctx);
            }
        }
    }

    /// Не дать циклу UI заснуть: хоткей и меню трея обрабатываются в кадре, а со
    /// спрятанным окном системных событий нет. Заодно видим в логе, если цикл встал.
    pub(crate) fn keep_alive(&mut self, ctx: &egui::Context) {
        let gap = self.last_frame.elapsed();
        self.last_frame = std::time::Instant::now();
        if self.hidden && !self.quitting && gap > std::time::Duration::from_secs(1) {
            crate::logln!(
                "трей: кадров не было {:.1}с — цикл засыпал (хоткей/меню могли не отвечать)",
                gap.as_secs_f32()
            );
        }
        if self.dictating && !self.hidden {
            // Визуализатор должен идти плавно, а не десятью кадрами в секунду.
            ctx.request_repaint();
        } else {
            // Свёрнуто в трей или простой — экран никто не видит, кадры не нужны.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub(crate) fn hide_window(&mut self, ctx: &egui::Context) {
        if self.hidden {
            return;
        }
        self.hidden = true;
        crate::logln!("окно свёрнуто в трей");
        // Окно уезжает за экран, а не прячется: см. tray::set_taskbar — по-настоящему
        // спрятанное (или свёрнутое) окно останавливает цикл eframe, и вместе с ним
        // перестают работать хоткей и меню трея.
        crate::tray::set_taskbar(false);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            -32000.0, -32000.0,
        )));
    }

    pub(crate) fn show_window(&mut self, ctx: &egui::Context) {
        if !self.hidden {
            return;
        }
        self.hidden = false;
        crate::logln!("окно восстановлено из трея");
        crate::tray::set_taskbar(true);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(self.window_pos));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}
