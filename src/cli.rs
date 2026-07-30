//! Диагностические режимы командной строки: работают без GUI и служат для проверки
//! железа и вставки на живой системе. Вынесены из `main.rs`, чтобы тот оставался
//! коротким: запуск приложения и ничего больше.

use crate::{audio, mic, models, selftest, server, vad};

/// Разобрать аргументы и выполнить диагностический режим.
/// `true` — режим отработал, приложение запускать не нужно.
pub fn dispatch(args: &[String]) -> bool {
        // Качество распознавания: `--stt-bench <файл.wav> <эталонный текст>`.
        if let Some(i) = args.iter().position(|a| a == "--stt-bench") {
            let wav = args.get(i + 1).cloned().unwrap_or_default();
            let reference = args.get(i + 2).cloned().unwrap_or_default();
            let model = args.get(i + 3).cloned();
            crate::bench::run(std::path::Path::new(&wav), &reference, model.as_deref());
            crate::server::shutdown();
            return true;
        }

        if selftest::dispatch(args) {
            return true;
        }
        // `tvoice --probe` печатает статус доступа к микрофону и список устройств
        // захвата. Удобно для проверки на «капризной» системе.
        if args.iter().any(|a| a == "--probe") {
            probe();
            return true;
        }
        if args.iter().any(|a| a == "--net-check") {
            net_check();
            return true;
        }
        if args.iter().any(|a| a == "--selftest-stt") {
            selftest_stt();
            return true;
        }
        if args.iter().any(|a| a == "--stream-test") {
            stream_test();
            return true;
        }
        if let Some(i) = args.iter().position(|a| a == "--vad-test") {
            let secs = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(6);
            vad_test(secs);
            return true;
        }
        // Автовыход по тишине: `--idle-test`. Кормим поток распознавания тишиной и смотрим,
        // через сколько он закончит сам. Ни микрофон, ни модель не нужны: на тишине
        // распознавание не вызывается ни разу, а проверяем мы именно правило по времени.
        if let Some(i) = args.iter().position(|a| a == "--idle-test") {
            idle_test(args.get(i + 1).filter(|s| !s.starts_with('-')).cloned());
            return true;
        }

        // Сигнал старта диктовки: `--sound-test [раз]`. Печатает, за сколько возвращается
        // `play` — то есть насколько нажатие хоткея задержалось бы, если играть в его потоке.
        if let Some(i) = args.iter().position(|a| a == "--sound-test") {
            let times: usize = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(3);
            println!("== сигнал старта: {times} раз ==");
            let t0 = std::time::Instant::now();
            crate::sound::prewarm();
            println!("подготовка: {:.0} мс", t0.elapsed().as_secs_f32() * 1000.0);
            // Играем парами «вход → выход»: слышно, что обратный сигнал действительно
            // обратный, и видно, что оба вызова не блокируют вызывающего.
            for n in 1..=times {
                for name in ["вход", "выход"] {
                    let t = std::time::Instant::now();
                    if name == "вход" {
                        crate::sound::set_active(false);
                        crate::sound::play_enter();
                    } else {
                        crate::sound::play_exit();
                    }
                    println!(
                        "  #{n} {name}: вызов {:.2} мс",
                        t.elapsed().as_secs_f32() * 1000.0
                    );
                    std::thread::sleep(std::time::Duration::from_millis(800));
                }
            }
            return true;
        }

        // Автозапуск: `--autostart status|on|off`. Тем же кодом, что и галочка в настройках.
        if let Some(i) = args.iter().position(|a| a == "--autostart") {
            match args.get(i + 1).map(String::as_str) {
                Some("on") | Some("off") => {
                    let on = args[i + 1] == "on";
                    match crate::autostart::set(on) {
                        Ok(()) => println!("автозапуск: {}", if on { "включён" } else { "выключен" }),
                        Err(e) => println!("не изменить автозапуск: {e}"),
                    }
                }
                _ => println!(
                    "автозапуск: {}",
                    if crate::autostart::is_enabled() { "включён" } else { "выключен" }
                ),
            }
            return true;
        }

        // Разбор VAD по готовому файлу: `--vad-file <wav>`.
        //
        // В отличие от `--vad-test`, микрофон не трогает: прогон повторяем, поэтому им можно
        // сравнивать поведение VAD до и после правки на одном и том же звуке. Без этого
        // «стало лучше» проверить нечем — порог живёт от уровня фона, а фон каждый раз свой.
        if let Some(i) = args.iter().position(|a| a == "--vad-file") {
            let path = args.get(i + 1).cloned().unwrap_or_default();
            vad_file(&path);
            return true;
        }
        if args.iter().any(|a| a == "--rec-test") {
            let secs = args
                .iter()
                .position(|a| a == "--rec-test")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(3);
            record_test(secs);
            return true;
        }
    false
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
/// Проверить, что поток распознавания заканчивает сам после долгого молчания.
fn idle_test(wav: Option<String>) {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    // Без файла — чистая тишина (проверяем само правило). С файлом — реальная запись:
    // так видно, что счётчик молчания начинает идти именно после конца речи, а не
    // застревает из-за разметки VAD.
    let (source, rate) = match &wav {
        Some(path) => match audio::read_wav_mono(std::path::Path::new(path)) {
            Ok((s, r)) => {
                println!("== автовыход: подаю {path} ==");
                (s, r.max(1))
            }
            Err(e) => {
                println!("не прочитать {path}: {e}");
                return;
            }
        },
        None => {
            println!("== автовыход: кормлю поток тишиной ==");
            (Vec::new(), 16_000)
        }
    };
    let feed_rate = rate;
    let live = mic::LiveCapture {
        buf: Arc::new(Mutex::new(Vec::new())),
        rate: Arc::new(AtomicU32::new(feed_rate)),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let status = crate::dictation::new_shared();

    // Буфер наполняем в реальном времени: поток отсчитывает молчание по часам, поэтому
    // «прокрутить» тест быстрее нельзя — приходится ждать честно.
    let feeder = {
        let (live, stop) = (live.clone(), stop.clone());
        std::thread::spawn(move || {
            let chunk = feed_rate as usize / 20; // 50мс
            let mut at = 0usize;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                if let Ok(mut b) = live.buf.lock() {
                    // Кончился файл (или его и не было) — дальше тишина.
                    let end = (at + chunk).min(source.len());
                    if at < end {
                        b.extend_from_slice(&source[at..end]);
                        at = end;
                    } else {
                        b.extend(std::iter::repeat(0.0).take(chunk));
                    }
                }
            }
        })
    };

    let t0 = Instant::now();
    crate::streaming::run(
        live,
        stop.clone(),
        "модель-не-нужна".into(),
        "ru".into(),
        false,
        status.clone(),
        egui::Context::default(),
    );

    let mut fired = None;
    while t0.elapsed() < Duration::from_secs(45) {
        std::thread::sleep(Duration::from_millis(100));
        if status.lock().map(|s| s.auto_stop).unwrap_or(false) {
            fired = Some(t0.elapsed());
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = feeder.join();

    match fired {
        Some(d) => println!(
            "остановился сам через {:.1} с (ожидалось ~{} с {})",
            d.as_secs_f32(),
            10,
            if wav.is_some() { "после конца речи" } else { "с начала" }
        ),
        None => println!("НЕ остановился за 45 с — автовыход не работает"),
    }
    let (state, busy) = status
        .lock()
        .map(|s| (s.state.clone(), s.busy))
        .unwrap_or_default();
    println!("состояние: {state:?}, busy={busy}");
}

/// Прогнать VAD по файлу и напечатать разметку — воспроизводимая проверка порога.
fn vad_file(path: &str) {
    let (mono, rate) = match audio::read_wav_mono(std::path::Path::new(path)) {
        Ok(v) => v,
        Err(e) => {
            println!("не прочитать {path}: {e}");
            return;
        }
    };
    let rate = (rate as usize).max(1);
    let s = |n: usize| n as f32 / rate as f32;
    println!(
        "{path}\n  {:.1}с @ {rate} Гц | {}",
        s(mono.len()),
        audio::stats(&mono, rate as u32)
    );

    let mut v = vad::Vad::new(rate);
    v.feed(&mono);
    let seg = v.segment(0);
    println!(
        "  шум={:.4}±{:.4} порог={:.4} | речи {:.1}с из {:.1}с, конец речи {:.1}с",
        v.noise(),
        v.dev(),
        v.on(),
        s(seg.speech),
        s(seg.analysed),
        s(seg.speech_end)
    );
    let spans = v.spans();
    println!("  участков речи: {}", spans.len());
    for (a, b) in spans.iter().take(20) {
        println!("    {:6.2} – {:6.2}с  ({:.2}с)", s(*a), s(*b), s(b - a));
    }
}

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
