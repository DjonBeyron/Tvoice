//! Настройки → «Микрофон и система»: доступ, устройство записи, трей, журнал.

use egui::{Color32, RichText, Rounding, Sense};

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
        self.access_card(ui);
        ui.add_space(t::MD);
        self.device_card(ui);
        ui.add_space(t::MD);
        self.tray_card(ui);
        ui.add_space(t::MD);
        self.log_card(ui);
        ui.add_space(t::XL);
    }

    fn access_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            let (text, color) = self.permission_chip();
            k::row(
                ui,
                |ui| {
                    k::heading(ui, "Доступ к микрофону");
                    k::hint(ui, "Системное разрешение на запись звука.");
                },
                |ui| k::chip(ui, text, color),
            );

            ui.add_space(t::SM);
            ui.horizontal(|ui| {
                if k::ghost_button(ui, "Запросить").clicked() {
                    self.push_log("→ Запрос доступа (WinRT)…");
                    self.engine.send(MicCommand::RequestOfficial);
                }
                if k::ghost_button(ui, "Обновить").clicked() {
                    self.engine.send(MicCommand::RefreshPermission);
                    self.engine.send(MicCommand::EnumerateDevices);
                }
                if let Some(report) = &self.permission {
                    let _ = report;
                    let arrow = if self.show_details { "Скрыть" } else { "Детали" };
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
                .unwrap_or_else(|| "не выбрано".to_string());

            k::heading(ui, "Устройство записи");
            k::hint(ui, "Микрофон, с которого идёт диктовка.");
            ui.add_space(t::XS);
            // Список во всю ширину: имена устройств длинные и в строку не помещаются.
            let w = ui.available_width();
            egui::ComboBox::from_id_source("device")
                .selected_text(RichText::new(shorten(&selected_label, 42)).size(t::T_BODY))
                .width(w)
                .show_ui(ui, |ui| {
                    for d in &self.devices {
                        let label = if d.is_default {
                            format!("{} (по умолчанию)", d.name)
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
                    ui.label(RichText::new("Проверить микрофон").size(t::T_BODY));
                    k::hint(ui, "Запись без распознавания — убедиться, что звук идёт.");
                },
                |ui| {
                    if self.capturing {
                        if ui.button("Стоп").clicked() {
                            self.engine.send(MicCommand::StopCapture);
                        }
                    } else if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new("Записать"))
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
                "Сохранять запись в .wav",
                "Файлы складываются в папку recordings рядом с программой.",
            ) {
                self.dirty = true;
            }
            if self.last_record_path.is_some() || Self::recordings_dir().exists() {
                ui.add_space(t::BASE);
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new("Открыть папку записей")
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
            k::heading(ui, "Работа в фоне");
            ui.add_space(t::SM);
            k::hint(
                ui,
                "Крестик прячет TVOICE в трей — хоткей продолжает работать. \
                 Правый клик по значку: начать диктовку или выйти.",
            );
            ui.add_space(t::SM);
            if k::switch_row(
                ui,
                &mut self.start_in_tray,
                "Запускать свёрнутым в трей",
                "",
            ) {
                self.dirty = true;
            }
        });
    }

    fn log_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::row(
                ui,
                |ui| {
                    k::heading(ui, "Журнал");
                    k::hint(ui, "Последние события. Полная запись — в файле tvoice.log.");
                },
                |ui| {
                    if k::ghost_button(ui, "Открыть файл").clicked() {
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
