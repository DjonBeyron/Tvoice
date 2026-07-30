//! Настройки → «Движок и модели»: чем распознавать и какой моделью.

use egui::RichText;

use crate::lang::tr;
use crate::app::TvoiceApp;
use crate::models::{self, download, ModelInfo};
use crate::theme as t;
use crate::ui_kit as k;

impl TvoiceApp {
    pub(crate) fn settings_engine(&mut self, ui: &mut egui::Ui) {
        self.engine_card(ui);
        ui.add_space(t::MD);

        k::label(ui, tr("Модели распознавания", "Recognition models"));
        ui.add_space(t::XS);
        k::hint(
            ui,
            tr("Бесплатные модели Whisper, работают офлайн. Чем крупнее — тем точнее и медленнее.",
               "Free Whisper models, working offline. The bigger, the more accurate and slower."),
        );
        ui.add_space(t::SM);
        for info in models::CATALOG {
            self.model_card(ui, info);
            ui.add_space(t::XS);
        }
        ui.add_space(t::XL);
    }

    fn engine_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            let active = models::active_engine();
            k::row(
                ui,
                |ui| {
                    k::heading(ui, tr("Движок whisper.cpp", "whisper.cpp engine"));
                    k::hint(ui, tr("Считает распознавание. GPU в разы быстрее на тяжёлых моделях.",
                        "Does the recognition. A GPU is several times faster on heavy models."));
                },
                |ui| {
                    let (txt, col) = match active {
                        Some(models::Engine::Gpu) => ("GPU (NVIDIA)", t::OK),
                        Some(models::Engine::Cpu) => ("CPU", t::OK),
                        None => (tr("не установлен", "not installed"), t::WARN),
                    };
                    k::chip(ui, txt, col);
                },
            );

            let busy = self
                .downloads
                .lock()
                .map(|d| matches!(d.active.as_deref(), Some("whisper.exe") | Some("whisper-gpu")))
                .unwrap_or(false);
            if busy {
                ui.add_space(t::SM);
                self.download_progress(ui);
                return;
            }

            ui.add_space(t::SM);
            self.engine_option(ui, false, active, "CPU", tr("универсально, ~8 МБ", "works anywhere, ~8 MB"));
            ui.add_space(t::XS);
            self.engine_option(
                ui,
                true,
                active,
                "GPU (NVIDIA CUDA)",
                tr("быстрее, ~680 МБ, нужна карта NVIDIA", "faster, ~680 MB, needs an NVIDIA card"),
            );
        });
    }

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

        k::card_selected(ui, is_active, |ui| {
            k::row(
                ui,
                |ui| {
                    ui.label(RichText::new(name).size(t::T_BODY).strong());
                    k::hint(ui, note);
                },
                |ui| {
                    if is_active {
                        k::tag(ui, tr("активен", "active"), t::PRIMARY);
                    } else {
                        let label = if cached {
                            tr("Переключить", "Switch").to_string()
                        } else if gpu {
                            tr("Скачать · 680 МБ", "Download · 680 MB").to_string()
                        } else {
                            tr("Скачать · 8 МБ", "Download · 8 MB").to_string()
                        };
                        if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                            download::start_whisper_binary(
                                gpu,
                                self.downloads.clone(),
                                self.ctx.clone(),
                            );
                        }
                    }
                },
            );
        });
    }

    fn model_card(&mut self, ui: &mut egui::Ui, info: &'static ModelInfo) {
        let is_active = self.selected_model.as_deref() == Some(info.id);
        let downloaded = models::is_downloaded(info.file);
        let this_downloading = self
            .downloads
            .lock()
            .map(|d| d.active.as_deref() == Some(info.id))
            .unwrap_or(false);

        k::card_selected(ui, is_active, |ui| {
            k::row(
                ui,
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(info.id).size(t::T_BODY_LG).strong());
                        ui.add_space(t::XS);
                        if is_active {
                            k::tag(ui, tr("используется", "in use"), t::PRIMARY);
                        } else if downloaded {
                            k::tag(ui, tr("скачана", "downloaded"), t::OK);
                        }
                    });
                    k::hint(ui, info.desc());
                },
                |ui| {
                    if this_downloading {
                        return;
                    }
                    if downloaded {
                        if is_active {
                            ui.add_space(t::BASE);
                        } else if ui.button(tr("Выбрать", "Select")).clicked() {
                            self.selected_model = Some(info.id.to_string());
                            self.dirty = true;
                        }
                    } else {
                        let can = !self.downloads_busy();
                        let label = format!("{} · {}", tr("Скачать", "Download"), fmt_size(info.size_mb));
                        if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                            download::start_model(info, self.downloads.clone(), self.ctx.clone());
                        }
                    }
                },
            );

            ui.add_space(t::XS);
            // Бирки одноцветные. Разноцветные (оранжевая скорость, зелёная точность)
            // читались как состояние — «плохо/хорошо», — хотя это просто характеристики
            // модели, и ни одна из них сама по себе не хуже другой.
            ui.horizontal(|ui| {
                k::tag(ui, &format!("{}: {}", tr("скорость", "speed"), info.speed()), t::MUTED);
                ui.add_space(t::BASE);
                k::tag(ui, &format!("{} ~{}%", tr("точность", "accuracy"), info.accuracy), t::MUTED);
                ui.add_space(t::BASE);
                k::tag(ui, &fmt_size(info.size_mb), t::MUTED);
            });
            ui.add_space(t::XS);
            k::bar(ui, info.accuracy as f32 / 100.0, 4.0, t::PRIMARY_STRONG);

            if this_downloading {
                ui.add_space(t::XS);
                self.download_progress(ui);
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
        if let Some(e) = err {
            ui.label(RichText::new(e).size(t::T_LABEL).color(t::BAD));
            return;
        }
        let text = if total > 0 {
            format!("{msg} · {} {} {}", fmt_bytes(done), tr("из", "of"), fmt_bytes(total))
        } else {
            msg
        };
        ui.add(
            egui::ProgressBar::new(frac)
                .text(RichText::new(text).size(t::T_LABEL))
                .desired_height(18.0)
                .fill(t::PRIMARY_STRONG),
        );
    }

    pub(crate) fn downloads_busy(&self) -> bool {
        self.downloads.lock().map(|d| d.is_busy()).unwrap_or(false)
    }
}

fn fmt_size(mb: u32) -> String {
    if mb >= 1024 {
        format!("{:.1} {}", mb as f32 / 1024.0, tr("ГБ", "GB"))
    } else {
        format!("{mb} {}", tr("МБ", "MB"))
    }
}

fn fmt_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.2} {}", mb / 1024.0, tr("ГБ", "GB"))
    } else {
        format!("{mb:.1} {}", tr("МБ", "MB"))
    }
}
