//! Настройки → «Микрофон и система»: доступ, устройство записи, трей, журнал.

use egui::{Color32, RichText, Rounding, Sense};

use crate::lang::tr;
use crate::app::TvoiceApp;
use crate::mic::wav::timestamp_filename;
use crate::mic::MicCommand;
use crate::theme as t;
use crate::ui_kit as k;

/// Обрезать длинную строку по границе, чтобы она не растягивала карточку.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

impl TvoiceApp {
    pub(crate) fn settings_system(&mut self, ui: &mut egui::Ui) {
        self.language_card(ui);
        ui.add_space(t::MD);
        self.access_card(ui);
        ui.add_space(t::MD);
        self.device_card(ui);
        ui.add_space(t::MD);
        self.tray_card(ui);
        ui.add_space(t::MD);
        self.log_card(ui);
        ui.add_space(t::XL);
    }

    /// Язык интерфейса. Отдельной карточкой и первой в разделе: если человек открыл
    /// настройки, не понимая языка меню, искать переключатель он будет сверху.
    fn language_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::row(
                ui,
                |ui| {
                    k::heading(ui, tr("Язык интерфейса", "Interface language"));
                    k::hint(
                        ui,
                        tr(
                            "Не влияет на язык распознавания — тот выбирается на экране «Диктовка».",
                            "Does not affect the recognition language — that one is on the Dictation screen.",
                        ),
                    );
                },
                |ui| {
                    egui::ComboBox::from_id_source("ui_lang")
                        .selected_text(RichText::new(self.ui_lang.label()).size(t::T_BODY))
                        .show_ui(ui, |ui| {
                            for lang in crate::lang::Lang::ALL {
                                if ui
                                    .selectable_label(self.ui_lang == lang, lang.label())
                                    .clicked()
                                {
                                    self.ui_lang = lang;
                                    // Применяем сразу: egui рисует кадр целиком, поэтому
                                    // перезапуск не нужен — надписи сменятся на этом же кадре.
                                    crate::lang::set(lang);
                                    self.dirty = true;
                                }
                            }
                        });
                },
            );
        });
    }

    fn access_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            let (text, color) = self.permission_chip();
            k::row(
                ui,
                |ui| {
                    k::heading(ui, tr("Доступ к микрофону", "Microphone access"));
                    k::hint(ui, tr("Системное разрешение на запись звука.", "The system permission to record audio."));
                },
                |ui| k::chip(ui, text, color),
            );

            ui.add_space(t::SM);
            ui.horizontal(|ui| {
                if k::ghost_button(ui, tr("Запросить", "Request")).clicked() {
                    self.push_log(tr("→ Запрос доступа (WinRT)…", "→ Requesting access (WinRT)…"));
                    self.engine.send(MicCommand::RequestOfficial);
                }
                if k::ghost_button(ui, tr("Обновить", "Refresh")).clicked() {
                    self.engine.send(MicCommand::RefreshPermission);
                    self.engine.send(MicCommand::EnumerateDevices);
                }
                if let Some(report) = &self.permission {
                    let _ = report;
                    let arrow = if self.show_details { tr("Скрыть", "Hide") } else { tr("Детали", "Details") };
                    if k::ghost_button(ui, arrow).clicked() {
                        self.show_details = !self.show_details;
                    }
                }
            });

            if let Some((ok, detail)) = &self.last_official {
                ui.add_space(t::XS);
                let c = if *ok { t::OK } else { t::BAD };
                ui.label(RichText::new(detail).size(t::T_LABEL).color(c));
            }
            if self.show_details {
                if let Some(report) = &self.permission {
                    ui.add_space(t::XS);
                    for line in &report.details {
                        ui.label(RichText::new(line).size(t::T_LABEL).color(t::MUTED));
                    }
                }
            }
        });
    }

    fn device_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            let selected_label = self
                .devices
                .iter()
                .find(|d| Some(&d.id) == self.selected.as_ref())
                .map(|d| d.name.clone())
                .unwrap_or_else(|| tr("не выбрано", "not selected").to_string());

            k::heading(ui, tr("Устройство записи", "Recording device"));
            k::hint(ui, tr("Микрофон, с которого идёт диктовка.", "The microphone dictation listens to."));
            ui.add_space(t::XS);
            // Список во всю ширину: имена устройств длинные и в строку не помещаются.
            let w = ui.available_width();
            egui::ComboBox::from_id_source("device")
                .selected_text(RichText::new(shorten(&selected_label, 42)).size(t::T_BODY))
                .width(w)
                .show_ui(ui, |ui| {
                    for d in &self.devices {
                        let label = if d.is_default {
                            format!("{} ({})", d.name, tr("по умолчанию", "default"))
                        } else {
                            d.name.clone()
                        };
                        ui.selectable_value(&mut self.selected, Some(d.id.clone()), label);
                    }
                });

            if let Some((dev, fmt)) = &self.capture_info {
                ui.add_space(t::XS);
                k::hint(ui, &format!("{dev} — {fmt}"));
            }

            k::divider(ui);
            k::row(
                ui,
                |ui| {
                    ui.label(RichText::new(tr("Проверить микрофон", "Test the microphone")).size(t::T_BODY));
                    k::hint(ui, tr("Запись без распознавания — убедиться, что звук идёт.", "Recording without recognition — to check that sound arrives."));
                },
                |ui| {
                    if self.capturing {
                        if ui.button(tr("Стоп", "Stop")).clicked() {
                            self.engine.send(MicCommand::StopCapture);
                        }
                    } else if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new(tr("Записать", "Record")))
                        .clicked()
                    {
                        let record = self.record_enabled.then(|| {
                            let dir = Self::recordings_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            let path = dir.join(timestamp_filename());
                            self.last_record_path = Some(path.clone());
                            path
                        });
                        self.engine.send(MicCommand::StartCapture {
                            device: self.selected.clone(),
                            record,
                            live: None,
                        });
                    }
                },
            );

            ui.add_space(t::XS);
            k::bar(ui, self.level, 6.0, t::PRIMARY_STRONG);

            ui.add_space(t::XS);
            if k::switch_row(
                ui,
                &mut self.record_enabled,
                tr("Сохранять запись в .wav", "Save the recording to .wav"),
                tr("Файлы складываются в папку recordings рядом с программой.",
                "Files go to the recordings folder next to the program."),
            ) {
                self.dirty = true;
            }
            if self.last_record_path.is_some() || Self::recordings_dir().exists() {
                ui.add_space(t::BASE);
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(tr("Открыть папку записей", "Open recordings folder"))
                                .size(t::T_LABEL)
                                .color(t::PRIMARY),
                        )
                        .sense(Sense::click()),
                    )
                    .clicked()
                {
                    let _ = std::process::Command::new("explorer")
                        .arg(Self::recordings_dir())
                        .spawn();
                }
            }
        });
    }

    fn tray_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::heading(ui, tr("Работа в фоне", "Background operation"));
            ui.add_space(t::SM);
            k::hint(
                ui,
                tr(
                    "Крестик прячет TVOICE в трей — хоткей продолжает работать. \
                     Правый клик по значку: начать диктовку или выйти.",
                    "The close button hides TVOICE in the tray — the hotkey keeps working. \
                     Right-click the icon to start dictation or quit.",
                ),
            );
            ui.add_space(t::SM);
            if k::switch_row(
                ui,
                &mut self.start_in_tray,
                tr("Запускать свёрнутым в трей", "Start minimised to tray"),
                "",
            ) {
                self.dirty = true;
            }
            if k::switch_row(
                ui,
                &mut self.autostart,
                tr("Запускать при старте Windows", "Start with Windows"),
                tr("Свёрнутым в трей, независимо от настройки выше", "Minimised to tray, regardless of the setting above"),
            ) {
                // Настройка живёт в реестре, а не в config.json, поэтому применяем сразу.
                if let Err(e) = crate::autostart::set(self.autostart) {
                    crate::logln!("автозапуск: не изменить — {e}");
                    self.autostart = !self.autostart; // галочка не должна врать
                }
            }
        });
    }

    fn log_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::row(
                ui,
                |ui| {
                    k::heading(ui, tr("Журнал", "Log"));
                    k::hint(ui, tr("Последние события. Полная запись — в файле tvoice.log.", "Recent events. The full record is in tvoice.log."));
                },
                |ui| {
                    if k::ghost_button(ui, tr("Открыть файл", "Open file")).clicked() {
                        let _ = std::process::Command::new("explorer")
                            .arg(crate::models::app_dir().join("tvoice.log"))
                            .spawn();
                    }
                },
            );
            ui.add_space(t::XS);
            egui::Frame::none()
                .fill(Color32::from_rgb(0x0D, 0x0F, 0x0E))
                .rounding(Rounding::same(t::R_SM))
                .inner_margin(egui::Margin::same(t::XS))
                .show(ui, |ui| {
                    ui.set_min_height(90.0);
                    egui::ScrollArea::vertical()
                        .id_source("log")
                        .max_height(140.0)
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for line in &self.log {
                                ui.label(
                                    RichText::new(line)
                                        .size(t::T_LABEL_SM)
                                        .monospace()
                                        .color(t::MUTED),
                                );
                            }
                        });
                });
        });
    }
}
