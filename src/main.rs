// TVOICE — минимальный тёмный интерфейс + надёжный доступ к микрофону на Windows 11.
//
// В debug-сборке консоль остаётся видимой (удобно смотреть логи),
// в release — прячется, чтобы приложение выглядело как обычная GUI-программа.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Объявлен первым: макрос logln! виден только модулям, объявленным после него.
#[macro_use]
mod log;

mod app;
mod audio;
mod caret;
mod cli;
mod config;
mod control;
mod dictation;
mod hotkey;
mod hud;
mod inject;
mod mic;
mod models;
mod overlay;
mod selftest;
mod server;
mod streaming;
mod theme;
mod transcribe;
mod tray;
mod ui_dictation;
mod ui_mic;
mod ui_models;
mod userinput;
mod vad;

use app::TvoiceApp;

fn main() -> eframe::Result<()> {
    log::install_panic_hook();

    // Диагностика без GUI: --probe, --vad-test, --caret-test, --inject-* и прочее (cli.rs).
    let args: Vec<String> = std::env::args().collect();
    if cli::dispatch(&args) {
        return Ok(());
    }

    logln!("=== TVOICE v{} запуск GUI ===", env!("CARGO_PKG_VERSION"));

    // Компактное окно: приложение фоновое, живёт в трее и работает по хоткею.
    // «Свёрнуто в трей» = окно за экраном, а не спрятанное: спрятанное окно
    // останавливает цикл eframe, и хоткей с меню трея перестают отвечать.
    let hidden = config::load().start_in_tray;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TVOICE")
            .with_inner_size([400.0, 600.0])
            .with_min_inner_size([360.0, 460.0])
            .with_position(if hidden {
                egui::pos2(-32000.0, -32000.0)
            } else {
                egui::pos2(200.0, 120.0)
            })
            .with_app_id("dev.pith.tvoice"),
        ..Default::default()
    };

    eframe::run_native(
        "TVOICE",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(TvoiceApp::new(cc.egui_ctx.clone())))
        }),
    )
}
