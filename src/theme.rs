//! Токены оформления TVOICE — единственный источник цветов, отступов и размеров текста.
//!
//! Палитра, шкала отступов, радиусы и типографика взяты из макетов заказчика (Material 3,
//! тёмная схема с сиреневым акцентом). Ни один экран не заводит своих цветов и размеров:
//! всё берётся отсюда, иначе интерфейс снова расползётся.

use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, Rounding, Stroke, Visuals};

// --- Цвет -------------------------------------------------------------------
/// Фон приложения.
pub const BG: Color32 = rgb(0x12, 0x14, 0x13);
/// Поверхности по возрастанию «высоты»: карточки, наведение, выделение.
pub const SURFACE_LOW: Color32 = rgb(0x1A, 0x1C, 0x1C);
pub const SURFACE: Color32 = rgb(0x1E, 0x20, 0x20);
pub const SURFACE_HIGH: Color32 = rgb(0x28, 0x2A, 0x2A);
pub const SURFACE_HIGHEST: Color32 = rgb(0x33, 0x35, 0x35);
/// Акцент: спокойный сиреневый для активного состояния и ссылок.
pub const PRIMARY: Color32 = rgb(0xCF, 0xBD, 0xFF);
/// Насыщенный акцент для заливок (главная кнопка, прогресс).
pub const PRIMARY_STRONG: Color32 = rgb(0x9D, 0x7A, 0xFF);
/// Текст на акцентной заливке.
pub const ON_PRIMARY: Color32 = rgb(0x39, 0x00, 0x93);
/// Текст: основной и приглушённый.
pub const ON_SURFACE: Color32 = rgb(0xE2, 0xE2, 0xE1);
/// Второстепенный текст: подписи, пояснения, неактивные пункты.
///
/// Заметно тусклее основного. Прежний оттенок (#CBC3D5) почти не отличался от основного
/// текста, и подсказки спорили с ним за внимание вместо того, чтобы отходить на второй план.
pub const MUTED: Color32 = rgb(0x92, 0x8B, 0xA0);
/// Заголовок раздела внутри карточки: тише основного текста, но громче пояснений.
pub const HEADING: Color32 = rgb(0xC2, 0xBD, 0xCB);
/// Разделители и обводки.
pub const OUTLINE: Color32 = rgb(0x49, 0x44, 0x53);
/// Смысловые цвета состояния — отдельно от акцента.
pub const OK: Color32 = rgb(0x4A, 0xDE, 0x80);
pub const WARN: Color32 = rgb(0xE7, 0xB4, 0x16);
pub const BAD: Color32 = rgb(0xFF, 0xB4, 0xAB);

// --- Отступы ----------------------------------------------------------------
pub const BASE: f32 = 4.0;
pub const XS: f32 = 8.0;
pub const SM: f32 = 12.0;
pub const MD: f32 = 16.0;
pub const LG: f32 = 24.0;
pub const XL: f32 = 32.0;
/// Ширина боковой панели.
pub const SIDEBAR: f32 = 168.0;

// --- Радиусы ----------------------------------------------------------------
pub const R_SM: f32 = 4.0;
pub const R_MD: f32 = 8.0;
pub const R_LG: f32 = 12.0;

// --- Типографика ------------------------------------------------------------
pub const T_HEADLINE_MD: f32 = 22.0;
pub const T_HEADLINE_SM: f32 = 17.0;
pub const T_BODY_LG: f32 = 15.0;
pub const T_BODY: f32 = 13.5;
pub const T_LABEL: f32 = 12.0;
pub const T_LABEL_SM: f32 = 11.0;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn apply(ctx: &Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(ON_SURFACE);
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = rgb(0x0D, 0x0F, 0x0E);
    visuals.faint_bg_color = SURFACE;
    visuals.hyperlink_color = PRIMARY;
    visuals.selection.bg_fill = PRIMARY.linear_multiply(0.30);
    visuals.selection.stroke = Stroke::new(1.0_f32, PRIMARY);
    visuals.window_rounding = Rounding::same(R_LG);
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 16.0,
        spread: 0.0,
        color: Color32::from_black_alpha(96),
    };

    let r = Rounding::same(R_MD);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, OUTLINE.linear_multiply(0.5));
    w.noninteractive.rounding = r;

    w.inactive.bg_fill = SURFACE_HIGH;
    w.inactive.weak_bg_fill = SURFACE_HIGH;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, OUTLINE.linear_multiply(0.6));
    w.inactive.rounding = r;

    w.hovered.bg_fill = SURFACE_HIGHEST;
    w.hovered.weak_bg_fill = SURFACE_HIGHEST;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, PRIMARY.linear_multiply(0.5));
    w.hovered.rounding = r;

    w.active.bg_fill = PRIMARY.linear_multiply(0.22);
    w.active.weak_bg_fill = PRIMARY.linear_multiply(0.22);
    w.active.bg_stroke = Stroke::new(1.0_f32, PRIMARY);
    w.active.rounding = r;

    // Фокус клавиатурой должен быть виден — иначе интерфейс не пройти без мыши.
    w.open.bg_stroke = Stroke::new(1.0_f32, PRIMARY);
    visuals.widgets.noninteractive.fg_stroke.color = MUTED;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(XS, XS);
    style.spacing.button_padding = egui::vec2(SM, XS);
    style.spacing.window_margin = egui::Margin::same(MD);
    style.spacing.menu_margin = egui::Margin::same(XS);
    style.spacing.interact_size.y = 28.0;
    style.spacing.scroll.bar_width = 6.0;
    ctx.set_style(style);
}

/// Шрифт: встроенный DejaVu Sans. Он покрывает кириллицу и пунктуацию целиком,
/// поэтому вместо отсутствующих глифов не появляются «квадраты».
fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".to_owned(),
        FontData::from_static(include_bytes!("../assets/DejaVuSans.ttf")),
    );
    // Первым в пропорциональной семье — это основной текст интерфейса.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "dejavu".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("dejavu".to_owned());
    ctx.set_fonts(fonts);
}
