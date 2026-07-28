//! Вкладка «Микрофон»: статус доступа, устройства, запись, уровень, журнал.

use egui::{Color32, RichText, Rounding, Sense, Vec2};

use crate::app::{card, meter_bar, status_dot, TvoiceApp};
use crate::mic::wav::timestamp_filename;
use crate::mic::{MicCommand, PermissionState};
use crate::theme;

impl TvoiceApp {
    pub(crate) fn mic_tab(&mut self, ui: &mut egui::Ui) {
        self.permission_card(ui);
        ui.add_space(6.0);
        self.access_buttons(ui);
        ui.add_space(6.0);
        self.device_section(ui);
        ui.add_space(6.0);
        self.level_meter(ui);
        ui.add_space(6.0);
        self.log_console(ui);
    }

    fn permission_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let (state, color) = match self.permission.as_ref().map(|r| r.effective) {
                Some(PermissionState::Allowed) => ("Разрешён", theme::OK),
                Some(PermissionState::Denied) => ("Запрещён", theme::BAD),
                Some(PermissionState::PromptRequired) => ("Требуется запрос", theme::WARN),
                Some(PermissionState::Unknown) | None => ("Проверяется…", theme::MUTED),
            };
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.add_space(4.0);
                ui.label(RichText::new("Статус доступа").size(13.0).color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(state).strong().color(color));
                });
            });

            if let Some(report) = &self.permission {
                ui.add_space(4.0);
                let arrow = if self.show_details { "▾" } else { "▸" };
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!("{arrow} подробности"))
                                .size(12.0)
                                .color(theme::MUTED),
                        )
                        .sense(Sense::click()),
                    )
                    .clicked()
                {
                    self.show_details = !self.show_details;
                }
                if self.show_details {
                    for line in &report.details {
                        ui.label(RichText::new(line).size(12.0).color(theme::MUTED));
                    }
                }
            }
        });
    }

    fn access_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let official = egui::Button::new(
                RichText::new("Запросить доступ (WinRT)")
                    .strong()
                    .color(Color32::BLACK),
            )
            .fill(theme::ACCENT)
            .min_size(Vec2::new(0.0, 34.0));
            if ui.add(official).clicked() {
                self.push_log("→ Запрос доступа (WinRT)…");
                self.engine.send(MicCommand::RequestOfficial);
            }
            if ui
                .add(egui::Button::new("Обновить статус").min_size(Vec2::new(0.0, 34.0)))
                .clicked()
            {
                self.engine.send(MicCommand::RefreshPermission);
                self.engine.send(MicCommand::EnumerateDevices);
            }
        });
        if let Some((ok, detail)) = &self.last_official {
            let c = if *ok { theme::OK } else { theme::BAD };
            ui.label(RichText::new(detail).size(12.0).color(c));
        }
    }

    fn device_section(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Устройство захвата").size(13.0).color(theme::MUTED));
            ui.add_space(4.0);
            let selected_label = self
                .devices
                .iter()
                .find(|d| Some(&d.id) == self.selected.as_ref())
                .map(|d| d.name.clone())
                .unwrap_or_else(|| "— не выбрано —".to_string());

            egui::ComboBox::from_id_source("device_combo")
                .selected_text(selected_label)
                .width(ui.available_width())
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

            ui.add_space(6.0);
            ui.add_enabled(
                !self.capturing,
                egui::Checkbox::new(&mut self.record_enabled, "Записывать в .wav"),
            );

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if !self.capturing {
                    let start = egui::Button::new(
                        RichText::new("▶ Старт (WASAPI)").strong().color(Color32::BLACK),
                    )
                    .fill(theme::OK)
                    .min_size(Vec2::new(0.0, 32.0));
                    if ui.add_enabled(self.selected.is_some(), start).clicked() {
                        let record = if self.record_enabled {
                            let dir = Self::recordings_dir();
                            match std::fs::create_dir_all(&dir) {
                                Ok(()) => {
                                    let path = dir.join(timestamp_filename());
                                    self.last_record_path = Some(path.clone());
                                    Some(path)
                                }
                                Err(e) => {
                                    self.push_log(format!("Папка записей: {e}"));
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        self.engine.send(MicCommand::StartCapture {
                            device: self.selected.clone(),
                            record,
                            live: None,
                        });
                    }
                } else {
                    let stop = egui::Button::new(RichText::new("■ Стоп").strong())
                        .fill(theme::BAD)
                        .min_size(Vec2::new(0.0, 32.0));
                    if ui.add(stop).clicked() {
                        self.engine.send(MicCommand::StopCapture);
                    }
                }
            });

            if let Some((dev, fmt)) = &self.capture_info {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{dev} — {fmt}")).size(12.0).color(theme::MUTED));
            }

            if self.last_record_path.is_some() || Self::recordings_dir().exists() {
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new("Открыть папку записей")
                                .size(12.0)
                                .color(theme::ACCENT),
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

    fn level_meter(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Уровень входа").size(13.0).color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let db = if self.level > 1e-5 {
                        format!("{:.1} dB", 20.0 * self.level.log10())
                    } else {
                        "-∞ dB".to_string()
                    };
                    ui.label(RichText::new(db).size(12.0).monospace().color(theme::MUTED));
                });
            });
            ui.add_space(6.0);
            meter_bar(ui, self.level);
        });
    }

    fn log_console(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Журнал").size(13.0).color(theme::MUTED));
            ui.add_space(4.0);
            egui::Frame::none()
                .fill(Color32::from_rgb(0x0D, 0x0F, 0x12))
                .rounding(Rounding::same(8.0))
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.set_min_height(90.0);
                    egui::ScrollArea::vertical()
                        .max_height(120.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for line in &self.log {
                                ui.label(
                                    RichText::new(line)
                                        .size(12.0)
                                        .monospace()
                                        .color(Color32::from_rgb(0xB6, 0xC0, 0xCC)),
                                );
                            }
                        });
                });
        });
    }
}
