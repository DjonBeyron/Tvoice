//! Экран «Диктовка»: состояние, живой текст, крупная кнопка и то, чем диктуют.
//!
//! Здесь нет настроек — только то, что нужно во время работы. Всё остальное уехало
//! в «Настройки», иначе главный экран снова превратится в свалку из карточек.

use egui::{RichText, Sense, Vec2};

use crate::app::{Route, TvoiceApp};
use crate::models;
use crate::theme as t;
use crate::ui_kit as k;

/// Языки распознавания (код + подпись).
pub const LANGS: &[(&str, &str)] = &[
    ("ru", "Русский"),
    ("en", "English"),
    ("uk", "Українська"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("auto", "Авто"),
];

impl TvoiceApp {
    pub(crate) fn screen_main(&mut self, ui: &mut egui::Ui) {
        let ready = self.ready_to_dictate();
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(t::LG, t::MD))
            .show(ui, |ui| {
                // Прокрутка: в невысоком окне нижняя строка иначе обрезается.
                let w = crate::ui_shell::column_width(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        crate::ui_shell::content_column(ui, w, |ui| {
                            self.transcript_card(ui);
                            ui.add_space(t::MD);
                            self.level_row(ui);
                            ui.add_space(t::LG);
                            self.controls(ui, ready);
                            ui.add_space(t::MD);
                        });
                    });
            });
    }

    /// Готово ли приложение диктовать: есть движок и скачанная модель.
    fn ready_to_dictate(&self) -> bool {
        models::whisper_exe().is_some()
            && self
                .selected_model
                .as_ref()
                .and_then(|id| models::by_id(id))
                .map(|m| models::is_downloaded(m.file))
                .unwrap_or(false)
    }

    /// Живой текст: то, что уже распознано в этом сеансе.
    fn transcript_card(&mut self, ui: &mut egui::Ui) {
        let (state, text, error) = self
            .dictation
            .lock()
            .map(|s| (s.state.clone(), s.last_text.clone(), s.error.clone()))
            .unwrap_or_default();

        k::card(ui, |ui| {
            let dictating = self.dictating;
            k::row(
                ui,
                |ui| k::label(ui, "Распознанный текст"),
                |ui| {
                    let color = if dictating { t::PRIMARY } else { t::MUTED };
                    ui.label(
                        RichText::new(if state.is_empty() { "Ожидание" } else { &state })
                            .size(t::T_LABEL)
                            .color(color),
                    );
                },
            );
            ui.add_space(t::SM);

            // Пока текста нет, прокрутка не нужна — рисуем подсказку напрямую.
            // Вложенная область прокрутки на пустом содержимом вела себя непредсказуемо.
            if let Some(err) = &error {
                ui.label(RichText::new(err).size(t::T_BODY).color(t::BAD));
                ui.add_space(100.0);
            } else if text.is_empty() {
                ui.label(
                    RichText::new(
                        "Нажмите кнопку ниже или хоткей — текст появится здесь \
                         и сразу пойдёт в активное окно.",
                    )
                    .size(t::T_BODY)
                    .color(t::MUTED),
                );
                ui.add_space(100.0);
            } else {
                egui::ScrollArea::vertical()
                    .id_source("transcript")
                    .max_height(150.0)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_height(120.0);
                        ui.label(RichText::new(text).size(t::T_BODY_LG));
                    });
            }
        });
    }

    /// Уровень входа: во время диктовки видно, что микрофон слышит.
    fn level_row(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            let db = if self.level > 1e-5 {
                format!("{:.0} дБ", 20.0 * self.level.log10())
            } else {
                "—".to_string()
            };
            k::row(
                ui,
                |ui| k::label(ui, "Уровень микрофона"),
                |ui| ui.label(RichText::new(db).size(t::T_LABEL).color(t::MUTED)),
            );
            ui.add_space(t::XS);
            let color = if self.dictating { t::PRIMARY_STRONG } else { t::OUTLINE };
            k::bar(ui, self.level, 6.0, color);
        });
    }

    /// Нижний блок: язык, крупная кнопка и активная модель.
    fn controls(&mut self, ui: &mut egui::Ui, ready: bool) {
        ui.vertical_centered(|ui| {
            if self.dictating {
                pulse_rings(ui, self.voice);
                ui.add_space(t::SM);
            }
            let combo = self.hotkey.label();
            let (text, hint) = if self.dictating {
                ("Остановить", format!("Идёт запись — говорите, или нажмите {combo}"))
            } else if ready {
                ("Начать диктовку", format!("или нажмите {combo}"))
            } else {
                (
                    "Начать диктовку",
                    "сначала скачайте движок и модель".to_string(),
                )
            };

            let resp = ui.add_enabled_ui(ready, |ui| k::primary_button(ui, text, 240.0));
            if resp.inner.clicked() {
                if self.dictating {
                    self.stop_dictation();
                } else {
                    self.begin_dictation(self.insert_enabled);
                }
            }
            ui.add_space(t::XS);
            // Подсказка второстепенна — держим её заметно тише основного текста.
            ui.label(
                RichText::new(hint)
                    .size(t::T_LABEL)
                    .color(t::MUTED.linear_multiply(0.55)),
            );
            if !ready {
                ui.add_space(t::XS);
                if k::ghost_button(ui, "Открыть настройки движка").clicked() {
                    self.route = Route::Settings;
                    self.settings_tab = crate::app::SettingsTab::Engine;
                }
            }
        });

        ui.add_space(t::LG);
        {
            // Язык.
            let current = LANGS
                .iter()
                .find(|(c, _)| *c == self.lang)
                .map(|(_, n)| *n)
                .unwrap_or("Русский");
            let ready_now = ready;
            k::row(ui, |ui| {
            ui.horizontal(|ui| {
            k::label(ui, "Язык");
            ui.add_space(t::XS);
            egui::ComboBox::from_id_source("lang")
                .selected_text(RichText::new(current).size(t::T_BODY))
                .width(140.0)
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
            }, |ui| {
                let model = self
                    .selected_model
                    .as_ref()
                    .and_then(|id| models::by_id(id))
                    .map(|m| m.id)
                    .unwrap_or("модель не выбрана");
                ui.label(RichText::new(model).size(t::T_LABEL).color(t::MUTED));
                ui.add_space(t::XS);
                k::dot(ui, if ready_now { t::OK } else { t::WARN }, 5.0);
            });
        }
    }
}

/// Кольца-пульсация во время записи — тот же визуализатор, что в макете.
/// `voice` — приведённая к 0…1 громкость: от неё зависят и размах, и яркость,
/// поэтому в тишине кольца едва теплятся, а на голосе расходятся широко.
fn pulse_rings(ui: &mut egui::Ui, voice: f32) {
    let size = 72.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let t_now = ui.input(|i| i.time) as f32;
    let p = ui.painter();
    let c = rect.center();
    let v = voice.clamp(0.0, 1.0);

    // Три кольца, разнесённые по фазе; на громком звуке они и быстрее, и дальше.
    let speed = 0.45 + v * 0.75;
    for i in 0..3 {
        let phase = (t_now * speed + i as f32 / 3.0) % 1.0;
        let r = size * (0.22 + phase * (0.28 + v * 0.22));
        let alpha = (1.0 - phase) * (0.12 + v * 0.5);
        p.circle_stroke(
            c,
            r,
            egui::Stroke::new(1.5_f32, t::PRIMARY.linear_multiply(alpha)),
        );
    }
    // Ядро: мягкий ореол и точка, растущая с голосом.
    p.circle_filled(c, size * 0.20 + v * 4.0, t::PRIMARY.linear_multiply(0.18));
    p.circle_filled(c, size * 0.08 + v * 7.0, t::PRIMARY);
}
