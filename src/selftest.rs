//! Самотесты вставки и индикатора: прогоняют боевой код на живой системе.
//!
//! Все они печатают в окно-приёмник, поэтому у каждого есть предохранитель по заголовку
//! окна: если фокус ушёл, тест обрывается, а не набирает текст в чужой документ.

use crate::{hud, inject, overlay, streaming, transcribe};

/// Разобрать аргументы самотестов. `true` — режим отработал.
pub fn dispatch(args: &[String]) -> bool {
        // Стоимость приведения звука к 16 кГц: `--resample-bench [секунд]`.
        if let Some(i) = args.iter().position(|a| a == "--resample-bench") {
            let secs: usize = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(12);
            let rate = 48_000usize;
            // Шум по всей полосе — худший случай для фильтра.
            let mut x = 12345u32;
            let noise: Vec<f32> = (0..rate * secs)
                .map(|_| {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    (x >> 8) as f32 / 8_388_608.0 - 1.0
                })
                .collect();
            let tmp = crate::models::app_dir().join("temp");
            let _ = std::fs::create_dir_all(&tmp);
            let wav = tmp.join("resample_bench.wav");
            let t0 = std::time::Instant::now();
            let _ = crate::audio::write_16k_wav_from_mono(&noise, rate as u32, &wav);
            println!(
                "{secs}с звука @48кГц → 16кГц за {:.0}мс",
                t0.elapsed().as_secs_f32() * 1000.0
            );
            let _ = std::fs::remove_file(&wav);
            return true;
        }

        // Резкость отклика на голос: `--pulse-sim`. Гоним синтетический всплеск громкости
        // через ту же огибающую, что и живой индикатор, и печатаем высоту столбиками.
        if args.iter().any(|a| a == "--pulse-sim") {
            // Кадры по 33мс: тишина, слог, короткая пауза, слог, затухание.
            let mut levels = vec![0.015f32; 6];
            levels.extend(std::iter::repeat(0.22).take(4));
            levels.extend(std::iter::repeat(0.015).take(6));
            levels.extend(std::iter::repeat(0.12).take(3));
            levels.extend(std::iter::repeat(0.015).take(10));
            for (n, v) in overlay::simulate_pulse(&levels).iter().enumerate() {
                let bar = "#".repeat((v * 40.0).round() as usize);
                println!("{:>5}мс уровень={:.3} → {v:.2} {bar}", n * 33, levels[n]);
            }
            return true;
        }

        // Файл значка для встраивания в .exe: `--icon-file <файл.ico>`.
        // Рисуется тем же генератором, что значок трея, — держать их врозь нельзя.
        if let Some(i) = args.iter().position(|a| a == "--icon-file") {
            let out = args.get(i + 1).cloned().unwrap_or_else(|| "tvoice.ico".into());
            match write_ico(&out, &[16, 24, 32, 48, 64, 128]) {
                Ok(n) => println!("значок записан: {out} ({n} байт)"),
                Err(e) => println!("не записать {out}: {e}"),
            }
            return true;
        }

        // Картинка значка в файл: `--icon-preview <файл.bmp>` — посмотреть, как он
        // выглядит в трее и на панели задач, не запуская приложение.
        if let Some(i) = args.iter().position(|a| a == "--icon-preview") {
            let out = args.get(i + 1).cloned().unwrap_or_else(|| "icon.bmp".into());
            let size = 64usize;
            let px = hud::icon_pixels(size as i32);
            // Показываем на двух подложках, чтобы оценить края и прозрачность.
            let (tw, th) = (size * 2, size);
            let mut rgb = vec![0u32; tw * th];
            for (col, bg) in [(0usize, 0x1E2020u32), (1, 0xF2F2F0)] {
                for y in 0..size {
                    for x in 0..size {
                        let s = px[y * size + x];
                        let a = ((s >> 24) & 0xFF) as u32;
                        let mix = |sc: u32, bc: u32| (sc + bc * (255 - a) / 255).min(255);
                        rgb[y * tw + col * size + x] = (mix((s >> 16) & 0xFF, (bg >> 16) & 0xFF) << 16)
                            | (mix((s >> 8) & 0xFF, (bg >> 8) & 0xFF) << 8)
                            | mix(s & 0xFF, bg & 0xFF);
                    }
                }
            }
            write_bmp(&out, tw, th, &rgb);
            return true;
        }

        // Показать индикатор без диктовки: `--overlay-demo [сек]`. Нужно, чтобы убедиться,
        // что окно вообще появляется на экране, и снять его скриншотом.
        if let Some(i) = args.iter().position(|a| a == "--overlay-demo") {
            let secs: u64 = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(6);
            // Демонстрация должна показывать то же, что и настоящая диктовка.
            let cfg = crate::config::load();
            overlay::set_anchor(overlay::Anchor::from_id(&cfg.hud_anchor));
            overlay::set_scale(cfg.hud_scale);
            let ov = overlay::Overlay::spawn();
            ov.set_visible(true);
            let t0 = std::time::Instant::now();
            while t0.elapsed().as_secs() < secs {
                // Уровень гоняем по синусоиде — видно, что точки живые.
                let s = t0.elapsed().as_secs_f32();
                ov.set_level(0.5 + 0.5 * (s * 2.0).sin());
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            ov.set_visible(false);
            std::thread::sleep(std::time::Duration::from_millis(200));
            println!("демонстрация индикатора завершена");
            return true;
        }

        // Проверка привязки индикатора: `--caret-test [сек]` печатает, находится ли
        // курсор ввода активного окна (и какого именно).
        if let Some(i) = args.iter().position(|a| a == "--caret-test") {
            let secs: u64 = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
            crate::caret::watch();
            crate::caret::set_active(true);
            for _ in 0..secs * 2 {
                let win = inject::foreground();
                match crate::caret::position() {
                    Some(((x, y), src)) => println!("{win} → курсор ввода ({x}, {y}), источник {src}"),
                    None => println!("{win} → курсора ввода нет, индикатор пойдёт к мыши"),
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            return true;
        }

        // Картинка индикатора в файл: `--overlay-preview <файл.bmp>`. Рисуем те же кадры,
        // что видит пользователь, поверх светлой и тёмной подложки — видно и края, и свечение.
        if let Some(i) = args.iter().position(|a| a == "--overlay-preview") {
            let out = args.get(i + 1).cloned().unwrap_or_else(|| "overlay.bmp".into());
            overlay_preview(&out);
            return true;
        }

        // Симуляция правок: `--rewrite-sim <черновик>…` печатает, сколько символов
        // стёрлось бы и вставилось на каждом уточнении. Без окна и без вставки.
        if let Some(i) = args.iter().position(|a| a == "--rewrite-sim") {
            let drafts: Vec<String> = args[(i + 1).min(args.len())..].to_vec();
            for (n, (back, add, shown)) in streaming::simulate(&drafts).iter().enumerate() {
                println!("{n}: -{back} +{add} → {shown:?}");
            }
            return true;
        }

        // Склейка сегментов whisper: `--whisper-norm <файл_с_сырым_ответом>` печатает
        // строки как они пришли и результат склейки. Нужно, чтобы видеть текстом, что
        // перенос строки посреди слова не превращается в пробел («готов» + «ый»).
        if let Some(i) = args.iter().position(|a| a == "--whisper-norm") {
            let path = args.get(i + 1).cloned().unwrap_or_default();
            match std::fs::read_to_string(&path) {
                Ok(raw) => {
                    println!("сырой ответ, {} строк:", raw.lines().count());
                    for l in raw.lines() {
                        println!("  |{l}|");
                    }
                    println!("после склейки:\n  {:?}", transcribe::normalize(&raw));
                }
                Err(e) => println!("не прочитать {path:?}: {e}"),
            }
            return true;
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
                return true;
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
            return true;
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
                return true;
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
            return true;
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
                    return true;
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
            return true;
        }
    false
}

/// Отрисовать кадры индикатора в BMP: три уровня громкости на двух подложках.
/// Нужно, чтобы оценивать вид индикатора глазами, а не запуская диктовку.
fn overlay_preview(path: &str) {
    let (w, h) = hud::size_for(1.0);
    let (cols, rows) = (4usize, 2usize);
    let (tw, th) = (w as usize * cols, h as usize * rows);
    let mut out = vec![0u32; tw * th]; // 0x00RRGGBB
    let mut frame = vec![0u32; (w * h) as usize];

    // Последние два столбца — одинаковая громкость в разные моменты: по ним видно,
    // что точки расходятся по высоте, то есть рандомайзер работает.
    for (row, bg) in [(0usize, 0x1E2020u32), (1usize, 0xF2F2F0u32)] {
        for (col, level, t) in [
            (0usize, 0.0f32, 0.7f32),
            (1, 0.45, 1.9),
            (2, 1.0, 3.1),
            (3, 1.0, 5.4),
        ] {
            hud::draw_frame(&mut frame, w, h, 1.0, t, level);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let s = frame[y * w as usize + x];
                    // Буфер предумножен на альфу: dst = src + bg*(1-a).
                    let a = ((s >> 24) & 0xFF) as u32;
                    let mix = |sc: u32, bc: u32| (sc + bc * (255 - a) / 255).min(255);
                    let px = (mix((s >> 16) & 0xFF, (bg >> 16) & 0xFF) << 16)
                        | (mix((s >> 8) & 0xFF, (bg >> 8) & 0xFF) << 8)
                        | mix(s & 0xFF, bg & 0xFF);
                    out[(row * h as usize + y) * tw + col * w as usize + x] = px;
                }
            }
        }
    }

    write_bmp(path, tw, th, &out);
}

/// Записать картинку 24-битным BMP: строки снизу вверх, выравнивание строки на 4 байта.
fn write_bmp(path: &str, tw: usize, th: usize, out: &[u32]) {
    let stride = (tw * 3 + 3) & !3;
    let data_size = stride * th;
    let mut bytes = Vec::with_capacity(54 + data_size);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&((54 + data_size) as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&(tw as i32).to_le_bytes());
    bytes.extend_from_slice(&(th as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 24]);
    for y in (0..th).rev() {
        let mut line = Vec::with_capacity(stride);
        for x in 0..tw {
            let p = out[y * tw + x];
            line.push((p & 0xFF) as u8);
            line.push(((p >> 8) & 0xFF) as u8);
            line.push(((p >> 16) & 0xFF) as u8);
        }
        line.resize(stride, 0);
        bytes.extend_from_slice(&line);
    }
    match std::fs::write(path, &bytes) {
        Ok(()) => println!("индикатор нарисован в {path} ({tw}×{th})"),
        Err(e) => println!("не удалось записать {path}: {e}"),
    }
}

/// Записать .ico с несколькими размерами. Каждый кадр — 32-битный DIB с альфой:
/// формат древний, но понимают его все версии Windows, а PNG внутри .ico — не все.
fn write_ico(path: &str, sizes: &[i32]) -> std::io::Result<usize> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &s in sizes {
        let px = hud::icon_pixels(s);
        let (w, h) = (s as u32, s as u32);
        let mut img = Vec::new();
        // BITMAPINFOHEADER: высота удвоена — за цветом идёт маска прозрачности.
        img.extend_from_slice(&40u32.to_le_bytes());
        img.extend_from_slice(&w.to_le_bytes());
        img.extend_from_slice(&(h * 2).to_le_bytes());
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        img.extend_from_slice(&[0u8; 24]);
        // Цвет: строки снизу вверх, BGRA.
        for y in (0..s).rev() {
            for x in 0..s {
                img.extend_from_slice(&px[(y * s + x) as usize].to_le_bytes());
            }
        }
        // Маска: не нужна при 32 битах, но структура её требует — кладём нули.
        let stride = ((s as usize + 31) / 32) * 4;
        img.extend(std::iter::repeat(0u8).take(stride * s as usize));
        images.push(img);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // зарезервировано
    out.extend_from_slice(&1u16.to_le_bytes()); // тип: значок
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&s, img) in sizes.iter().zip(&images) {
        // 256 в каталоге записывается нулём — байт больше не вмещает.
        out.push(if s >= 256 { 0 } else { s as u8 });
        out.push(if s >= 256 { 0 } else { s as u8 });
        out.push(0); // палитры нет
        out.push(0);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += img.len();
    }
    for img in &images {
        out.extend_from_slice(img);
    }
    std::fs::write(path, &out)?;
    Ok(out.len())
}
