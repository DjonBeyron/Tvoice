//! Вкладка «Модели»: движок whisper.cpp + каталог моделей Whisper с параметрами и загрузкой.

use egui::{Color32, RichText, Rounding, Sense, Stroke, Vec2};

use crate::app::{card, TvoiceApp};
use crate::models::{self, download, ModelInfo};
use crate::theme;

impl TvoiceApp {
    pub(crate) fn models_tab(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.whisper_engine_card(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new("Модели распознавания (бесплатные, офлайн)")
                    .size(13.0)
                    .color(theme::MUTED),
            );
            ui.add_space(4.0);
            for info in models::CATALOG {
                self.model_card(ui, info);
                ui.add_space(6.0);
            }
        });
    }

    fn whisper_engine_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let active = models::active_engine();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Движок").size(13.0).color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (txt, col) = match active {
                        Some(models::Engine::Gpu) => ("GPU (NVIDIA) ✓", theme::OK),
                        Some(models::Engine::Cpu) => ("CPU ✓", theme::OK),
                        None => ("не установлен", theme::WARN),
                    };
                    ui.label(RichText::new(txt).strong().color(col));
                });
            });

            let busy = self
                .downloads
                .lock()
                .map(|d| matches!(d.active.as_deref(), Some("whisper.exe") | Some("whisper-gpu")))
                .unwrap_or(false);

            if busy {
                self.download_progress(ui);
                return;
            }

            ui.add_space(6.0);
            ui.label(
                RichText::new("Выберите движок распознавания:")
                    .size(12.0)
                    .color(theme::MUTED),
            );
            ui.add_space(4.0);
            self.engine_option(ui, false, active, "CPU", "универсально, ~8 МБ");
            ui.add_space(4.0);
            self.engine_option(ui, true, active, "GPU (NVIDIA CUDA)", "быстрее, ~680 МБ, нужна карта NVIDIA");
        });
    }

    /// Один вариант движка: индикатор активности + кнопка выбрать/скачать.
    fn engine_option(
        &self,
        ui: &mut egui::Ui,
        gpu: bool,
        active: Option<models::Engine>,
        name: &str,
        note: &str,
    ) {
        let this = if gpu { models::Engine::Gpu } else { models::Engine::Cpu };
        let is_active = active == Some(this);
        let cached = models::engine_zip_cached(gpu);
        let can = !self.downloads_busy();

        let stroke = if is_active {
            Stroke::new(1.5_f32, theme::ACCENT)
        } else {
            Stroke::new(1.0_f32, theme::LINE)
        };
        egui::Frame::none()
            .stroke(stroke)
            .rounding(Rounding::same(8.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).strong());
                            if is_active {
                                ui.label(RichText::new("● активен").size(12.0).color(theme::ACCENT));
                            } else if cached {
                                ui.label(RichText::new("скачан").size(11.0).color(theme::MUTED));
                            }
                        });
                        ui.label(RichText::new(note).size(11.0).color(theme::MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if is_active {
                            ui.add_enabled(false, egui::Button::new("Активен"));
                        } else {
                            // Из кэша — переключение, иначе загрузка.
                            let label = if cached {
                                "Переключить".to_string()
                            } else if gpu {
                                "Скачать (~680 МБ)".to_string()
                            } else {
                                "Скачать (~8 МБ)".to_string()
                            };
                            if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                                download::start_whisper_binary(
                                    gpu,
                                    self.downloads.clone(),
                                    self.ctx.clone(),
                                );
                            }
                        }
                    });
                });
            });
    }

    fn model_card(&mut self, ui: &mut egui::Ui, info: &'static ModelInfo) {
        let is_active = self.selected_model.as_deref() == Some(info.id);
        let downloaded = models::is_downloaded(info.file);

        let stroke = if is_active {
            Stroke::new(1.5_f32, theme::ACCENT)
        } else {
            Stroke::new(1.0_f32, theme::LINE)
        };

        egui::Frame::none()
            .fill(theme::PANEL)
            .stroke(stroke)
            .rounding(Rounding::same(12.0))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // Заголовок + размер.
                ui.horizontal(|ui| {
                    ui.label(RichText::new(info.id).size(15.0).strong());
                    if is_active {
                        ui.label(RichText::new("● активна").size(12.0).color(theme::ACCENT));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(fmt_size(info.size_mb))
                                .size(12.0)
                                .monospace()
                                .color(theme::MUTED),
                        );
                    });
                });

                ui.add_space(4.0);
                // Параметры: скорость + точность.
                ui.horizontal(|ui| {
                    chip(ui, &format!("скорость: {}", info.speed), theme::WARN);
                    ui.add_space(6.0);
                    chip(ui, &format!("точность ~{}%", info.accuracy), theme::OK);
                });
                ui.add_space(4.0);
                accuracy_bar(ui, info.accuracy);

                ui.add_space(6.0);
                ui.label(RichText::new(info.desc).size(12.0).color(theme::MUTED));

                ui.add_space(8.0);

                // Действие: прогресс / скачать / выбрать.
                let this_downloading = self
                    .downloads
                    .lock()
                    .map(|d| d.active.as_deref() == Some(info.id))
                    .unwrap_or(false);

                if this_downloading {
                    self.download_progress(ui);
                } else if downloaded {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("✓ загружена").size(12.0).color(theme::OK));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if is_active {
                                ui.add_enabled(false, egui::Button::new("Выбрана"));
                            } else if ui.button("Выбрать").clicked() {
                                self.selected_model = Some(info.id.to_string());
                                self.dirty = true;
                            }
                        });
                    });
                } else {
                    let can = !self.downloads_busy();
                    let btn = egui::Button::new(format!("Скачать ({})", fmt_size(info.size_mb)))
                        .min_size(Vec2::new(0.0, 30.0));
                    if ui.add_enabled(can, btn).clicked() {
                        download::start_model(info, self.downloads.clone(), self.ctx.clone());
                    }
                }
            });
    }

    /// Строка прогресса текущей загрузки.
    fn download_progress(&self, ui: &mut egui::Ui) {
        let (frac, msg, done, total, err) = {
            let d = self.downloads.lock().unwrap();
            (
                d.fraction(),
                d.message.clone(),
                d.downloaded,
                d.total,
                d.error.clone(),
            )
        };
        ui.add_space(4.0);
        if let Some(e) = err {
            ui.label(RichText::new(e).size(12.0).color(theme::BAD));
            return;
        }
        let text = if total > 0 {
            format!("{} · {} / {}", msg, fmt_bytes(done), fmt_bytes(total))
        } else {
            msg
        };
        ui.add(egui::ProgressBar::new(frac).text(text).desired_height(18.0));
    }

    fn downloads_busy(&self) -> bool {
        self.downloads.lock().map(|d| d.is_busy()).unwrap_or(false)
    }
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.linear_multiply(0.15))
        .stroke(Stroke::new(1.0_f32, color.linear_multiply(0.5)))
        .rounding(Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(8.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

fn accuracy_bar(ui: &mut egui::Ui, accuracy: u8) {
    let desired = Vec2::new(ui.available_width(), 6.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(3.0), Color32::from_rgb(0x0D, 0x0F, 0x12));
    let mut fill = rect;
    fill.set_width(rect.width() * (accuracy as f32 / 100.0));
    p.rect_filled(fill, Rounding::same(3.0), theme::ACCENT.linear_multiply(0.8));
}

fn fmt_size(mb: u32) -> String {
    if mb >= 1024 {
        format!("{:.1} ГБ", mb as f32 / 1024.0)
    } else {
        format!("{mb} МБ")
    }
}

fn fmt_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.2} ГБ", mb / 1024.0)
    } else {
        format!("{mb:.1} МБ")
    }
}
