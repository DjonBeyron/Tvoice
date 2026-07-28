//! Минимальный писатель WAV (PCM 16-бит) без внешних зависимостей.
//!
//! Захват из WASAPI обычно приходит во float32; мы конвертируем в 16-бит PCM —
//! такой файл гарантированно открывается любым проигрывателем.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

pub struct WavWriter {
    inner: BufWriter<File>,
    data_bytes: u32,
    channels: u16,
    sample_rate: u32,
}

impl WavWriter {
    /// Создать файл и записать временный заголовок (размеры допишем при финализации).
    pub fn create(path: &Path, channels: u16, sample_rate: u32) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut w = Self {
            inner: BufWriter::new(file),
            data_bytes: 0,
            channels: channels.max(1),
            sample_rate,
        };
        w.write_header(0)?;
        Ok(w)
    }

    fn write_header(&mut self, data_bytes: u32) -> io::Result<()> {
        let channels = self.channels;
        let sample_rate = self.sample_rate;
        let bits_per_sample: u16 = 16;
        let block_align: u16 = channels * (bits_per_sample / 8);
        let byte_rate: u32 = sample_rate * block_align as u32;
        let riff_size: u32 = 36 + data_bytes;

        let w = &mut self.inner;
        w.write_all(b"RIFF")?;
        w.write_all(&riff_size.to_le_bytes())?;
        w.write_all(b"WAVE")?;

        w.write_all(b"fmt ")?;
        w.write_all(&16u32.to_le_bytes())?; // размер fmt-чанка
        w.write_all(&1u16.to_le_bytes())?; // PCM
        w.write_all(&channels.to_le_bytes())?;
        w.write_all(&sample_rate.to_le_bytes())?;
        w.write_all(&byte_rate.to_le_bytes())?;
        w.write_all(&block_align.to_le_bytes())?;
        w.write_all(&bits_per_sample.to_le_bytes())?;

        w.write_all(b"data")?;
        w.write_all(&data_bytes.to_le_bytes())?;
        Ok(())
    }

    /// Дописать блок 16-битных сэмплов (interleaved по каналам).
    pub fn write_i16(&mut self, samples: &[i16]) -> io::Result<()> {
        for &s in samples {
            self.inner.write_all(&s.to_le_bytes())?;
        }
        self.data_bytes = self
            .data_bytes
            .saturating_add((samples.len() * 2) as u32);
        Ok(())
    }

    /// Закрыть файл, дописав корректные размеры в заголовок.
    pub fn finalize(mut self) -> io::Result<u32> {
        self.inner.flush()?;
        let data_bytes = self.data_bytes;
        // RIFF size @ смещение 4, data size @ смещение 40.
        self.inner.seek(SeekFrom::Start(4))?;
        self.inner.write_all(&(36 + data_bytes).to_le_bytes())?;
        self.inner.seek(SeekFrom::Start(40))?;
        self.inner.write_all(&data_bytes.to_le_bytes())?;
        self.inner.flush()?;
        Ok(data_bytes)
    }

    /// Длительность записи в секундах по накопленным данным.
    pub fn seconds(&self) -> f32 {
        let bytes_per_sec = self.sample_rate * self.channels as u32 * 2;
        if bytes_per_sec == 0 {
            0.0
        } else {
            self.data_bytes as f32 / bytes_per_sec as f32
        }
    }
}

/// Имя файла с локальной меткой времени: `tvoice_YYYYMMDD_HHMMSS.wav`.
pub fn timestamp_filename() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "tvoice_{:04}{:02}{:02}_{:02}{:02}{:02}.wav",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}
