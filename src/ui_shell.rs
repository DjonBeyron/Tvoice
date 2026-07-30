//! Оболочка интерфейса: боковая панель, верхняя строка и переключение экранов.
//!
//! Всё приложение — два экрана: «Диктовка» (то, ради чего его открывают) и «Настройки»
//! с тремя разделами. Плоский список вкладок из прежней версии разросся и путал:
//! состояние микрофона, каталог моделей и настройки вставки лежали в одном ряду.

use egui::{RichText, Rounding, Sense, Vec2};

use crate::lang::tr;
use crate::app::{Route, SettingsTab, TvoiceApp};
use crate::mic::PermissionState;
use crate::theme as t;
use crate::ui_kit as k;

impl TvoiceApp {
    pub(crate) fn shell(&mut self, ctx: &egui::Context) {
        self.topbar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(t::BG))
            .show(ctx, |ui| match self.route {
                Route::Main => self.screen_main(ui),
                Route::Settings => self.screen_settings(ui),
            });
        // Панель рисуем ПОСЛЕ содержимого: она всплывает над ним, а не раздвигает его.
        // Постоянно занимать треть узкого окна ради двух пунктов незачем.
        self.sidebar_overlay(ctx);
    }

    /// Выдвижная боковая панель поверх содержимого.
    fn sidebar_overlay(&mut self, ctx: &egui::Context) {
        if !self.sidebar_open {
            return;
        }
        let screen = ctx.screen_rect();
        // Затемнение под панелью: и отделяет её от содержимого, и закрывает по клику мимо —
        // иначе панель пришлось бы закрывать тем же бургером, что неочевидно.
        egui::Area::new(egui::Id::new("sidebar_scrim"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let resp = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, Rounding::ZERO, egui::Color32::from_black_alpha(110));
                if resp.clicked() {
                    self.sidebar_open = false;
                }
            });

        egui::Area::new(egui::Id::new("sidebar"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(t::SURFACE)
                    .stroke(egui::Stroke::new(1.0_f32, t::OUTLINE.linear_multiply(0.5)))
                    .inner_margin(egui::Margin {
                        left: 0.0,
                        right: 0.0,
                        top: t::LG,
                        bottom: t::MD,
                    })
                    .show(ui, |ui| {
                        ui.set_width(t::SIDEBAR);
                        ui.set_height(screen.height() - t::LG - t::MD);
                        self.sidebar_body(ui);
                    });
            });
    }

    fn sidebar_body(&mut self, ui: &mut egui::Ui) {
        {
            {
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
                if self.nav_item(ui, route == Route::Main, tr("Диктовка", "Dictation")) {
                    self.route = Route::Main;
                    self.sidebar_open = false;
                }
                if self.nav_item(ui, route == Route::Settings, tr("Настройки", "Settings")) {
                    self.route = Route::Settings;
                    self.sidebar_open = false;
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
                            (tr("Идёт запись", "Recording"), t::PRIMARY)
                        } else {
                            (tr("Готов", "Ready"), t::MUTED)
                        };
                        k::dot(ui, color, 6.0);
                        ui.add_space(t::BASE);
                        ui.label(RichText::new(text).size(t::T_LABEL).color(color));
                    });
                });
            }
        }
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
                let (full, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
                // Заголовок и кнопка стоят по той же колонке, что и карточки ниже: иначе на
                // широком окне содержимое центрировано, а шапка разъезжается по краям.
                let pad = column_pad(ui);
                let rect = egui::Rect::from_min_max(
                    egui::pos2(full.left() + pad, full.top()),
                    egui::pos2((full.left() + pad + column_width(ui)).min(full.right()), full.bottom()),
                );
                let cy = rect.center().y;

                // Бургер — первым в строке: панель теперь всплывающая, и открыть её больше
                // неоткуда.
                let burger_w = k::burger_button(ui, egui::pos2(rect.left(), cy), &mut self.sidebar_open);

                let name = match self.route {
                    Route::Main => tr("Диктовка", "Dictation"),
                    Route::Settings => tr("Настройки", "Settings"),
                };
                let title = ui.painter().layout_no_wrap(
                    name.to_owned(),
                    egui::FontId::proportional(t::T_HEADLINE_MD),
                    t::ON_SURFACE,
                );
                let title_w = title.size().x;
                let title_x = rect.left() + burger_w + t::SM;
                ui.painter().galley(
                    egui::pos2(title_x, cy - title.size().y / 2.0),
                    title,
                    t::ON_SURFACE,
                );

                let (text, color) = self.permission_chip();
                k::chip_on_line(ui, egui::pos2(title_x + title_w + t::SM, cy), text, color);

                let (resp, _) =
                    k::pill_button_on_line(ui, egui::pos2(rect.right(), cy), tr("В трей", "To tray"));
                let hide = resp.clicked();
                if hide {
                    let ctx = self.ctx.clone();
                    self.hide_window(&ctx);
                }
                // Линия под шапкой. Тянем её по колонке, а не по всему окну: линия во всю
                // ширину при центрированном содержимом только подчёркивала бы пустые поля.
                ui.add_space(t::SM);
                let (line, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(line.left() + pad, line.top()),
                        egui::pos2((line.left() + pad + column_width(ui)).min(line.right()), line.bottom()),
                    ),
                    Rounding::ZERO,
                    t::OUTLINE.linear_multiply(0.4),
                );
            });
    }

    /// Состояние доступа к микрофону — короткой строкой для шапки.
    pub(crate) fn permission_chip(&self) -> (&'static str, egui::Color32) {
        match self.permission.as_ref().map(|r| r.effective) {
            Some(PermissionState::Allowed) => (tr("Микрофон доступен", "Microphone available"), t::OK),
            Some(PermissionState::Denied) => (tr("Микрофон запрещён", "Microphone blocked"), t::BAD),
            Some(PermissionState::PromptRequired) => (tr("Нужен запрос доступа", "Permission needed"), t::WARN),
            Some(PermissionState::Unknown) | None => (tr("Проверяю микрофон", "Checking microphone"), t::MUTED),
        }
    }

    fn screen_settings(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(t::LG, t::MD))
            .show(ui, |ui| {
                // Ширину считаем ДО прокрутки: внутри неё доступная ширина больше
                // реально видимой, и колонка получалась шире окна.
                let w = column_width(ui);
                let mut tab = self.settings_tab;
                // Вкладки — по той же колонке, что и карточки под ними.
                content_column(ui, w, |ui| {
                    k::tabs(
                        ui,
                        &mut tab,
                        &[
                            (SettingsTab::Engine, tr("Движок и модели", "Engine and models")),
                            (SettingsTab::Hotkeys, tr("Ввод", "Input")),
                            (SettingsTab::Privacy, tr("Микрофон и система", "Microphone and system")),
                        ],
                    );
                });
                self.settings_tab = tab;
                ui.add_space(t::MD);
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

/// Предел ширины колонки содержимого.
///
/// Тянуть карточки на всю ширину нельзя: строка в полтора экрана читается плохо, глазу
/// тяжело возвращаться к началу следующей. Но и упираться в узкую колонку на широком
/// мониторе незачем — отсюда предел, а не фиксированная ширина.
const COLUMN_MAX: f32 = 860.0;

/// Ширина колонки содержимого — считается от размеров окна, а не от доступной ширины.
///
/// Внутри прокрутки egui сообщает больше места, чем видно на самом деле, и колонка
/// раз за разом получалась шире окна. Геометрия окна известна точно: ширина минус
/// боковая панель, поля и запас под полосу прокрутки.
pub fn column_width(ui: &egui::Ui) -> f32 {
    room(ui).min(COLUMN_MAX).max(240.0)
}

/// Сколько места остаётся содержимому по горизонтали.
fn room(ui: &egui::Ui) -> f32 {
    // Боковую панель не вычитаем: она всплывает поверх содержимого и места не занимает.
    ui.ctx().screen_rect().width() - 2.0 * t::LG - t::SM
}

/// Отступ слева, которым колонка ставится по центру свободного места.
///
/// Раньше колонка прижималась к левому краю, и на широком окне справа оставалась пустая
/// полоса в половину экрана — выглядело так, будто окно не растянулось, а содержимое
/// съехало. Той же мерой выравниваются шапка и вкладки настроек, иначе центрированные
/// карточки разошлись бы с заголовком.
pub fn column_pad(ui: &egui::Ui) -> f32 {
    ((room(ui) - column_width(ui)) / 2.0).max(0.0)
}

/// Колонка содержимого заданной ширины, поставленная по центру свободного места.
///
/// Единая мера для всех экранов: и содержимое, и шапка, и вкладки настроек считают отступ
/// одинаково, поэтому на любой ширине окна они стоят на одной вертикали.
pub fn content_column(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let pad = column_pad(ui);
    // Смещение задаём прямоугольником, а не горизонтальной раскладкой с отступом.
    // Обёртка `ui.horizontal` меняла то, какую ширину считает доступной вложенный код:
    // карточки внутри брали ширину строки, а не колонки, и в окне исходного размера
    // вылезали за правый край — кнопки и чипы обрезались.
    let avail = ui.available_rect_before_wrap();
    let rect = egui::Rect::from_min_size(
        egui::pos2(avail.left() + pad, avail.top()),
        egui::vec2(width, avail.height()),
    );
    ui.allocate_ui_at_rect(rect, |ui| {
        ui.set_max_width(width);
        // Жёсткая отсечка по колонке: даже если какой-то виджет посчитает себя шире
        // положенного, он не нарисуется поверх соседней панели и за краем окна.
        let clip = ui.clip_rect();
        let left = ui.max_rect().left();
        ui.set_clip_rect(egui::Rect::from_min_max(
            egui::pos2(left, clip.top()),
            egui::pos2((left + width).min(clip.right()), clip.bottom()),
        ));
        add(ui);
    });
}
