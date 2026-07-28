//! Аудио-утилиты: приведение записанного WAV к формату, который требует Whisper (16 кГц моно).

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::mic::wav::WavWriter;

const TARGET_RATE: u32 = 16_000;

/// Записать моно-сэмплы (в диапазоне -1..1) как 16 кГц моно PCM16 WAV.
/// Используется потоковым режимом: буфер захвата → временный wav для whisper.
pub fn write_16k_wav_from_mono(mono: &[f32], in_rate: u32, output: &Path) -> Result<()> {
    let resampled = resample_linear(mono, in_rate.max(1), TARGET_RATE);
    let pcm: Vec<i16> = resampled
        .iter()
        .map(|&v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();
    let mut writer = WavWriter::create(output, 1, TARGET_RATE)?;
    writer.write_i16(&pcm)?;
    writer.finalize()?;
    Ok(())
}

/// Прочитать наш PCM16-WAV и записать его как 16 кГц моно PCM16 (для whisper.cpp).
pub fn wav_to_16k_mono(input: &Path, output: &Path) -> Result<()> {
    let bytes = fs::read(input)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(anyhow!("не WAV-файл"));
    }
    // Наш писатель кладёт канонический 44-байтовый заголовок PCM.
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]).max(1) as usize;
    let in_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]).max(1);
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
    if bits != 16 {
        return Err(anyhow!("ожидался PCM16, получено {bits} бит"));
    }

    // Данные начинаются после заголовка (ищем чанк "data" на всякий случай).
    let data_start = find_data_chunk(&bytes).unwrap_or(44);
    let samples: &[u8] = &bytes[data_start..];

    // Разбираем в i16 и сводим каналы в моно (среднее).
    let frame_bytes = channels * 2;
    let frame_count = samples.len() / frame_bytes;
    let mut mono: Vec<f32> = Vec::with_capacity(frame_count);
    for f in 0..frame_count {
        let base = f * frame_bytes;
        let mut acc = 0i32;
        for c in 0..channels {
            let o = base + c * 2;
            let s = i16::from_le_bytes([samples[o], samples[o + 1]]);
            acc += s as i32;
        }
        mono.push(acc as f32 / channels as f32);
    }

    // Ресемплинг до 16 кГц линейной интерполяцией.
    let out = resample_linear(&mono, in_rate, TARGET_RATE);

    let mut writer = WavWriter::create(output, 1, TARGET_RATE)?;
    let pcm: Vec<i16> = out
        .iter()
        .map(|&v| v.clamp(-32768.0, 32767.0) as i16)
        .collect();
    writer.write_i16(&pcm)?;
    writer.finalize()?;
    Ok(())
}

fn find_data_chunk(bytes: &[u8]) -> Option<usize> {
    // Идём по чанкам с 12-го байта.
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]])
            as usize;
        if id == b"data" {
            return Some(pos + 8);
        }
        pos += 8 + size;
    }
    None
}

fn resample_linear(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == out_rate {
        return input.to_vec();
    }
    let out_len = (input.len() as u64 * out_rate as u64 / in_rate as u64) as usize;
    let ratio = in_rate as f32 / out_rate as f32;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f32 * ratio;
        let idx = src.floor() as usize;
        let frac = src - idx as f32;
        let a = input[idx];
        let b = *input.get(idx + 1).unwrap_or(&a);
        out.push(a + (b - a) * frac);
    }
    out
}
