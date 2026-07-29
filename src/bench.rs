//! Стенд качества распознавания: сравнивает варианты обработки звука и настройки
//! декодирования на записи с заранее известным текстом.
//!
//! Нужен, чтобы решения принимались по числам. «Стало лучше» на слух — ненадёжно:
//! разница в окончаниях составляет единицы процентов ошибок, а слух подстраивается.

use std::path::Path;

use crate::{audio, models, server};

/// Один прогон: как называется вариант, что вернул сервер, сколько занял и сколько ошибок.
struct Run {
    name: String,
    text: String,
    secs: f32,
    errors: usize,
    words: usize,
}

/// `--stt-bench <файл.wav> <эталонный текст>` — измерить качество на своей записи.
pub fn run(wav: &Path, reference: &str, model_id: Option<&str>) {
    let Some(model) = pick_model(model_id) else {
        println!("нет скачанной модели — нечем измерять");
        return;
    };
    println!("модель: {model}\nэталон: {reference}\n");

    let tmp = models::app_dir().join("temp");
    let _ = std::fs::create_dir_all(&tmp);

    // Исходник читаем как есть: у синтезатора и микрофона частота выше 16 кГц,
    // значит оба варианта приведения имеют смысл.
    let (mono, rate) = match audio::read_wav_mono(wav) {
        Ok(v) => v,
        Err(e) => {
            println!("не прочитать {}: {e}", wav.display());
            return;
        }
    };
    println!("запись: {:.1}с @ {rate} Гц\n", mono.len() as f32 / rate as f32);

    // Чистая синтезированная речь ничего не показывает: выше 8 кГц у неё пусто,
    // зеркалить нечего. Добавляем широкополосный шум на уровне живого микрофона —
    // тогда и появляется то, что фильтр должен убирать.
    let noisy = add_noise(&mono, 20.0);

    let filtered = tmp.join("bench_filtered.wav");
    let plain = tmp.join("bench_plain.wav");
    let n_filtered = tmp.join("bench_noisy_filtered.wav");
    let n_plain = tmp.join("bench_noisy_plain.wav");
    let _ = audio::write_16k_wav_variant(&mono, rate, &filtered, true);
    let _ = audio::write_16k_wav_variant(&mono, rate, &plain, false);
    let _ = audio::write_16k_wav_variant(&noisy, rate, &n_filtered, true);
    let _ = audio::write_16k_wav_variant(&noisy, rate, &n_plain, false);

    let base = server::decoding_args();
    let mut runs = Vec::new();

    // 1. Приведение звука: с фильтром против прежнего прореживания без фильтра —
    //    на чистой записи и на записи с шумом.
    if rate > 16_000 {
        runs.push(measure("чисто: без фильтра", &model, &base, &plain, reference));
        runs.push(measure("чисто: с фильтром", &model, &base, &filtered, reference));
        runs.push(measure("шум 20дБ: без фильтра", &model, &base, &n_plain, reference));
        runs.push(measure("шум 20дБ: с фильтром", &model, &base, &n_filtered, reference));
    }

    // 2. Настройки декодирования — на звуке с фильтром.
    let prompt = "Здравствуйте! Это диктовка на русском языке: запятые, точки и \
                  окончания слов на месте.";
    let variants: Vec<(&str, Vec<String>)> = vec![
        ("декодирование: жадное (как было)", args(&["-bs", "1", "-nf"])),
        ("декодирование: перебор лучей 5", args(&["-bs", "5", "-nf"])),
        ("перебор 5 + откат по температуре", args(&["-bs", "5"])),
        (
            "перебор 5 + начальный контекст",
            [args(&["-bs", "5", "-nf", "-sns"]), vec!["--prompt".into(), prompt.into()]].concat(),
        ),
    ];
    // Настройки декодирования проверяем на зашумлённой записи: на чистой разницы
    // не увидеть — там любой вариант справляется.
    for (name, a) in &variants {
        runs.push(measure(name, &model, a, &n_filtered, reference));
    }

    println!("{:<38} {:>7} {:>8}  текст", "вариант", "ошибок", "время");
    for r in &runs {
        println!(
            "{:<38} {:>3}/{:<3} {:>7.2}с  {}",
            r.name, r.errors, r.words, r.secs, r.text
        );
    }
    for f in [&filtered, &plain, &n_filtered, &n_plain] {
        let _ = std::fs::remove_file(f);
    }
}

/// Подмешать белый шум с заданным отношением сигнал/шум (дБ).
///
/// Белый — значит по всей полосе до половины частоты дискретизации: именно эта часть
/// и зеркалится в речевой диапазон при прореживании без фильтра.
fn add_noise(input: &[f32], snr_db: f32) -> Vec<f32> {
    let rms = (input.iter().map(|v| v * v).sum::<f32>() / input.len().max(1) as f32).sqrt();
    let amp = rms / 10f32.powf(snr_db / 20.0);
    let mut x = 0x2545_F491u32;
    input
        .iter()
        .map(|&v| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let n = (x >> 8) as f32 / 8_388_608.0 - 1.0;
            (v + n * amp * 1.732).clamp(-1.0, 1.0)
        })
        .collect()
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn measure(name: &str, model: &str, extra: &[String], wav: &Path, reference: &str) -> Run {
    match server::measure(model, "ru", extra, wav) {
        Ok((text, secs)) => {
            let (errors, words) = word_errors(reference, &text);
            Run {
                name: name.to_string(),
                text,
                secs,
                errors,
                words,
            }
        }
        Err(e) => Run {
            name: name.to_string(),
            text: format!("ОШИБКА: {e}"),
            secs: 0.0,
            errors: 0,
            words: 0,
        },
    }
}

/// Модель берём ту же, что выбрана в приложении: мерить надо на том, чем пользуются,
/// а не на первой попавшейся из каталога.
fn pick_model(explicit: Option<&str>) -> Option<String> {
    if let Some(id) = explicit {
        if let Some(m) = models::by_id(id).filter(|m| models::is_downloaded(m.file)) {
            return Some(m.file.to_string());
        }
        println!("модель {id} не найдена или не скачана — беру выбранную в приложении");
    }
    let cfg = crate::config::load();
    cfg.model
        .as_deref()
        .and_then(models::by_id)
        .filter(|m| models::is_downloaded(m.file))
        .or_else(|| models::CATALOG.iter().find(|m| models::is_downloaded(m.file)))
        .map(|m| m.file.to_string())
}

/// Число ошибок по словам (расстояние Левенштейна) и длина эталона.
///
/// Считаем именно по словам и с приведением: нас интересуют пропущенные и
/// перепутанные слова, а не регистр и знаки препинания.
fn word_errors(reference: &str, got: &str) -> (usize, usize) {
    let a = normalize(reference);
    let b = normalize(got);
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return (m, 0);
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    (prev[m], n)
}

fn normalize(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}
