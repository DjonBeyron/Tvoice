//! Каркас интерфейса TVOICE: вкладки, состояние, диспетчеризация, общие помощники.
//! Отрисовка вкладок вынесена в ui_mic.rs / ui_models.rs / ui_dictation.rs.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use egui::{Color32, RichText, Rounding, Sense, Stroke, Vec2};

use crate::dictation::{self, SharedDictation};
use crate::hotkey::Hotkey;
use crate::mic::{DeviceInfo, MicCommand, MicEngine, PermissionReport};
use crate::models::{self, SharedDownload};
use crate::overlay::Overlay;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Mic,
    Models,
    Dictation,
}

pub struct TvoiceApp {
    pub(crate) ctx: egui::Context,
    pub(crate) engine: MicEngine,
    pub(crate) tab: Tab,

    // --- вкладка «Микрофон» ---
    pub(crate) permission: Option<PermissionReport>,
    pub(crate) devices: Vec<DeviceInfo>,
    pub(crate) selected: Option<String>,
    pub(crate) capturing: bool,
    pub(crate) capture_info: Option<(String, String)>,
    pub(crate) last_official: Option<(bool, String)>,
    pub(crate) show_details: bool,
    pub(crate) record_enabled: bool,
    pub(crate) last_record_path: Option<PathBuf>,
    pub(crate) log: Vec<String>,
    pub(crate) level: f32,

    // --- вкладка «Модели» ---
    pub(crate) downloads: SharedDownload,
    pub(crate) selected_model: Option<String>,

    // --- вкладка «Диктовка» ---
    pub(crate) hotkey: Hotkey,
    pub(crate) dictation: SharedDictation,
    pub(crate) lang: String,
    pub(crate) insert_enabled: bool,
    pub(crate) dictating: bool,
    pub(crate) dictation_temp: Option<PathBuf>,
    /// Вставлять ли результат текущего сеанса (хоткей — по галочке, тест-кнопка — нет).
    pub(crate) dictation_insert: bool,
    pub(crate) streaming_enabled: bool,
    /// Способ вставки текста (см. `inject::MODE_*`).
    pub(crate) paste_mode: u8,
    /// Пауза между символами при печати, мкс (только из config.json).
    pub(crate) char_delay_us: u32,
    /// Флаг остановки активного потокового распознавания (Some — поток идёт).
    pub(crate) stream_stop: Option<Arc<AtomicBool>>,
    pub(crate) overlay: Overlay,
    /// Настройки изменились — сохранить в конце кадра.
    pub(crate) dirty: bool,
    /// Был ли хоткей в режиме захвата в прошлом кадре (для сохранения по завершении).
    pub(crate) was_capturing: bool,
    /// Шла ли загрузка движка в прошлом кадре (чтобы перезапустить сервер по завершении).
    pub(crate) was_downloading_engine: bool,
}

impl TvoiceApp {
    pub fn new(ctx: egui::Context) -> Self {
        let engine = MicEngine::spawn(ctx.clone());
        engine.send(MicCommand::RefreshPermission);
        engine.send(MicCommand::EnumerateDevices);

        // Загружаем сохранённые настройки.
        let cfg = crate::config::load();

        // Хоткей из конфига.
        let hotkey = Hotkey::spawn(ctx.clone());
        if let Ok(mut h) = hotkey.config.lock() {
            *h = cfg.hotkey();
        }

        // Модель: из конфига (если она скачана), иначе первая скачанная.
        let selected_model = cfg
            .model
            .clone()
            .filter(|id| {
                models::by_id(id)
                    .map(|m| models::is_downloaded(m.file))
                    .unwrap_or(false)
            })
            .or_else(|| {
                models::CATALOG
                    .iter()
                    .find(|m| models::is_downloaded(m.file))
                    .map(|m| m.id.to_string())
            });

        let app = Self {
            hotkey,
            ctx,
            engine,
            tab: Tab::Mic,
            permission: None,
            devices: Vec::new(),
            selected: None,
            capturing: false,
            capture_info: None,
            last_official: None,
            show_details: false,
            record_enabled: true,
            last_record_path: None,
            log: Vec::new(),
            level: 0.0,
            downloads: models::new_shared(),
            selected_model,
            dictation: dictation::new_shared(),
            lang: cfg.lang.clone(),
            insert_enabled: cfg.insert,
            dictating: false,
            dictation_temp: None,
            dictation_insert: true,
            streaming_enabled: cfg.streaming,
            paste_mode: cfg.paste_mode,
            char_delay_us: cfg.char_delay_us,
            stream_stop: None,
            overlay: Overlay::spawn(),
            dirty: false,
            was_capturing: false,
            was_downloading_engine: false,
        };
        crate::inject::set_mode(app.paste_mode);
        crate::inject::set_char_delay_us(app.char_delay_us);
        // Прогреваем whisper-server заранее, чтобы первая диктовка была быстрой.
        if let Some(id) = &app.selected_model {
            if let Some(m) = models::by_id(id) {
                if models::is_downloaded(m.file) {
                    crate::server::prewarm(m.file.to_string(), app.lang.clone());
                }
            }
        }
        app
    }

    pub(crate) fn push_log(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 200 {
            let overflow = self.log.len() - 200;
            self.log.drain(0..overflow);
        }
    }

    /// Папка записей рядом с исполняемым файлом.
    pub(crate) fn recordings_dir() -> PathBuf {
        models::app_dir().join("recordings")
    }

    /// Сохранить текущие настройки (хоткей, модель, язык, вставка) в config.json.
    pub(crate) fn persist(&self) {
        let h = self.hotkey.config.lock().map(|c| c.clone()).unwrap_or_default();
        let cfg = crate::config::Config {
            ctrl: h.ctrl,
            alt: h.alt,
            shift: h.shift,
            win: h.win,
            vk: h.vk,
            key_name: h.key_name,
            model: self.selected_model.clone(),
            lang: self.lang.clone(),
            insert: self.insert_enabled,
            streaming: self.streaming_enabled,
            paste_mode: self.paste_mode,
            char_delay_us: self.char_delay_us,
        };
        crate::config::save(&cfg);
    }
}

impl eframe::App for TvoiceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events();
        self.handle_hotkey();

        // Завершился захват хоткея → сохранить новую комбинацию.
        let capturing = self.hotkey.is_capturing();
        if self.was_capturing && !capturing {
            self.dirty = true;
        }
        self.was_capturing = capturing;

        // Завершилась загрузка движка → перезапустить сервер на новом движке (в т.ч. GPU),
        // чтобы сразу подтвердить, что он заводится (виден лог server: запуск/готов).
        let dl_engine = self
            .downloads
            .lock()
            .map(|d| matches!(d.active.as_deref(), Some("whisper.exe") | Some("whisper-gpu")))
            .unwrap_or(false);
        if self.was_downloading_engine && !dl_engine {
            if let Some(file) = self
                .selected_model
                .as_ref()
                .and_then(|id| models::by_id(id))
                .filter(|m| models::is_downloaded(m.file))
                .map(|m| m.file.to_string())
            {
                crate::server::shutdown();
                crate::server::prewarm(file, self.lang.clone());
                self.push_log("Движок обновлён — перезапускаю распознавание.");
            }
        }
        self.was_downloading_engine = dl_engine;

        // Скрытая самопроверка вставки: TVOICE_INSERT_TEST авто-прогоняет диктовку со вставкой.
        {
            use std::sync::atomic::{AtomicU8, Ordering};
            use std::sync::OnceLock;
            static START: OnceLock<std::time::Instant> = OnceLock::new();
            static STEP: AtomicU8 = AtomicU8::new(0);
            if std::env::var("TVOICE_INSERT_TEST").is_ok() {
                let el = START.get_or_init(std::time::Instant::now).elapsed().as_millis();
                if el > 1000 && STEP.load(Ordering::Relaxed) == 0 {
                    STEP.store(1, Ordering::Relaxed);
                    let insert = std::env::var("TVOICE_TEST_NOINSERT").is_err();
                    self.begin_dictation(insert);
                } else if el > 3500 && STEP.load(Ordering::Relaxed) == 1 {
                    STEP.store(2, Ordering::Relaxed);
                    self.stop_dictation();
                }
            }
        }

        // Плавное считывание уровня из движка.
        let target = self.engine.level();
        self.level = if target > self.level {
            target
        } else {
            self.level * 0.85 + target * 0.15
        };

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("TVOICE")
                        .size(20.0)
                        .strong()
                        .color(Color32::from_rgb(0xF2, 0xF5, 0xF9)),
                );
                ui.add_space(10.0);
                tab_button(ui, &mut self.tab, Tab::Mic, "Микрофон");
                tab_button(ui, &mut self.tab, Tab::Models, "Модели");
                tab_button(ui, &mut self.tab, Tab::Dictation, "Диктовка");
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            match self.tab {
                Tab::Mic => self.mic_tab(ui),
                Tab::Models => self.models_tab(ui),
                Tab::Dictation => self.dictation_tab(ui),
            }
        });

        // Индикатор у курсора во время диктовки (нативное окно в своём потоке).
        self.overlay.set_level(self.level);
        self.overlay.set_visible(self.dictating);

        // Сохраняем настройки, если менялись, и прогреваем сервер под новую модель/язык.
        if self.dirty {
            self.persist();
            self.dirty = false;
            if let Some(id) = &self.selected_model {
                if let Some(m) = models::by_id(id) {
                    if models::is_downloaded(m.file) {
                        crate::server::prewarm(m.file.to_string(), self.lang.clone());
                    }
                }
            }
        }

        // Постоянно опрашиваем: гарантирует обработку хоткея даже когда окно не в фокусе.
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }
}

impl Drop for TvoiceApp {
    fn drop(&mut self) {
        // Гасим резидентный whisper-server (иначе процесс остался бы висеть).
        crate::server::shutdown();
    }
}

fn tab_button(ui: &mut egui::Ui, current: &mut Tab, tab: Tab, label: &str) {
    let selected = *current == tab;
    let text = if selected {
        RichText::new(label).strong().color(theme::ACCENT)
    } else {
        RichText::new(label).color(theme::MUTED)
    };
    if ui.selectable_label(selected, text).clicked() {
        *current = tab;
    }
}

// --- общие UI-помощники (используются вкладками) ---

/// «Карточка» с фоном и обводкой.
pub(crate) fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(theme::PANEL)
        .stroke(Stroke::new(1.0_f32, theme::LINE))
        .rounding(Rounding::same(12.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub(crate) fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 6.5, color.linear_multiply(0.25));
    ui.painter().circle_filled(rect.center(), 4.5, color);
}

/// Горизонтальный индикатор уровня.
pub(crate) fn meter_bar(ui: &mut egui::Ui, level: f32) {
    let desired = Vec2::new(ui.available_width(), 18.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(6.0), Color32::from_rgb(0x0D, 0x0F, 0x12));
    let level = level.clamp(0.0, 1.0);
    if level > 0.0 {
        let mut fill = rect;
        fill.set_width(rect.width() * level);
        let color = if level < 0.6 {
            theme::OK
        } else if level < 0.85 {
            theme::WARN
        } else {
            theme::BAD
        };
        painter.rect_filled(fill, Rounding::same(6.0), color);
    }
    painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0_f32, theme::LINE));
}
