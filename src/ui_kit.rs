//! Общие элементы интерфейса: карточка, заголовок раздела, чип, переключатель, кнопки.
//!
//! Экраны собираются только из них — так оформление остаётся одинаковым, а правка
//! вида делается в одном месте. Ни один экран не рисует рамки и цвета сам.

use egui::{Color32, Response, RichText, Rounding, Sense, Stroke, Ui, Vec2};

use crate::theme as t;

// --- Текст ------------------------------------------------------------------

/// Заголовок раздела внутри карточки.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(t::T_HEADLINE_SM)
            .strong()
            .color(t::HEADING),
    );
}

/// Подпись над полем: мелкая, приглушённая, вразрядку.
pub fn label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(t::T_LABEL_SM)
            .color(t::MUTED),
    );
}

/// Пояснение под элементом.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(t::T_LABEL).color(t::MUTED));
}

// --- Контейнеры -------------------------------------------------------------

/// Карточка: основная единица компоновки на всех экранах.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    boxed(ui, t::MD, |ui, inner| {
        egui::Frame::none()
            .fill(t::SURFACE)
            .stroke(Stroke::new(1.0_f32, t::OUTLINE.linear_multiply(0.45)))
            .rounding(Rounding::same(t::R_MD))
            .inner_margin(egui::Margin::same(t::MD))
            .show(ui, |ui| {
                ui.set_width(inner);
                clip_to_self(ui, inner);
                add(ui)
            })
            .inner
    })
}

/// Сузить область отсечки до содержимого самой карточки.
///
/// Строки и чипы внутри меряют ширину по отсечке (см. `row`), а она до сих пор была
/// отсечкой КОЛОНКИ. Поэтому чип в правом верхнем углу дотягивался до края колонки, то есть
/// ровно до границы карточки, и обрезался ею. Сузив отсечку, мы заодно даём вложенным
/// элементам верную меру: дальше своей карточки они не потянутся.
fn clip_to_self(ui: &mut Ui, inner: f32) {
    let clip = ui.clip_rect();
    let left = ui.max_rect().left();
    ui.set_clip_rect(egui::Rect::from_min_max(
        egui::pos2(left, clip.top()),
        egui::pos2((left + inner).min(clip.right()), clip.bottom()),
    ));
}

/// Обёртка фиксированной ширины вокруг карточки.
///
/// Без неё вложенная карточка получала места БОЛЬШЕ, чем есть у родителя: замер на экране
/// настроек дал строку с правым краем 748 при родителе до 668. Карточка выезжала за
/// колонку, и её резала защитная отсечка — снаружи это выглядело как обрубленные кнопки и
/// чипы без правого края. `Frame` берёт место из `available_rect_before_wrap` родителя, а
/// тот после `set_width` оказывался шире, чем следует; явная обёртка это пресекает.
fn boxed<R>(ui: &mut Ui, margin: f32, add: impl FnOnce(&mut Ui, f32) -> R) -> R {
    // Ширину ограничиваем ещё и областью отсечки. `available_width` по ходу отрисовки
    // раздувается: замер показал, как родитель вырос с 40..668 до 40..760, а колонка —
    // с 24..684 до 24..776. Причину роста найти не удалось, а отсечка колонки остаётся
    // верной всё время, поэтому меряем по ней — карточка не вылезает за колонку, и её
    // перестаёт обрубать.
    let limit = (ui.clip_rect().right() - ui.next_widget_position().x).max(0.0);
    let w = ui.available_width().min(limit).max(0.0);
    let inner = (w - 2.0 * margin).max(0.0);
    // Прямоугольник задаём ЖЁСТКО, а не «желаемым размером». `allocate_ui_with_layout`
    // отдаёт родителю фактически занятое место, а `set_width` внутри фрейма расширяет Ui —
    // рост уходил наружу и раздувал родителя для следующих карточек: замер показал, как
    // колонка по ходу отрисовки выросла с 24..684 до 24..776, а внешняя карточка — с
    // 40..668 до 40..760. Отсюда и обрубленные кнопки: карточка считала себя шире колонки
    // и попадала под защитную отсечку.
    let top_left = ui.next_widget_position();
    let rect = egui::Rect::from_min_size(top_left, Vec2::new(w, ui.available_height().max(0.0)));
    ui.allocate_ui_at_rect(rect, |ui| {
        ui.set_max_width(w);
        add(ui, inner)
    })
    .inner
}

/// Карточка с выделенной рамкой — для выбранного варианта.
pub fn card_selected<R>(ui: &mut Ui, selected: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (fill, stroke) = if selected {
        (t::SURFACE, Stroke::new(1.5_f32, t::PRIMARY.linear_multiply(0.7)))
    } else {
        (
            t::SURFACE_LOW,
            Stroke::new(1.0_f32, t::OUTLINE.linear_multiply(0.45)),
        )
    };
    boxed(ui, t::SM, |ui, inner| {
        egui::Frame::none()
            .fill(fill)
            .stroke(stroke)
            .rounding(Rounding::same(t::R_MD))
            .inner_margin(egui::Margin::same(t::SM))
            .show(ui, |ui| {
                ui.set_width(inner);
                clip_to_self(ui, inner);
                add(ui)
            })
            .inner
    })
}

/// Строка «слева содержимое, справа действие» — типовая компоновка настроек.
///
/// Ширину обеих половин считаем заранее и жёстко её задаём. Двухфазная раскладка
/// («сначала правое, левому — остаток») внутри прокрутки давала обратную связь:
/// широкое содержимое раздвигало область, та отдавала больше ширины, карточка росла
/// и уезжала за край окна. Здесь расти нечему — сумма половин всегда равна строке.
pub fn row<R>(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui) -> R) -> R {
    let total = (ui.clip_rect().right() - ui.next_widget_position().x)
        .min(ui.available_width())
        .max(0.0);
    let right_w = (total * 0.42).min(250.0).max(0.0);
    let left_w = (total - right_w - t::SM).max(0.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(left_w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(left_w);
                left(ui);
            },
        );
        // Правой половине отдаём ВСЮ оставшуюся ширину строки, а не заранее посчитанные
        // `right_w`: с фиксированной шириной блок вставал сразу за левой половиной, и
        // кнопка повисала посреди карточки вместо правого края. Выравнивание
        // `right_to_left` + `Align::Min` прижимает её к правому ВЕРХНЕМУ углу — там её и
        // ищут глазами, когда слева несколько строк описания.
        // По той же причине, что и в `boxed`: `available_width` раздувается, отсечка — нет.
        let rest = (ui.clip_rect().right() - ui.next_widget_position().x)
            .min(ui.available_width())
            .max(0.0);
        ui.allocate_ui_with_layout(
            Vec2::new(rest, 0.0),
            egui::Layout::right_to_left(egui::Align::Min),
            |ui| {
                ui.set_max_width(rest);
                right(ui)
            },
        )
        .inner
    })
    .inner
}

/// Тонкий разделитель между строками одного раздела.
pub fn divider(ui: &mut Ui) {
    ui.add_space(t::SM);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, Rounding::ZERO, t::OUTLINE.linear_multiply(0.35));
    ui.add_space(t::SM);
}

// --- Индикаторы -------------------------------------------------------------

/// Высота мелких элементов строки заголовка: чипа и кнопки. Одна на всех, иначе
/// разные по росту элементы на одной оси всё равно смотрятся вразнобой.
pub const PILL_H: f32 = 22.0;

/// Чип состояния: точка + подпись.
///
/// Размер считаем сами и рисуем в отведённый прямоугольник, а не собираем из виджетов
/// внутри рамки. Раскладка egui давала здесь выбор из трёх зол, все проверены: с нулевым
/// запрошенным размером чип вылезал за карточку и обрезался, с `with_layout` растягивался
/// на всю правую половину строки, с `horizontal` наследовал правостороннее направление и
/// точка уезжала за текст. С готовым прямоугольником ни одного из этих вопросов нет.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(t::T_LABEL_SM),
        color,
    );
    let dot_r = 2.5;
    let pad = t::XS;
    let gap = t::BASE + 1.0;
    let h = PILL_H;
    let w = pad * 2.0 + dot_r * 2.0 + gap + galley.size().x;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let round = Rounding::same(h / 2.0);
    let p = ui.painter();
    p.rect_filled(rect, round, color.linear_multiply(0.12));
    p.rect_stroke(rect, round, Stroke::new(1.0_f32, color.linear_multiply(0.35)));
    // Точка — по оптическому центру букв, как и в чипе шапки (см. `chip_on_line`).
    let ty = rect.center().y - galley.size().y / 2.0;
    let ink_cy = ty + galley.mesh_bounds.center().y;
    p.circle_filled(egui::pos2(rect.left() + pad + dot_r, ink_cy), dot_r, color);
    p.galley(
        egui::pos2(rect.left() + pad + dot_r * 2.0 + gap, ty),
        galley,
        color,
    );
}

/// Чип состояния, нарисованный по центру заданной линии; возвращает ширину.
///
/// Обычный чип центрируется по своей рамке, а заголовок рядом — по своей, и оптически
/// они расходятся. Здесь и подложка, и точка, и текст строятся от одной осевой,
/// поэтому строка получается ровной при любом размере заголовка.
pub fn chip_on_line(ui: &mut Ui, left_center: egui::Pos2, text: &str, color: Color32) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(t::T_LABEL_SM),
        color,
    );
    let dot_r = 2.5;
    let pad = t::XS;
    let gap = t::BASE + 1.0;
    let h = PILL_H;
    let w = pad * 2.0 + dot_r * 2.0 + gap + galley.size().x;
    let rect = egui::Rect::from_min_size(
        egui::pos2(left_center.x, left_center.y - h / 2.0),
        Vec2::new(w, h),
    );
    let round = Rounding::same(h / 2.0);
    let p = ui.painter();
    p.rect_filled(rect, round, color.linear_multiply(0.12));
    p.rect_stroke(rect, round, Stroke::new(1.0_f32, color.linear_multiply(0.35)));
    // Точку ставим по ОПТИЧЕСКОМУ центру надписи, а не по геометрическому центру пилюли.
    // Строка шрифта несимметрична — сверху запас под выносные, снизу под нижние, — и
    // точка, выставленная по геометрии, оказывалась на пиксель выше букв: замер на
    // «Микрофон доступен» дал центр букв 61.5 против центра пилюли 60.5.
    let ty = rect.center().y - galley.size().y / 2.0;
    let ink_cy = ty + galley.mesh_bounds.center().y;
    p.circle_filled(
        egui::pos2(rect.left() + pad + dot_r, ink_cy),
        dot_r,
        color,
    );
    p.galley(
        egui::pos2(rect.left() + pad + dot_r * 2.0 + gap, ty),
        galley,
        color,
    );
    w
}

/// Небольшая кнопка строки заголовка: ширина по подписи, высота как у чипа,
/// центр — на заданной осевой линии. Возвращает отклик и занятую ширину.
pub fn pill_button_on_line(
    ui: &mut Ui,
    right_center: egui::Pos2,
    text: &str,
) -> (Response, f32) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(t::T_LABEL_SM),
        t::MUTED,
    );
    let w = galley.size().x + 2.0 * t::SM;
    let rect = egui::Rect::from_min_size(
        egui::pos2(right_center.x - w, right_center.y - PILL_H / 2.0),
        Vec2::new(w, PILL_H),
    );
    let resp = ui.put(
        rect,
        egui::Button::new(RichText::new(text).size(t::T_LABEL_SM).color(t::MUTED))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0_f32, t::OUTLINE))
            .rounding(Rounding::same(PILL_H / 2.0)),
    );
    (resp, w)
}

/// Кнопка-бургер: три полоски, центр — на заданной осевой линии. Возвращает занятую ширину
/// и переключает переданный признак по нажатию.
pub fn burger_button(ui: &mut Ui, left_center: egui::Pos2, open: &mut bool) -> f32 {
    let size = PILL_H;
    let rect = egui::Rect::from_min_size(
        egui::pos2(left_center.x, left_center.y - size / 2.0),
        Vec2::splat(size),
    );
    // Именно `interact`, а не `allocate_rect`: строка шапки уже размечена целиком, и
    // повторная разметка того же места нажатий не получала.
    let resp = ui.interact(rect, ui.id().with("burger"), Sense::click());
    let color = if resp.hovered() || *open { t::ON_SURFACE } else { t::MUTED };
    if resp.hovered() {
        ui.painter().rect_filled(rect, Rounding::same(t::R_SM), t::SURFACE_HIGH);
    }
    let p = ui.painter();
    let bar_w = size * 0.5;
    let x0 = rect.center().x - bar_w / 2.0;
    for i in 0..3 {
        let y = rect.center().y + (i as f32 - 1.0) * 5.0;
        p.line_segment(
            [egui::pos2(x0, y), egui::pos2(x0 + bar_w, y)],
            Stroke::new(1.5_f32, color),
        );
    }
    if resp.clicked() {
        *open = !*open;
    }
    size
}

/// Метка-бирка без точки (скорость, точность, размер).
pub fn tag(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.linear_multiply(0.12))
        .rounding(Rounding::same(t::R_SM))
        .inner_margin(egui::Margin::symmetric(t::XS, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(t::T_LABEL_SM).color(color));
        });
}

pub fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

/// Полоса заполнения (уровень входа, точность модели, прогресс загрузки).
pub fn bar(ui: &mut Ui, value: f32, height: f32, color: Color32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let p = ui.painter();
    let round = Rounding::same(height / 2.0);
    p.rect_filled(rect, round, t::SURFACE_HIGHEST);
    let mut fill = rect;
    fill.set_width(rect.width() * value.clamp(0.0, 1.0));
    if fill.width() > 1.0 {
        p.rect_filled(fill, round, color);
    }
}

// --- Кнопки и переключатели -------------------------------------------------

/// Главное действие экрана: заливка акцентом.
pub fn primary_button(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    let btn = egui::Button::new(
        RichText::new(text)
            .size(t::T_BODY_LG)
            .strong()
            .color(t::ON_PRIMARY),
    )
    .fill(t::PRIMARY_STRONG)
    .rounding(Rounding::same(t::R_LG))
    .min_size(Vec2::new(min_width, 42.0));
    ui.add(btn)
}

/// Второстепенное действие: обводка без заливки.
pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    let btn = egui::Button::new(RichText::new(text).size(t::T_BODY))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0_f32, t::OUTLINE))
        .rounding(Rounding::same(t::R_MD));
    ui.add(btn)
}

/// Переключатель настройки: заголовок, пояснение и галочка справа.
pub fn switch_row(ui: &mut Ui, on: &mut bool, title: &str, note: &str) -> bool {
    let changed = row(
        ui,
        |ui| {
            ui.label(RichText::new(title).size(t::T_BODY));
            if !note.is_empty() {
                ui.label(RichText::new(note).size(t::T_LABEL).color(t::MUTED));
            }
        },
        |ui| ui.add(egui::Checkbox::without_text(on)).changed(),
    );
    changed
}

/// Вкладки второго уровня (внутри настроек).
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for (value, name) in items {
            let active = *current == *value;
            let color = if active { t::PRIMARY } else { t::MUTED };
            let resp = ui.add(
                egui::Label::new(RichText::new(*name).size(t::T_BODY).color(color))
                    .sense(Sense::click()),
            );
            if resp.clicked() {
                *current = *value;
                changed = true;
            }
            // Подчёркивание активной вкладки — состояние видно без цвета.
            if active {
                let r = resp.rect;
                let line = egui::Rect::from_min_size(
                    egui::pos2(r.left(), r.bottom() + 4.0),
                    Vec2::new(r.width(), 2.0),
                );
                ui.painter().rect_filled(line, Rounding::same(1.0), t::PRIMARY);
            }
            ui.add_space(t::MD);
        }
    });
    ui.add_space(t::XS);
    changed
}
