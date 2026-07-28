// TVOICE — минимальный тёмный интерфейс + надёжный доступ к микрофону на Windows 11.
//
// В debug-сборке консоль остаётся видимой (удобно смотреть логи),
// в release — прячется, чтобы приложение выглядело как обычная GUI-программа.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod control;
mod dictation;
mod hotkey;
mod inject;
#[macro_use]
mod log;
mod mic;
mod models;
mod overlay;
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

    // Диагностический режим без GUI: `tvoice --probe` печатает статус доступа
    // и список устройств захвата. Удобно для проверки на «капризной» системе.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--probe") {
        probe();
        return Ok(());
    }
    if args.iter().any(|a| a == "--net-check") {
        net_check();
        return Ok(());
    }
    if args.iter().any(|a| a == "--selftest-stt") {
        selftest_stt();
        return Ok(());
    }
    if args.iter().any(|a| a == "--stream-test") {
        stream_test();
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--vad-test") {
        let secs = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(6);
        vad_test(secs);
        return Ok(());
    }
    // Симуляция правок: `--rewrite-sim <черновик>…` печатает, сколько символов
    // стёрлось бы и вставилось на каждом уточнении. Без окна и без вставки.
    if let Some(i) = args.iter().position(|a| a == "--rewrite-sim") {
        let drafts: Vec<String> = args[(i + 1).min(args.len())..].to_vec();
        for (n, (back, add, shown)) in streaming::simulate(&drafts).iter().enumerate() {
            println!("{n}: -{back} +{add} → {shown:?}");
        }
        return Ok(());
    }

    // Автотест уточнения: подаём последовательность «черновиков» whisper и правим
    // вставленный текст той же логикой, что и поток. В окне должна остаться последняя версия.
    // `--inject-rewrite <окно> <черновик>…`
    if let Some(i) = args.iter().position(|a| a == "--inject-rewrite") {
        let need = args.get(i + 1).cloned().unwrap_or_default();
        let drafts: Vec<String> = args[(i + 2).min(args.len())..].to_vec();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let fg = inject::foreground();
        if !need.is_empty() && !fg.contains(need.as_str()) {
            logln!("inject-rewrite: активно {fg}, а нужно {need:?} — тест отменён");
            return Ok(());
        }
        logln!("inject-rewrite: приёмник {fg}, черновиков {}", drafts.len());
        inject::ctrl_combo(0x41); // Ctrl+A — пишем поверх прошлого прогона
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut shown = String::new();
        for d in &drafts {
            let keep = streaming::common_prefix(&shown, d);
            let back = shown.chars().count() - keep;
            let add: String = d.chars().skip(keep).collect();
            inject::replace_text(back, &add);
            shown = d.clone();
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        inject::ctrl_combo(0x53); // Ctrl+S
        std::thread::sleep(std::time::Duration::from_millis(700));
        logln!("inject-rewrite: завершено");
        return Ok(());
    }

    // Автотест потоковой вставки: несколько кусков с паузами через ту же очередь,
    // что и настоящая диктовка. `--inject-seq <пауза_мкс> <зазор_мс> <окно> <кусок>…`
    if let Some(i) = args.iter().position(|a| a == "--inject-seq") {
        let delay_us = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(8000);
        let gap_ms: u64 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(2000);
        let need = args.get(i + 3).cloned().unwrap_or_default();
        let chunks: Vec<String> = args[(i + 4).min(args.len())..].to_vec();
        inject::set_mode(inject::MODE_AUTO);
        inject::set_char_delay_us(delay_us);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let fg = inject::foreground();
        if !need.is_empty() && !fg.contains(need.as_str()) {
            logln!("inject-seq: активно {fg}, а нужно {need:?} — тест отменён");
            return Ok(());
        }
        logln!("inject-seq: приёмник {fg}, кусков {}", chunks.len());
        inject::ctrl_combo(0x41); // Ctrl+A — пишем поверх прошлого прогона
        std::thread::sleep(std::time::Duration::from_millis(150));
        for c in &chunks {
            // Перед каждым куском сверяем окно: если фокус увели (уведомление, мессенджер),
            // тест обязан остановиться, а не печатать в чужое поле ввода.
            let now = inject::foreground();
            if !need.is_empty() && !now.contains(need.as_str()) {
                logln!("inject-seq: фокус ушёл в {now} — тест прерван");
                break;
            }
            inject::queue_text(c);
            std::thread::sleep(std::time::Duration::from_millis(gap_ms));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        inject::ctrl_combo(0x53); // Ctrl+S
        std::thread::sleep(std::time::Duration::from_millis(700));
        logln!("inject-seq: завершено");
        return Ok(());
    }

    // Автотест вставки: `--inject-test <keys|clip|auto> <пауза_мкс> <текст>`.
    // Ждём 1.5с (успеть сфокусировать окно-приёмник), вставляем, затем шлём Ctrl+S,
    // чтобы приёмник (Блокнот) сохранил файл и результат можно было сверить.
    if let Some(i) = args.iter().position(|a| a == "--inject-test") {
        let mode = args.get(i + 1).map(String::as_str).unwrap_or("keys");
        let delay_us = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(2000);
        let text = args
            .get(i + 3)
            .cloned()
            .unwrap_or_else(|| "Проверка ввода TVOICE — привет 123".into());
        inject::set_mode(match mode {
            "clip" => inject::MODE_CLIPBOARD,
            "auto" => inject::MODE_AUTO,
            _ => inject::MODE_KEYS,
        });
        inject::set_char_delay_us(delay_us);
        logln!("inject-test: режим={mode} пауза={delay_us}мкс текст={text:?}");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        // Предохранитель: тест жмёт Ctrl+A и печатает поверх выделения, поэтому
        // работаем только с ожидаемым окном-приёмником (иначе затрём чужой документ).
        let fg = inject::foreground();
        let need = args.iter().position(|a| a == "--window").and_then(|i| args.get(i + 1));
        if let Some(need) = need {
            if !fg.contains(need.as_str()) {
                logln!("inject-test: активно {fg}, а нужно {need:?} — тест отменён");
                return Ok(());
            }
        }
        logln!("inject-test: приёмник {fg}");
        inject::ctrl_combo(0x41); // Ctrl+A — прошлый прогон заменяем, а не дописываем
        std::thread::sleep(std::time::Duration::from_millis(150));
        inject::type_text(&text);
        std::thread::sleep(std::time::Duration::from_millis(300));
        inject::ctrl_combo(0x53); // Ctrl+S
        std::thread::sleep(std::time::Duration::from_millis(700));
        logln!("inject-test: завершено");
        return Ok(());
    }
    if args.iter().any(|a| a == "--rec-test") {
        let secs = args
            .iter()
            .position(|a| a == "--rec-test")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(3);
        record_test(secs);
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

/// Headless-самопроверка доступа к микрофону.
fn probe() {
    let _com = mic::wasapi::ComGuard::init_mta();

    println!("== TVOICE probe ==");
    let report = mic::permission::report();
    println!("Итоговый статус: {}", report.effective.label());
    for line in &report.details {
        println!("  {line}");
    }

    match mic::wasapi::enumerate_capture_devices() {
        Ok(devs) => {
            println!("Устройств захвата: {}", devs.len());
            for d in &devs {
                let mark = if d.is_default { " * default" } else { "" };
                println!("  - {}{mark}\n    id: {}", d.name, d.id);
            }
        }
        Err(e) => println!("Ошибка перечисления устройств: {e}"),
    }

    println!("-- скан инициализации захвата --");
    match mic::wasapi::scan_capture_init() {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => println!("scan error: {e}"),
    }
}

/// Проверка сетевых источников (без больших загрузок).
fn net_check() {
    println!("== TVOICE net-check ==");
    match models::download::find_binary_zip_url() {
        Ok(url) => println!("whisper.cpp zip: {url}"),
        Err(e) => println!("whisper.cpp zip: ОШИБКА {e}"),
    }
    let m = &models::CATALOG[0];
    let url = m.url();
    match ureq::head(&url).set("User-Agent", "tvoice").call() {
        Ok(r) => println!(
            "модель {} → HTTP {} (Content-Length: {})",
            m.file,
            r.status(),
            r.header("Content-Length").unwrap_or("?")
        ),
        Err(e) => println!("модель {}: ОШИБКА {e}", m.file),
    }
}

/// Полный сквозной самотест STT: скачать движок+tiny-модель, записать 3с, распознать.
fn selftest_stt() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::Arc;

    println!("== TVOICE selftest-stt ==");
    let ctx = egui::Context::default();
    let dl = models::new_shared();

    if models::whisper_exe().is_none() {
        println!("Скачиваю whisper.cpp…");
        models::download::start_whisper_binary(false, dl.clone(), ctx.clone());
        wait_download(&dl);
    }
    println!("whisper.exe: {:?}", models::whisper_exe());

    let tiny = &models::CATALOG[0];
    if !models::is_downloaded(tiny.file) {
        println!("Скачиваю {} …", tiny.file);
        models::download::start_model(tiny, dl.clone(), ctx.clone());
        wait_download(&dl);
    }
    println!("модель {} загружена: {}", tiny.file, models::is_downloaded(tiny.file));

    let temp = models::app_dir().join("temp");
    let _ = std::fs::create_dir_all(&temp);
    let wav = temp.join("selftest.wav");

    println!("Запись 3с — говорите в микрофон…");
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let (tx, _rx) = channel();
    let stop_t = Arc::clone(&stop);
    let wavc = wav.clone();
    let h = std::thread::spawn(move || {
        let _ = mic::wasapi::run_capture(None, Some(wavc), None, stop_t, level, tx);
    });
    std::thread::sleep(std::time::Duration::from_secs(3));
    stop.store(true, Ordering::Relaxed);
    let _ = h.join();

    let wav16 = temp.join("selftest16k.wav");
    if let Err(e) = audio::wav_to_16k_mono(&wav, &wav16) {
        println!("ресемпл: ОШИБКА {e}");
        return;
    }
    // Дважды через резидентный сервер: 1-й запуск включает старт сервера, 2-й — «тёплый».
    for pass in 1..=2 {
        let t0 = std::time::Instant::now();
        match server::transcribe(&wav16, tiny.file, "ru") {
            Ok(t) => println!(
                "проход {pass}: «{t}» за {:.2}с",
                t0.elapsed().as_secs_f32()
            ),
            Err(e) => println!("проход {pass}: ОШИБКА {e}"),
        }
    }
    server::shutdown();
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&wav16);
}

/// Диагностика микрофона и VAD: пишем `secs` секунд и раскладываем запись по полочкам —
/// частота, уровень фона, порог, найденные участки речи. Всё уходит в tvoice.log.
fn vad_test(secs: u64) {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    println!("== TVOICE vad-test: говорите {secs} с ==");
    let live = mic::LiveCapture {
        buf: Arc::new(Mutex::new(Vec::new())),
        rate: Arc::new(AtomicU32::new(16_000)),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let (tx, _rx) = channel();
    let (stop_t, live_t, level_t) = (stop.clone(), live.clone(), level.clone());
    let h = std::thread::spawn(move || {
        let _ = mic::wasapi::run_capture(None, None, Some(live_t), stop_t, level_t, tx);
    });
    std::thread::sleep(std::time::Duration::from_secs(secs));
    stop.store(true, Ordering::Relaxed);
    let _ = h.join();

    let (samples, rate) = {
        let b = live.buf.lock().unwrap();
        (b.clone(), live.rate.load(Ordering::Relaxed).max(1) as usize)
    };
    logln!(
        "vad-test: {} сэмплов @ {rate} Гц = {:.1}с",
        samples.len(),
        samples.len() as f32 / rate as f32
    );

    let mut v = vad::Vad::new(rate);
    v.feed(&samples);
    let seg = v.segment(0);
    let s = |n: usize| n as f32 / rate as f32;
    logln!(
        "vad-test: шум={:.4}±{:.4} порог={:.4} | речи {:.1}с, конец речи {:.1}с, хвост тишины {:.1}с",
        v.noise(),
        v.dev(),
        v.on(),
        s(seg.speech),
        s(seg.speech_end),
        s(seg.silence)
    );

    // Профиль громкости по полсекунды — видно, где была речь, а где фон.
    let step = rate / 2;
    let mut line = String::new();
    for (i, chunk) in samples.chunks(step.max(1)).enumerate() {
        let rms = (chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len().max(1) as f32).sqrt();
        line.push_str(&format!("{:.1}с={:.4} ", i as f32 / 2.0, rms));
    }
    logln!("vad-test: профиль по 0.5с: {line}");

    // Тот же звук, что ушёл бы в whisper.
    let tmp = models::app_dir().join("temp");
    let _ = std::fs::create_dir_all(&tmp);
    let wav = tmp.join("vadtest16k.wav");
    match audio::write_16k_wav_from_mono(&samples, rate as u32, &wav) {
        Ok(()) => logln!("vad-test: wav сохранён: {}", wav.display()),
        Err(e) => logln!("vad-test: ресемпл ОШИБКА: {e}"),
    }
    println!("готово, подробности в tvoice.log");
}

/// Проверка «живого» буфера: захват 4с → ресемпл → распознавание.
fn stream_test() {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    println!("== TVOICE stream-test (говорите 4с) ==");
    let live = mic::LiveCapture {
        buf: Arc::new(Mutex::new(Vec::new())),
        rate: Arc::new(AtomicU32::new(16_000)),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let (tx, _rx) = channel();
    let (stop_t, live_t, level_t) = (stop.clone(), live.clone(), level.clone());
    let h = std::thread::spawn(move || {
        let _ = mic::wasapi::run_capture(None, None, Some(live_t), stop_t, level_t, tx);
    });
    std::thread::sleep(std::time::Duration::from_secs(4));
    stop.store(true, Ordering::Relaxed);
    let _ = h.join();

    let (samples, rate) = {
        let b = live.buf.lock().unwrap();
        (b.clone(), live.rate.load(Ordering::Relaxed))
    };
    println!("собрано сэмплов: {} @ {} Гц ({:.1} с)", samples.len(), rate, samples.len() as f32 / rate.max(1) as f32);
    let tmp = models::app_dir().join("temp");
    let _ = std::fs::create_dir_all(&tmp);
    let wav = tmp.join("streamtest16k.wav");
    if let Err(e) = audio::write_16k_wav_from_mono(&samples, rate, &wav) {
        println!("ресемпл: ОШИБКА {e}");
        return;
    }
    let tiny = &models::CATALOG[0];
    match server::transcribe(&wav, tiny.file, "ru") {
        Ok(t) => println!("РАСПОЗНАНО: «{t}»"),
        Err(e) => println!("транскрибация: ОШИБКА {e}"),
    }
    server::shutdown();
    let _ = std::fs::remove_file(&wav);
}

fn wait_download(dl: &models::SharedDownload) {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let s = dl.lock().unwrap();
        if s.active.is_none() {
            if let Some(e) = &s.error {
                println!("  ошибка загрузки: {e}");
            }
            break;
        }
    }
}

/// Headless-тест записи: пишет `secs` секунд с устройства по умолчанию в recordings/.
fn record_test(secs: u64) {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::Arc;

    let dir = std::env::current_dir().unwrap_or_default().join("recordings");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(mic::wav::timestamp_filename());

    println!("== TVOICE rec-test: {secs} с → {} ==", path.display());

    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let (tx, rx) = channel();

    let stop_t = Arc::clone(&stop);
    let level_t = Arc::clone(&level);
    let path_t = path.clone();
    let handle = std::thread::spawn(move || {
        if let Err(e) = mic::wasapi::run_capture(None, Some(path_t), None, stop_t, level_t, tx) {
            eprintln!("Ошибка записи: {e}");
        }
    });

    for _ in 0..secs * 4 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        println!("  уровень: {:.3}", f32::from_bits(level.load(Ordering::Relaxed)));
    }
    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();

    for ev in rx.try_iter() {
        if let mic::MicEvent::Log(s) | mic::MicEvent::Error(s) = ev {
            println!("  {s}");
        }
    }
}
