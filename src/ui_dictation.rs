//! Вкладка «Диктовка»: хоткей, язык, вставка в курсор, статус и результат.

use egui::{Color32, RichText, Rounding, Vec2};

use crate::app::{card, TvoiceApp};
use crate::models;
use crate::theme;

/// Языки для whisper (код + подпись). "auto" — автоопределение.
const LANGS: &[(&str, &str)] = &[
    ("ru", "Русский"),
    ("en", "English"),
    ("uk", "Українська"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("auto", "Авто"),
];

impl TvoiceApp {
    pub(crate) fn dictation_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.dictation_howto(ui);
            ui.add_space(6.0);
            self.hotkey_editor(ui);
            ui.add_space(6.0);
            self.dictation_settings(ui);
            ui.add_space(6.0);
            self.dictation_status_card(ui);
            ui.add_space(6.0);
            self.tray_settings(ui);
        });
    }

    /// Работа в фоне: значок в трее и запуск свёрнутым.
    fn tray_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Фоновый режим").size(15.0).strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Закрытие окна крестиком прячет TVOICE в трей — хоткей продолжает работать. \
                     Правый клик по значку: начать диктовку или выйти.",
                )
                .size(11.0)
                .color(theme::MUTED),
            );
            ui.add_space(6.0);
            if ui
                .checkbox(&mut self.start_in_tray, "Запускать свёрнутым в трей")
                .changed()
            {
                self.dirty = true;
            }
            ui.add_space(6.0);
            let btn = egui::Button::new(RichText::new("▾ Свернуть в трей").size(13.0))
                .min_size(Vec2::new(0.0, 28.0));
            if ui.add(btn).clicked() {
                let ctx = self.ctx.clone();
                self.hide_window(&ctx);
            }
        });
    }

    fn dictation_howto(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Голосовой ввод").size(15.0).strong());
            ui.add_space(4.0);
            let hint = if self.streaming_enabled {
                format!(
                    "Нажмите {} и говорите — текст появляется на лету; нажмите ещё раз, чтобы закончить.",
                    self.hotkey.label()
                )
            } else {
                format!(
                    "Удерживайте {}, говорите, отпустите — текст вставится туда, где курсор.",
                    self.hotkey.label()
                )
            };
            ui.label(RichText::new(hint).size(13.0).color(theme::MUTED));
            if self.dictating {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    crate::app::status_dot(ui, theme::BAD);
                    ui.label(RichText::new("● запись…").strong().color(theme::BAD));
                });
            }
        });
    }

    /// Редактор горячей клавиши: модификаторы + основная клавиша.
    fn hotkey_editor(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let capturing = self.hotkey.is_capturing();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Горячая клавиша").size(13.0).color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if capturing {
                        ui.label(RichText::new("нажмите комбинацию…").strong().color(theme::WARN));
                    } else {
                        ui.label(RichText::new(self.hotkey.label()).strong().color(theme::ACCENT));
                    }
                });
            });

            ui.add_space(8.0);
            if capturing {
                ui.label(
                    RichText::new("Зажмите нужные клавиши (можно с модификаторами) или боковую кнопку мыши…")
                        .size(12.0)
                        .color(theme::MUTED),
                );
                ui.add_space(4.0);
                if ui.button("Отмена").clicked() {
                    self.hotkey.cancel_capture();
                }
            } else {
                let btn = egui::Button::new(
                    RichText::new("Изменить — зажмите клавиши").strong().color(Color32::BLACK),
                )
                .fill(theme::ACCENT)
                .min_size(Vec2::new(0.0, 30.0));
                if ui.add(btn).clicked() {
                    self.hotkey.start_capture();
                }
                ui.add_space(2.0);
                ui.label(
                    RichText::new("Поддерживаются любые клавиши и боковые кнопки мыши (X1/X2).")
                        .size(11.0)
                        .color(theme::MUTED),
                );
                if self.hotkey.is_risky() {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("⚠ Обычная клавиша без модификатора будет срабатывать при наборе текста.")
                            .size(11.0)
                            .color(theme::WARN),
                    );
                }
            }
        });
    }

    fn dictation_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            // Активная модель.
            ui.horizontal(|ui| {
                ui.label(RichText::new("Модель").size(13.0).color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match &self.selected_model {
                        Some(id) => ui.label(RichText::new(id).strong().color(theme::ACCENT)),
                        None => ui.label(RichText::new("не выбрана").color(theme::BAD)),
                    };
                });
            });
            if self.selected_model.is_none() || models::whisper_exe().is_none() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("→ Откройте вкладку «Модели»: скачайте whisper.cpp и модель.")
                        .size(12.0)
                        .color(theme::WARN),
                );
            }

            ui.add_space(8.0);
            // Язык.
            ui.horizontal(|ui| {
                ui.label(RichText::new("Язык").size(13.0).color(theme::MUTED));
                let current = LANGS
                    .iter()
                    .find(|(c, _)| *c == self.lang)
                    .map(|(_, n)| *n)
                    .unwrap_or("Русский");
                egui::ComboBox::from_id_source("lang_combo")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (code, name) in LANGS {
                            if ui
                                .selectable_value(&mut self.lang, code.to_string(), *name)
                                .clicked()
                            {
                                self.dirty = true;
                            }
                        }
                    });
            });

            ui.add_space(6.0);
            if ui
                .checkbox(
                    &mut self.streaming_enabled,
                    "Потоковый режим (как на iPhone): текст на лету",
                )
                .changed()
            {
                self.dirty = true;
            }
            ui.label(
                RichText::new(
                    "В потоковом режиме хоткей работает как переключатель: нажал — говоришь — нажал.",
                )
                .size(11.0)
                .color(theme::MUTED),
            );

            ui.add_space(6.0);
            if ui
                .checkbox(&mut self.insert_enabled, "Вставлять текст в позицию курсора")
                .changed()
            {
                self.dirty = true;
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Способ вставки").size(13.0).color(theme::MUTED));
                egui::ComboBox::from_id_source("paste_mode_combo")
                    .selected_text(crate::inject::mode_name(self.paste_mode))
                    .show_ui(ui, |ui| {
                        for mode in [
                            crate::inject::MODE_AUTO,
                            crate::inject::MODE_KEYS,
                            crate::inject::MODE_CLIPBOARD,
                        ] {
                            if ui
                                .selectable_value(
                                    &mut self.paste_mode,
                                    mode,
                                    crate::inject::mode_name(mode),
                                )
                                .clicked()
                            {
                                crate::inject::set_mode(self.paste_mode);
                                self.dirty = true;
                            }
                        }
                    });
            });
            ui.label(
                RichText::new(
                    "«Авто» вставляет через буфер обмена (прежнее содержимое возвращается) — \
                     это единственный способ, надёжный в приложениях на WinUI. «Клавиатура» \
                     не трогает буфер и нужна там, где Ctrl+V занят (терминалы), но может \
                     изредка путать символы.",
                )
                .size(11.0)
                .color(theme::MUTED),
            );

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(
                RichText::new("Проверка без вставки (результат появится ниже):")
                    .size(12.0)
                    .color(theme::MUTED),
            );
            ui.add_space(4.0);
            if !self.dictating {
                let btn = egui::Button::new(
                    RichText::new("▶ Тест: записать").strong().color(Color32::BLACK),
                )
                .fill(theme::OK)
                .min_size(Vec2::new(0.0, 30.0));
                if ui.add(btn).clicked() {
                    self.begin_dictation(false);
                }
            } else {
                let btn = egui::Button::new(RichText::new("■ Стоп и распознать").strong())
                    .fill(theme::BAD)
                    .min_size(Vec2::new(0.0, 30.0));
                if ui.add(btn).clicked() {
                    self.stop_dictation();
                }
            }
        });
    }

    fn dictation_status_card(&mut self, ui: &mut egui::Ui) {
        let (state, last_text, error, busy) = {
            let d = self.dictation.lock().unwrap();
            (d.state.clone(), d.last_text.clone(), d.error.clone(), d.busy)
        };
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Статус").size(13.0).color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let col = if error.is_some() {
                        theme::BAD
                    } else if busy {
                        theme::WARN
                    } else {
                        theme::MUTED
                    };
                    let txt = if state.is_empty() { "ожидание".to_string() } else { state };
                    ui.label(RichText::new(txt).color(col));
                });
            });

            ui.add_space(8.0);
            ui.label(RichText::new("Последний распознанный текст").size(12.0).color(theme::MUTED));
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(Color32::from_rgb(0x0D, 0x0F, 0x12))
                .rounding(Rounding::same(8.0))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_min_height(80.0);
                    ui.set_width(ui.available_width());
                    if last_text.is_empty() {
                        ui.label(RichText::new("—").color(theme::MUTED));
                    } else {
                        ui.label(RichText::new(&last_text).size(14.0));
                    }
                });

            if !last_text.is_empty() {
                ui.add_space(6.0);
                if ui
                    .add(egui::Button::new("Копировать").min_size(Vec2::new(0.0, 28.0)))
                    .clicked()
                {
                    ui.output_mut(|o| o.copied_text = last_text.clone());
                }
            }
        });
    }
}
