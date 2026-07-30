//! Локальная офлайн-транскрибация через внешний whisper.cpp (whisper-cli.exe).

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};

use crate::models;

/// Флаг Windows: не создавать окно консоли при запуске процесса.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Распознать 16 кГц моно WAV указанной моделью. `lang` — код языка ("ru", "en", …) или "auto".
pub fn transcribe(wav16k: &Path, model_file: &str, lang: &str) -> Result<String> {
    let exe = models::whisper_exe()
        .ok_or_else(|| anyhow!("whisper.cpp не установлен — скачайте его во вкладке «Модели»"))?;
    let model = models::model_path(model_file);
    if !model.is_file() {
        return Err(anyhow!("модель не найдена: {model_file}"));
    }

    let out_base = wav16k.with_extension(""); // whisper допишет .txt
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .to_string();

    let mut cmd = Command::new(&exe);
    cmd.arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(wav16k)
        .arg("-l")
        .arg(lang)
        .arg("-t")
        .arg(&threads)
        // Ускорение для диктовки: жадное декодирование (beam=1, best-of=1) без
        // температурного fallback — заметно быстрее при небольшой потере точности.
        .arg("-bs")
        .arg("1")
        .arg("-bo")
        .arg("1")
        .arg("-nf")
        .arg("-nt") // без таймкодов в выводе
        .arg("-otxt")
        .arg("-of")
        .arg(&out_base);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd
        .output()
        .map_err(|e| anyhow!("не удалось запустить whisper-cli: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("whisper вернул ошибку: {}", err.trim()));
    }

    let txt_path = out_base.with_extension("txt");
    let text = fs::read_to_string(&txt_path).unwrap_or_default();
    let _ = fs::remove_file(&txt_path);

    Ok(normalize(&text))
}

/// Склеить вывод whisper в одну строку.
///
/// whisper отдаёт текст сегментами, каждый на своей строке, и ведущий пробел — часть
/// текста сегмента, а не разделитель строк. Когда граница сегмента попадает ВНУТРЬ слова,
/// продолжение идёт без пробела:
///
/// ```text
/// Первое, что нужно понять - английский - это не набор готов
/// ый
/// ```
///
/// Поэтому перевод строки нельзя заменять пробелом — из «готовых» получалось «готов ых»
/// (а из «искусственный» — «искус ственный»). Строки склеиваем как есть; пробел
/// добавляем только там, где разрыв внутри слова невозможен — после знака препинания.
pub fn normalize(raw: &str) -> String {
    let mut glued = String::with_capacity(raw.len());
    for line in raw.lines() {
        if glued.ends_with(SENTENCE_END) {
            glued.push(' ');
        }
        glued.push_str(line);
    }
    squeeze(&glued)
}

/// Знаки, после которых слово точно закончилось.
const SENTENCE_END: [char; 10] = ['.', '!', '?', ',', ';', ':', '…', '»', '"', '\''];

/// Свернуть подряд идущие пробельные символы в один и обрезать края.
fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}
