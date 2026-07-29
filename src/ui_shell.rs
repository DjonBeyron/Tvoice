//! Оболочка интерфейса: боковая панель, верхняя строка и переключение экранов.
//!
//! Всё приложение — два экрана: «Диктовка» (то, ради чего его открывают) и «Настройки»
//! с тремя разделами. Плоский список вкладок из прежней версии разросся и путал:
//! состояние микрофона, каталог моделей и настройки вставки лежали в одном ряду.

use egui::{RichText, Rounding, Sense, Vec2};

use crate::app::{Route, SettingsTab, TvoiceApp};
use crate::mic::PermissionState;
use crate::theme as t;
use crate::ui_kit as k;

impl TvoiceApp {
    pub(crate) fn shell(&mut self, ctx: &egui::Context) {
        self.sidebar(ctx);
        self.topbar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(t::BG))
            .show(ctx, |ui| match self.route {
                Route::Main => self.screen_main(ui),
                Route::Settings => self.screen_settings(ui),
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(t::SIDEBAR)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(t::SURFACE)
                    .inner_margin(egui::Margin {
                        left: 0.0,
                        right: 0.0,
                        top: t::LG,
                        bottom: t::MD,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(t::MD);
                    ui.label(
                        RichText::new("TVOICE")
                            .size(t::T_HEADLINE_SM)
                            .strong()
                            .color(t::PRIMARY),
                    );
                });
                ui.add_space(t::LG);

                let route = self.route;
                if self.nav_item(ui, route == Route::Main, "Диктовка") {
                    self.route = Route::Main;
                }
                if self.nav_item(ui, route == Route::Settings, "Настройки") {
                    self.route = Route::Settings;
                }

                // Низ панели: состояние диктовки одним взглядом.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(t::XS);
                    ui.horizontal(|ui| {
                        ui.add_space(t::MD);
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .size(t::T_LABEL_SM)
                                .color(t::MUTED),
                        );
                    });
                    ui.add_space(t::XS);
                    ui.horizontal(|ui| {
                        ui.add_space(t::MD);
                        let (text, color) = if self.dictating {
                            ("Идёт запись", t::PRIMARY)
                        } else {
                            ("Готов", t::MUTED)
                        };
                        k::dot(ui, color, 6.0);
                        ui.add_space(t::BASE);
                        ui.label(RichText::new(text).size(t::T_LABEL).color(color));
                    });
                });
            });
    }

    /// Пункт бокового меню: активный помечен полосой слева и заливкой.
    fn nav_item(&self, ui: &mut egui::Ui, active: bool, text: &str) -> bool {
        let height = 38.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
        let hovered = resp.hovered();
        let p = ui.painter();
        if active || hovered {
            let fill = if active {
                t::SURFACE_HIGH
            } else {
                t::SURFACE_HIGH.linear_multiply(0.5)
            };
            p.rect_filled(rect, Rounding::ZERO, fill);
        }
        if active {
            let marker = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + height * 0.2),
                Vec2::new(2.0, height * 0.6),
            );
            p.rect_filled(marker, Rounding::ZERO, t::PRIMARY);
        }
        let color = if active { t::PRIMARY } else { t::MUTED };
        p.text(
            egui::pos2(rect.left() + t::MD, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(t::T_BODY),
            color,
        );
        resp.clicked()
    }

    fn topbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("topbar")
            .frame(
                egui::Frame::none()
                    .fill(t::BG)
                    .inner_margin(egui::Margin::symmetric(t::LG, t::SM)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                // Заголовок, чип и кнопка строятся от одной осевой линии: раскладка
                // egui центрирует каждый элемент по его собственной рамке, и оптически
                // они расходились — заголовок стоял выше чипа.
                let row_h = 36.0;
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
                let cy = rect.center().y;

                let name = match self.route {
                    Route::Main => "Диктовка",
                    Route::Settings => "Настройки",
                };
                let title = ui.painter().layout_no_wrap(
                    name.to_owned(),
                    egui::FontId::proportional(t::T_HEADLINE_MD),
                    t::ON_SURFACE,
                );
                let title_w = title.size().x;
                ui.painter().galley(
                    egui::pos2(rect.left(), cy - title.size().y / 2.0),
                    title,
                    t::ON_SURFACE,
                );

                let (text, color) = self.permission_chip();
                k::chip_on_line(ui, egui::pos2(rect.left() + title_w + t::SM, cy), text, color);

                let (resp, _) =
                    k::pill_button_on_line(ui, egui::pos2(rect.right(), cy), "В трей");
                let hide = resp.clicked();
                if hide {
                    let ctx = self.ctx.clone();
                    self.hide_window(&ctx);
                }
                // Линия под шапкой — граница между навигацией и содержимым.
                ui.add_space(t::SM);
                let w = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    Rounding::ZERO,
                    t::OUTLINE.linear_multiply(0.4),
                );
            });
    }

    /// Состояние доступа к микрофону — короткой строкой для шапки.
    pub(crate) fn permission_chip(&self) -> (&'static str, egui::Color32) {
        match self.permission.as_ref().map(|r| r.effective) {
            Some(PermissionState::Allowed) => ("Микрофон доступен", t::OK),
            Some(PermissionState::Denied) => ("Микрофон запрещён", t::BAD),
            Some(PermissionState::PromptRequired) => ("Нужен запрос доступа", t::WARN),
            Some(PermissionState::Unknown) | None => ("Проверяю микрофон", t::MUTED),
        }
    }

    fn screen_settings(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(t::LG, t::MD))
            .show(ui, |ui| {
                let mut tab = self.settings_tab;
                k::tabs(
                    ui,
                    &mut tab,
                    &[
                        (SettingsTab::Engine, "Движок и модели"),
                        (SettingsTab::Hotkeys, "Ввод"),
                        (SettingsTab::Privacy, "Микрофон и система"),
                    ],
                );
                self.settings_tab = tab;
                ui.add_space(t::MD);

                // Ширину считаем ДО прокрутки: внутри неё доступная ширина больше
                // реально видимой, и колонка получалась шире окна.
                let w = column_width(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        content_column(ui, w, |ui| match self.settings_tab {
                            SettingsTab::Engine => self.settings_engine(ui),
                            SettingsTab::Hotkeys => self.settings_input(ui),
                            SettingsTab::Privacy => self.settings_system(ui),
                        });
                    });
            });
    }
}

/// Ширина колонки содержимого — считается от размеров окна, а не от доступной ширины.
///
/// Внутри прокрутки egui сообщает больше места, чем видно на самом деле, и колонка
/// раз за разом получалась шире окна. Геометрия окна известна точно: ширина минус
/// боковая панель, поля и запас под полосу прокрутки.
pub fn column_width(ui: &egui::Ui) -> f32 {
    let screen = ui.ctx().screen_rect().width();
    (screen - t::SIDEBAR - 2.0 * t::LG - t::SM)
        .min(620.0)
        .max(240.0)
}

/// Колонка содержимого заданной ширины — единая мера для всех экранов.
pub fn content_column(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_max_width(width);
            add(ui);
        },
    );
}
