//! Декодирование сжатого звука в PCM через Media Foundation.
//!
//! Нужно ровно для одного: развернуть сигнал старта во времени, чтобы на выходе из захвата
//! играть его наоборот. MCI умеет играть mp3, но не умеет играть назад, а для разворота
//! нужны сами сэмплы. Тянуть в проект отдельный декодер ради одного короткого звука —
//! лишняя зависимость и лишний C-код в сборке; Media Foundation уже есть в системе и отдаёт
//! любой поддерживаемый ею формат сразу как PCM16, подставляя декодер сама.

use std::path::Path;

use anyhow::{anyhow, Result};
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::{
    IMFSourceReader, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio,
    MFShutdown, MFStartup, MFAudioFormat_PCM, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_VERSION,
    MFSTARTUP_NOSOCKET,
};

/// Разобранный звук: чередующиеся сэмплы, частота, число каналов.
pub struct Pcm {
    pub samples: Vec<i16>,
    pub rate: u32,
    pub channels: u16,
}

/// Декодировать файл в PCM16.
pub fn decode(path: &Path) -> Result<Pcm> {
    unsafe {
        // Инициализацию не откатываем при ошибках чтения: MFShutdown парный к MFStartup,
        // и лишний вызов сломал бы состояние на весь процесс. Гасим один раз в конце.
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)?;
        let result = read_all(path);
        let _ = MFShutdown();
        result
    }
}

unsafe fn read_all(path: &Path) -> Result<Pcm> {
    let url = HSTRING::from(path.as_os_str());
    let reader: IMFSourceReader = MFCreateSourceReaderFromURL(&url, None)?;
    let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

    // Просим PCM16 — какой декодер для этого нужен, Media Foundation решает сама.
    // Частоту и каналы не навязываем: пересчёт нам не нужен, звук играется как есть.
    let want = MFCreateMediaType()?;
    want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    want.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
    want.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    reader.SetCurrentMediaType(stream, None, &want)?;

    let actual = reader.GetCurrentMediaType(stream)?;
    let rate = actual.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;
    let channels = actual.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)? as u16;
    if rate == 0 || channels == 0 {
        return Err(anyhow!("декодер вернул формат 0 Гц / 0 каналов"));
    }

    let mut samples: Vec<i16> = Vec::new();
    loop {
        let mut flags = 0u32;
        let mut sample = None;
        reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            break;
        }
        // Кадр без данных — не конец потока (бывает при смене формата), просто читаем дальше.
        let Some(sample) = sample else {
            continue;
        };
        let buffer = sample.ConvertToContiguousBuffer()?;
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut len = 0u32;
        buffer.Lock(&mut data, None, Some(&mut len))?;
        if !data.is_null() {
            let words = len as usize / 2;
            samples.extend_from_slice(std::slice::from_raw_parts(data as *const i16, words));
        }
        buffer.Unlock()?;
    }
    if samples.is_empty() {
        return Err(anyhow!("в файле нет звука"));
    }
    Ok(Pcm {
        samples,
        rate,
        channels,
    })
}
