//! Каркас приложения: состояние и жизненный цикл кадра.
//!
//! Оформление и разметка живут отдельно: токены — в theme.rs, общие элементы — в
//! ui_kit.rs, оболочка и экраны — в ui_shell.rs, ui_main.rs, ui_engine.rs,
//! ui_input.rs, ui_system.rs.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;


use crate::dictation::{self, SharedDictation};
use crate::hotkey::Hotkey;
use crate::mic::{DeviceInfo, MicCommand, MicEngine, PermissionReport};
use crate::models::{self, SharedDownload};
use crate::overlay::Overlay;
use crate::tray::Tray;

/// Экран приложения.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Main,
    Settings,
}

/// Раздел настроек.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    Engine,
    Hotkeys,
    Privacy,
}

pub struct TvoiceApp {
    pub(crate) ctx: egui::Context,
    pub(crate) engine: MicEngine,
    pub(crate) route: Route,
    pub(crate) settings_tab: SettingsTab,

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
    /// Громкость, приведённая к 0…1 тем же способом, что и в индикаторе у курсора.
    pub(crate) voice: f32,
    /// Плавающая оценка тишины для этого приведения.
    pub(crate) voice_floor: f32,

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
    /// Значок в трее: приложение продолжает работать со спрятанным окном.
    pub(crate) tray: Tray,
    /// Окно спрятано в трей.
    pub(crate) hidden: bool,
    /// Выход запрошен из трея — крестик такой запрос перехватывать не должен.
    pub(crate) quitting: bool,
    /// Когда был прошлый кадр — чтобы заметить, что цикл со спрятанным окном встал.
    pub(crate) last_frame: std::time::Instant,
    /// Где стояло окно до сворачивания — туда и вернём.
    pub(crate) window_pos: egui::Pos2,
    /// Первый кадр: привести панель задач в соответствие состоянию окна.
    pub(crate) first_frame: bool,
    /// Прятать окно в трей при запуске.
    pub(crate) start_in_tray: bool,
    /// Запускаться вместе с Windows. В `config.json` не хранится — истина в реестре,
    /// см. `autostart`.
    pub(crate) autostart: bool,
    /// Язык интерфейса (не путать с языком распознавания `lang`).
    pub(crate) ui_lang: crate::lang::Lang,
    /// Где показывать индикатор диктовки.
    pub(crate) hud_anchor: crate::overlay::Anchor,
    /// Размер индикатора: 1.0 — базовый.
    pub(crate) hud_scale: f32,
    /// Картинка предпросмотра индикатора в настройках.
    pub(crate) hud_texture: Option<egui::TextureHandle>,
    /// Настройки изменились — сохранить в конце кадра.
    pub(crate) dirty: bool,
    /// Был ли хоткей в режиме захвата в прошлом кадре (для сохранения по завершении).
    pub(crate) was_capturing: bool,
    /// Шла ли загрузка движка в прошлом кадре (чтобы перезапустить сервер по завершении).
    pub(crate) was_downloading_engine: bool,
}

impl TvoiceApp {
    /// `hidden` — прятать ли окно сразу: это решает `main` (настройка либо флаг `--tray`
    /// от автозапуска), и состояние приложения обязано с ним совпадать, иначе окно уедет
    /// за экран, а приложение будет считать его видимым.
    pub fn new(ctx: egui::Context, hidden: bool) -> Self {
        let ctx_for_tray = ctx.clone();
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
            route: Route::Main,
            settings_tab: SettingsTab::Engine,
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
            voice: 0.0,
            voice_floor: 0.01,
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
            tray: Tray::spawn(
                &format!("TVOICE — диктовка: {}", cfg.hotkey().label()),
                ctx_for_tray,
            ),
            hidden,
            quitting: false,
            last_frame: std::time::Instant::now(),
            window_pos: egui::pos2(200.0, 120.0),
            first_frame: true,
            start_in_tray: cfg.start_in_tray,
            autostart: crate::autostart::is_enabled(),
            ui_lang: cfg.ui_lang(),
            hud_anchor: crate::overlay::Anchor::from_id(&cfg.hud_anchor),
            hud_scale: cfg.hud_scale,
            hud_texture: None,
            dirty: false,
            was_capturing: false,
            was_downloading_engine: false,
        };
        crate::lang::set(app.ui_lang);
        crate::inject::set_mode(app.paste_mode);
        crate::inject::set_char_delay_us(app.char_delay_us);
        crate::overlay::set_anchor(crate::overlay::Anchor::from_id(&cfg.hud_anchor));
        crate::overlay::set_scale(cfg.hud_scale);
        // Индикатор читает громкость сам, минуя цикл интерфейса: в трее тот идёт
        // 10 кадров в секунду, и пульсация от него получалась вялой.
        app.overlay.attach_level(app.engine.level_handle());
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
        let s: String = s.into();
        // Дублируем в файл: со свёрнутым в трей окном список в интерфейсе никто не видит,
        // а ошибки захвата микрофона выглядят как «хоткей сработал, но ничего не произошло».
        crate::logln!("ui: {s}");
        self.log.push(s);
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
            start_in_tray: self.start_in_tray,
            ui_lang: self.ui_lang.id().to_string(),
            hud_anchor: self.hud_anchor.id().to_string(),
            hud_scale: self.hud_scale,
        };
        crate::config::save(&cfg);
    }
}

impl eframe::App for TvoiceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events();
        self.handle_hotkey();
        self.handle_tray(ctx);

        // Завершился захват хоткея → сохранить новую комбинацию.
        let capturing = self.hotkey.is_capturing();
        if self.was_capturing && !capturing {
            self.dirty = true;
            // В подсказке значка держим актуальную комбинацию — из трея её больше негде увидеть.
            self.tray
                .set_tip(&format!("TVOICE — диктовка: {}", self.hotkey.label()));
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
        // Приведение к 0…1: постоянный множитель тут не работает, у пика фона и пика
        // речи разница всего в несколько раз (см. overlay::voice_amount).
        let voice = crate::overlay::voice_amount(target, &mut self.voice_floor);
        self.voice = if voice > self.voice {
            voice
        } else {
            self.voice * 0.6 + voice * 0.4
        };

        // Поток распознавания мог закончить сам (долгое молчание) — тогда захват, состояние
        // «диктую» и индикатор снимаем здесь: поток до них не достаёт.
        if self.dictating {
            let idle = self
                .dictation
                .lock()
                .map(|s| s.auto_stop)
                .unwrap_or(false);
            if idle {
                crate::logln!("диктовка остановлена по тишине");
                if let Ok(mut s) = self.dictation.lock() {
                    s.auto_stop = false;
                }
                self.stop_dictation(); // обратный сигнал сыграет он сам
            }
        }

        self.shell(ctx);

        // Индикатор у курсора во время диктовки (нативное окно в своём потоке).
        self.overlay.set_level(self.level);
        self.overlay.set_visible(self.dictating);
        // Отсюда же `sound` узнаёт, входит нажатие хоткея в захват или выходит из него.
        // Одно место на оба признака: новый путь остановки нельзя забыть учесть.
        crate::sound::set_active(self.dictating);

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

        // Постоянно опрашиваем: гарантирует обработку хоткея даже когда окно не в фокусе
        // и когда его вообще не видно (свёрнуто в трей).
        self.keep_alive(ctx);
    }
}

impl Drop for TvoiceApp {
    fn drop(&mut self) {
        // Гасим резидентный whisper-server (иначе процесс остался бы висеть).
        crate::server::shutdown();
    }
}
