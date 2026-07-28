//! Надёжный доступ к микрофону — многослойная стратегия.
//!
//! Слой 1 (проверка):   [`permission`] — читает глобальные тумблеры приватности Windows
//!                      из реестра (ConsentStore) и статус официального AppCapability API.
//! Слой 2 (официальный): [`winrt`] — WinRT `MediaCapture`, который на десктопных .exe
//!                      (Win10 1903+) вызывает системный запрос доступа и уважает выбор пользователя.
//! Слой 3 (резервный):  [`wasapi`] — прямой Core Audio / WASAPI. Более «хардкорный» путь,
//!                      открывает поток захвата в обход WinRT-обёртки и служит движком записи.
//!
//! Всё, что связано с COM, живёт в фоновых потоках: UI никогда не блокируется.

pub mod permission;
pub mod wasapi;
pub mod wav;
pub mod winrt;

use std::path::PathBuf;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub use permission::{PermissionReport, PermissionState};

/// Устройство захвата (микрофон).
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// «Живой» приёмник моно-сэмплов для потокового режима (буфер растёт по мере речи).
#[derive(Clone)]
pub struct LiveCapture {
    pub buf: Arc<Mutex<Vec<f32>>>,
    pub rate: Arc<AtomicU32>,
}

/// Команды из UI в фоновый движок.
pub enum MicCommand {
    RefreshPermission,
    RequestOfficial,
    EnumerateDevices,
    StartCapture {
        device: Option<String>,
        /// Куда писать WAV; `None` — только слушать (индикатор уровня без записи).
        record: Option<PathBuf>,
        /// «Живой» буфер для потоковой диктовки.
        live: Option<LiveCapture>,
    },
    StopCapture,
    Shutdown,
}

/// События из движка в UI.
#[derive(Clone, Debug)]
pub enum MicEvent {
    Log(String),
    Permission(PermissionReport),
    Devices(Vec<DeviceInfo>),
    AccessResult {
        ok: bool,
        detail: String,
    },
    CaptureStarted {
        device: String,
        format: String,
    },
    CaptureStopped,
    Error(String),
}

/// Ручка движка: UI держит её, шлёт команды и читает события/уровень сигнала.
pub struct MicEngine {
    cmd_tx: Sender<MicCommand>,
    event_rx: Receiver<MicEvent>,
    /// Текущий пиковый уровень входа (биты f32, 0.0..=1.0).
    level_bits: Arc<AtomicU32>,
    worker: Option<JoinHandle<()>>,
}

impl MicEngine {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (cmd_tx, cmd_rx) = channel::<MicCommand>();
        let (event_tx, event_rx) = channel::<MicEvent>();
        let level_bits = Arc::new(AtomicU32::new(0));

        let worker = {
            let level_bits = Arc::clone(&level_bits);
            thread::Builder::new()
                .name("tvoice-mic".into())
                .spawn(move || worker_main(cmd_rx, event_tx, ctx, level_bits))
                .expect("не удалось запустить фоновый поток микрофона")
        };

        Self {
            cmd_tx,
            event_rx,
            level_bits,
            worker: Some(worker),
        }
    }

    pub fn send(&self, cmd: MicCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    /// Забрать накопившиеся события (неблокирующе).
    pub fn drain_events(&self) -> Vec<MicEvent> {
        self.event_rx.try_iter().collect()
    }

    /// Текущий пиковый уровень 0.0..=1.0.
    /// Сам счётчик уровня: индикатор диктовки читает его напрямую и со своей частотой,
    /// не завися от того, как часто перерисовывается интерфейс.
    pub fn level_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.level_bits)
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level_bits.load(Ordering::Relaxed))
    }
}

impl Drop for MicEngine {
    fn drop(&mut self) {
        self.send(MicCommand::Shutdown);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Дескриптор активного захвата: флаг остановки + поток.
struct Capture {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl Capture {
    fn stop(self, events: &Sender<MicEvent>) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
        let _ = events.send(MicEvent::CaptureStopped);
    }
}

fn worker_main(
    cmd_rx: Receiver<MicCommand>,
    event_tx: Sender<MicEvent>,
    ctx: egui::Context,
    level_bits: Arc<AtomicU32>,
) {
    // Инициализируем COM (MTA) один раз на этот поток — нужно и WinRT, и WASAPI.
    let _com = wasapi::ComGuard::init_mta();

    let log = |s: &str| {
        let _ = event_tx.send(MicEvent::Log(s.to_string()));
    };
    log("Движок микрофона запущен (COM MTA).");

    let mut capture: Option<Capture> = None;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            MicCommand::RefreshPermission => {
                let report = permission::report();
                let _ = event_tx.send(MicEvent::Permission(report));
            }
            MicCommand::RequestOfficial => {
                log("Официальный запрос доступа через WinRT MediaCapture…");
                match winrt::request_microphone_access() {
                    Ok((ok, detail)) => {
                        let _ = event_tx.send(MicEvent::AccessResult { ok, detail });
                    }
                    Err(e) => {
                        let _ = event_tx.send(MicEvent::AccessResult {
                            ok: false,
                            detail: format!("Ошибка WinRT: {e}"),
                        });
                    }
                }
                // После запроса статус в реестре мог измениться.
                let _ = event_tx.send(MicEvent::Permission(permission::report()));
            }
            MicCommand::EnumerateDevices => match wasapi::enumerate_capture_devices() {
                Ok(devs) => {
                    let _ = event_tx.send(MicEvent::Log(format!("Найдено устройств: {}", devs.len())));
                    let _ = event_tx.send(MicEvent::Devices(devs));
                }
                Err(e) => {
                    let _ = event_tx.send(MicEvent::Error(format!("Перечисление устройств: {e}")));
                }
            },
            MicCommand::StartCapture { device, record, live } => {
                if let Some(c) = capture.take() {
                    c.stop(&event_tx);
                }
                let stop = Arc::new(AtomicBool::new(false));
                let level = Arc::clone(&level_bits);
                let ev = event_tx.clone();
                let stop_thread = Arc::clone(&stop);
                let handle = thread::Builder::new()
                    .name("tvoice-capture".into())
                    .spawn(move || {
                        if let Err(e) =
                            wasapi::run_capture(device, record, live, stop_thread, level, ev.clone())
                        {
                            let _ = ev.send(MicEvent::Error(format!("Захват (WASAPI): {e}")));
                        }
                    })
                    .expect("не удалось запустить поток захвата");
                capture = Some(Capture { stop, handle });
            }
            MicCommand::StopCapture => {
                if let Some(c) = capture.take() {
                    c.stop(&event_tx);
                    level_bits.store(0, Ordering::Relaxed);
                }
            }
            MicCommand::Shutdown => {
                if let Some(c) = capture.take() {
                    c.stop(&event_tx);
                }
                break;
            }
        }
        // Разбудить UI, чтобы он тут же отрисовал новые события.
        ctx.request_repaint();
    }
    log("Движок микрофона остановлен.");
}
