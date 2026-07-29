//! Настройки → «Ввод»: хоткей, режим диктовки и способ вставки текста.

use egui::RichText;

use crate::app::TvoiceApp;
use crate::inject;
use crate::overlay;
use crate::theme as t;
use crate::ui_kit as k;

impl TvoiceApp {
    pub(crate) fn settings_input(&mut self, ui: &mut egui::Ui) {
        self.hotkey_card(ui);
        ui.add_space(t::MD);
        self.mode_card(ui);
        ui.add_space(t::MD);
        self.paste_card(ui);
        ui.add_space(t::MD);
        self.hud_card(ui);
        ui.add_space(t::XL);
    }

    fn hotkey_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            let capturing = self.hotkey.is_capturing();
            k::row(
                ui,
                |ui| {
                    k::heading(ui, "Горячая клавиша");
                    k::hint(ui, "Работает поверх любого окна, в том числе из трея.");
                },
                |ui| {
                    if capturing {
                        if k::ghost_button(ui, "Отмена").clicked() {
                            self.hotkey.cancel_capture();
                        }
                    } else if k::ghost_button(ui, "Изменить").clicked() {
                        self.hotkey.start_capture();
                    }
                },
            );

            ui.add_space(t::SM);
            if capturing {
                ui.label(
                    RichText::new("Нажмите нужное сочетание — можно с кнопкой мыши")
                        .size(t::T_BODY)
                        .color(t::PRIMARY),
                );
            } else {
                // Сочетание показываем как клавиши, а не строкой: так читается быстрее.
                ui.horizontal(|ui| {
                    for (i, part) in self.hotkey.label().split(" + ").enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("+").size(t::T_LABEL).color(t::MUTED));
                        }
                        key_cap(ui, part);
                    }
                });
            }

            if self.hotkey.is_risky() {
                ui.add_space(t::XS);
                ui.label(
                    RichText::new(
                        "Обычная клавиша без модификаторов будет срабатывать при наборе текста.",
                    )
                    .size(t::T_LABEL)
                    .color(t::WARN),
                );
            }
        });
    }

    fn mode_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::heading(ui, "Как диктовать");
            ui.add_space(t::SM);

            if k::switch_row(
                ui,
                &mut self.streaming_enabled,
                "Текст появляется во время речи",
                "Хоткей работает переключателем: нажал — говоришь — нажал. \
                 Выключено: держишь клавишу, отпустил — текст вставился целиком.",
            ) {
                self.dirty = true;
            }
            k::divider(ui);
            if k::switch_row(
                ui,
                &mut self.insert_enabled,
                "Вставлять текст в активное окно",
                "Выключено — текст только показывается здесь, никуда не вставляется.",
            ) {
                self.dirty = true;
            }
        });
    }

    fn paste_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::heading(ui, "Способ вставки");
            k::hint(
                ui,
                "Через буфер обмена — надёжно везде, прежнее содержимое возвращается.",
            );
            ui.add_space(t::XS);
            {
                let w = ui.available_width();
                egui::ComboBox::from_id_source("paste_mode")
                    .selected_text(
                        RichText::new(inject::mode_name(self.paste_mode)).size(t::T_BODY),
                    )
                    .width(w)
                    .show_ui(ui, |ui| {
                        for mode in [
                            inject::MODE_AUTO,
                            inject::MODE_KEYS,
                            inject::MODE_CLIPBOARD,
                        ] {
                            if ui
                                .selectable_value(
                                    &mut self.paste_mode,
                                    mode,
                                    inject::mode_name(mode),
                                )
                                .clicked()
                            {
                                inject::set_mode(self.paste_mode);
                                self.dirty = true;
                            }
                        }
                    });
            }
            if self.paste_mode == inject::MODE_KEYS {
                ui.add_space(t::XS);
                ui.label(
                    RichText::new(
                        "Клавиатурный ввод нужен там, где Ctrl+V занят (терминалы), \
                         но в некоторых приложениях он изредка путает символы.",
                    )
                    .size(t::T_LABEL)
                    .color(t::WARN),
                );
            }
        });
    }
}

impl TvoiceApp {
    /// Индикатор диктовки: где показывать, какого размера, и как он выглядит.
    fn hud_card(&mut self, ui: &mut egui::Ui) {
        k::card(ui, |ui| {
            k::heading(ui, "Индикатор диктовки");
            k::hint(
                ui,
                "Небольшой значок, который виден во время записи поверх всех окон.",
            );
            ui.add_space(t::SM);

            k::row(
                ui,
                |ui| {
                    ui.label(RichText::new("Место на экране").size(t::T_BODY));
                    k::hint(ui, "Углы и середины сторон — на том мониторе, где вы работаете.");
                },
                |ui| {
                    let w = ui.available_width();
                    egui::ComboBox::from_id_source("hud_anchor")
                        .selected_text(RichText::new(self.hud_anchor.label()).size(t::T_BODY))
                        .width(w)
                        .show_ui(ui, |ui| {
                            for a in overlay::Anchor::ALL {
                                if ui
                                    .selectable_value(&mut self.hud_anchor, a, a.label())
                                    .clicked()
                                {
                                    overlay::set_anchor(a);
                                    self.dirty = true;
                                }
                            }
                        });
                },
            );

            k::divider(ui);
            k::row(
                ui,
                |ui| {
                    ui.label(RichText::new("Размер").size(t::T_BODY));
                    k::hint(ui, "От 60% до 220% базового.");
                },
                |ui| {
                    let slider = egui::Slider::new(
                        &mut self.hud_scale,
                        crate::hud::SCALE_MIN..=crate::hud::SCALE_MAX,
                    )
                    .show_value(false);
                    if ui.add_sized([ui.available_width(), 20.0], slider).changed() {
                        overlay::set_scale(self.hud_scale);
                        self.dirty = true;
                    }
                },
            );

            ui.add_space(t::SM);
            self.hud_preview(ui);
        });
    }

    /// Живой предпросмотр: рисуется тем же кодом, что и настоящий индикатор,
    /// поэтому разойтись с ним не может. Точки дышат под тот же уровень микрофона.
    fn hud_preview(&mut self, ui: &mut egui::Ui) {
        let time = ui.input(|i| i.time) as f32;
        let ([w, h], rgba) = crate::hud::preview(self.hud_scale, time, self.voice);
        let image = egui::ColorImage::from_rgba_premultiplied([w, h], &rgba);
        let texture = self
            .hud_texture
            .get_or_insert_with(|| ui.ctx().load_texture("hud", image.clone(), Default::default()));
        texture.set(image, Default::default());

        // Клетчатая подложка: видно, что фон индикатора полупрозрачный.
        egui::Frame::none()
            .fill(t::SURFACE_LOW)
            .rounding(egui::Rounding::same(t::R_SM))
            .inner_margin(egui::Margin::symmetric(t::MD, t::SM))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::from_texture(&*texture).fit_to_original_size(1.0));
                    ui.add_space(t::BASE);
                    ui.label(
                        RichText::new(format!("{w}×{h} точек · {:.0}%", self.hud_scale * 100.0))
                            .size(t::T_LABEL_SM)
                            .color(t::MUTED),
                    );
                });
            });
        // Предпросмотр живой — просим следующий кадр.
        ui.ctx().request_repaint();
    }
}

/// Клавиша сочетания — прямоугольник с подписью, как на клавиатуре.
fn key_cap(ui: &mut egui::Ui, text: &str) {
    egui::Frame::none()
        .fill(t::SURFACE_HIGHEST)
        .stroke(egui::Stroke::new(1.0_f32, t::OUTLINE))
        .rounding(egui::Rounding::same(t::R_SM))
        .inner_margin(egui::Margin::symmetric(t::SM, t::BASE + 1.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(t::T_BODY).color(t::PRIMARY));
        });
}
