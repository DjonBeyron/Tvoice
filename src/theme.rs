//! Минимальная тёмная тема TVOICE.
//!
//! Держим палитру нейтрально-графитовой с одним акцентом, скругления мягкие,
//! отступы просторные — интерфейс должен читаться спокойно и «дорого».

use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, Rounding, Stroke, Visuals};

/// Акцентный цвет (спокойный сине-зелёный) — активные элементы, индикатор доступа.
pub const ACCENT: Color32 = Color32::from_rgb(0x35, 0xC9, 0xB0);
/// Фон окна.
pub const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x17);
/// Фон карточек/панелей.
pub const PANEL: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x22);
/// Фон выделенных элементов.
pub const PANEL_HI: Color32 = Color32::from_rgb(0x23, 0x27, 0x2E);
/// Обводка.
pub const LINE: Color32 = Color32::from_rgb(0x2C, 0x31, 0x39);

pub const OK: Color32 = Color32::from_rgb(0x37, 0xD6, 0x7A);
pub const WARN: Color32 = Color32::from_rgb(0xE7, 0xB4, 0x16);
pub const BAD: Color32 = Color32::from_rgb(0xE5, 0x5A, 0x5A);
pub const MUTED: Color32 = Color32::from_rgb(0x8A, 0x93, 0xA0);

pub fn apply(ctx: &Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();

    visuals.override_text_color = Some(Color32::from_rgb(0xE6, 0xEA, 0xF0));
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = Color32::from_rgb(0x0D, 0x0F, 0x12);
    visuals.faint_bg_color = PANEL;
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT.linear_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    let rounding = Rounding::same(8.0);

    // Обычные виджеты.
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.weak_bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    visuals.widgets.noninteractive.rounding = rounding;

    visuals.widgets.inactive.bg_fill = PANEL_HI;
    visuals.widgets.inactive.weak_bg_fill = PANEL_HI;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
    visuals.widgets.inactive.rounding = rounding;

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2B, 0x30, 0x38);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2B, 0x30, 0x38);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT.linear_multiply(0.6));
    visuals.widgets.hovered.rounding = rounding;

    visuals.widgets.active.bg_fill = ACCENT.linear_multiply(0.25);
    visuals.widgets.active.weak_bg_fill = ACCENT.linear_multiply(0.25);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.rounding = rounding;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(16.0);
    style.spacing.interact_size.y = 30.0;
    ctx.set_style(style);
}

/// Встраиваем DejaVu Sans как запасной шрифт — у него широкое покрытие символов
/// (стрелки, пунктуация, спецсимволы), чтобы вместо отсутствующих глифов не было «квадратов».
fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".to_owned(),
        FontData::from_static(include_bytes!("../assets/DejaVuSans.ttf")),
    );
    // Добавляем в конец обеих семей — используется как fallback для недостающих глифов.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("dejavu".to_owned());
    }
    ctx.set_fonts(fonts);
}
