//! Резервный «хардкорный» путь — слой 3: прямой Core Audio / WASAPI.
//!
//! Здесь нет WinRT-обёртки: мы напрямую поднимаем COM, перечисляем конечные точки
//! захвата, активируем `IAudioClient`, открываем поток и сами читаем PCM-буферы.
//! Этот же движок используется для живого индикатора уровня сигнала.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_SHAREMODE_SHARED,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use super::wav::WavWriter;
use super::{DeviceInfo, MicEvent};

// Константы из mmreg.h / audioclient.h — задаём литералами, чтобы не тянуть лишние фичи.
const WAVE_FORMAT_PCM: u16 = 0x0001;
// Признак «тихого» буфера в флагах IAudioCaptureClient::GetBuffer.
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
// Автоконвертация формата силами WASAPI (float32 mix → запрошенный PCM).
const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x8000_0000;
const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;

/// RAII-инициализация COM в режиме MTA на текущем потоке.
pub struct ComGuard;

impl ComGuard {
    pub fn init_mta() -> Self {
        unsafe {
            // S_FALSE (уже инициализирован) нас устраивает.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        ComGuard
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// Перечислить активные устройства захвата с именами и пометкой устройства по умолчанию.
pub fn enumerate_capture_devices() -> Result<Vec<DeviceInfo>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

        let default_id = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .ok()
            .and_then(|d| device_id(&d));

        let collection = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;

        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = collection.Item(i)?;
            let Some(id) = device_id(&device) else { continue };
            let name = friendly_name(&device).unwrap_or_else(|| "Микрофон".to_string());
            let is_default = default_id.as_deref() == Some(id.as_str());
            out.push(DeviceInfo { id, name, is_default });
        }
        Ok(out)
    }
}

/// Диагностика: пробует инициализировать активное устройство захвата в разных режимах.
/// Каждый вариант — на свежем IAudioClient (Initialize можно звать лишь раз на клиент).
pub fn scan_capture_init() -> Result<Vec<String>> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let collection = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        let mut out = Vec::new();

        for i in 0..count {
            let device = collection.Item(i)?;
            let name = friendly_name(&device).unwrap_or_else(|| "?".to_string());
            out.push(format!("устройство [{i}]: {name}"));

            // Печатаем mix-формат один раз.
            if let Ok(c) = device.Activate::<IAudioClient>(CLSCTX_ALL, None) {
                if let Ok(p) = c.GetMixFormat() {
                    if !p.is_null() {
                        let (tag, rate, ch, bps) = (
                            (*p).wFormatTag,
                            (*p).nSamplesPerSec,
                            (*p).nChannels,
                            (*p).wBitsPerSample,
                        );
                        out.push(format!("   mix: tag={tag} {rate}Гц {ch}ch {bps}bit"));
                        CoTaskMemFree(Some(p as *const c_void));
                    }
                }
            }

            let pcm = |ch: u16, rate: u32| WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM,
                nChannels: ch,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * (ch * 2) as u32,
                nBlockAlign: ch * 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let ac = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

            // (метка, флаги, режим, формат-строитель, buffer)
            let try_variant = |label: &str, flags: u32, exclusive: bool, wf: &WAVEFORMATEX| {
                let Ok(client) = device.Activate::<IAudioClient>(CLSCTX_ALL, None) else {
                    return format!("   {label}: Activate ОШИБКА");
                };
                let mode = if exclusive {
                    windows::Win32::Media::Audio::AUDCLNT_SHAREMODE_EXCLUSIVE
                } else {
                    AUDCLNT_SHAREMODE_SHARED
                };
                let mut min_p: i64 = 0;
                let mut def_p: i64 = 0;
                let _ = client.GetDevicePeriod(Some(&mut def_p), Some(&mut min_p));
                let buf = if exclusive { min_p } else { 0 };
                let r = client.Initialize(mode, flags, buf, 0, wf as *const _, None);
                format!("   {label}: {r:?}")
            };

            // native mix через отдельный клиент
            {
                let native = (|| {
                    let client = device.Activate::<IAudioClient>(CLSCTX_ALL, None).ok()?;
                    let p = client.GetMixFormat().ok()?;
                    let r = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, p, None);
                    CoTaskMemFree(Some(p as *const c_void));
                    Some(format!("{r:?}"))
                })()
                .unwrap_or_else(|| "n/a".into());
                out.push(format!("   A native-mix shared: {native}"));
            }
            out.push(try_variant("B PCM 2ch/48k +autoconv", ac, false, &pcm(2, 48000)));
            out.push(try_variant("C PCM 1ch/48k +autoconv", ac, false, &pcm(1, 48000)));
            out.push(try_variant("D PCM 1ch/44.1k +autoconv", ac, false, &pcm(1, 44100)));
            out.push(try_variant("E PCM 2ch/48k excl", 0, true, &pcm(2, 48000)));
        }
        Ok(out)
    }
}

/// Открыть поток захвата и гнать пиковый уровень в `level`, пока не выставлен `stop`.
/// Если задан `record`, параллельно пишем PCM-16 в WAV-файл.
pub fn run_capture(
    device_id_opt: Option<String>,
    record: Option<PathBuf>,
    live: Option<super::LiveCapture>,
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    events: Sender<MicEvent>,
) -> Result<()> {
    // У каждого потока свой COM.
    let _com = ComGuard::init_mta();

    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

        let device: IMMDevice = match &device_id_opt {
            Some(id) => {
                let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
                enumerator.GetDevice(PCWSTR(wide.as_ptr()))?
            }
            None => enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?,
        };

        let mut client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| anyhow!("Activate(IAudioClient): {e}"))?;

        let pwfx = client
            .GetMixFormat()
            .map_err(|e| anyhow!("GetMixFormat: {e}"))?;
        if pwfx.is_null() {
            return Err(anyhow!("GetMixFormat вернул пустой формат"));
        }
        // WAVEFORMATEX — packed: читаем поля по значению в локальные переменные.
        let mix_rate: u32 = (*pwfx).nSamplesPerSec;
        let mix_channels: u16 = (*pwfx).nChannels.max(1);
        let mix_bits: u16 = (*pwfx).wBitsPerSample;
        let mix_tag: u16 = (*pwfx).wFormatTag;

        // --- Слой 3a: кооперативный SHARED-режим с родным mix-форматом (норма) ---
        let shared = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 0, 0, pwfx, None);
        CoTaskMemFree(Some(pwfx as *const c_void)); // WASAPI уже скопировал формат

        let sample_rate: u32;
        let n_channels: u16;
        let bits: u16;
        let is_float: bool;
        let mode: &str;

        if shared.is_ok() {
            sample_rate = mix_rate;
            n_channels = mix_channels;
            bits = mix_bits;
            is_float = mix_tag == 0x0003 || (mix_tag == 0xFFFE && mix_bits == 32);
            mode = "shared";
        } else {
            // --- Слой 3b: капризный драйвер не даёт SHARED → жёсткий EXCLUSIVE (PCM16) ---
            let code = shared
                .as_ref()
                .err()
                .map(|e| format!("0x{:08X}", e.code().0 as u32))
                .unwrap_or_default();
            let _ = events.send(MicEvent::Log(format!(
                "SHARED отклонён ({code}); переключаюсь на эксклюзивный режим."
            )));

            let block_align: u16 = mix_channels * 2;
            let wave = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM,
                nChannels: mix_channels,
                nSamplesPerSec: mix_rate,
                nAvgBytesPerSec: mix_rate * block_align as u32,
                nBlockAlign: block_align,
                wBitsPerSample: 16,
                cbSize: 0,
            };

            // После неудачного Initialize клиент непригоден — берём свежий.
            client = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| anyhow!("Activate(exclusive): {e}"))?;
            let mut def_p: i64 = 0;
            let mut min_p: i64 = 0;
            let _ = client.GetDevicePeriod(Some(&mut def_p), Some(&mut min_p));

            let mut buf = if min_p > 0 { min_p } else { def_p };
            let mut attempt =
                client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, 0, buf, 0, &wave, None);
            // Драйвер может требовать выровненный размер буфера.
            if let Err(e) = &attempt {
                if e.code().0 as u32 == 0x88890019 {
                    if let Ok(frames) = client.GetBufferSize() {
                        buf = (10_000_000i64 * frames as i64) / mix_rate as i64 + 1;
                        client = device
                            .Activate(CLSCTX_ALL, None)
                            .map_err(|e| anyhow!("Activate(exclusive/aligned): {e}"))?;
                        attempt =
                            client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, 0, buf, 0, &wave, None);
                    }
                }
            }
            attempt.map_err(|e| anyhow!("Initialize(exclusive): {e}"))?;

            sample_rate = mix_rate;
            n_channels = mix_channels;
            bits = 16;
            is_float = false;
            mode = "exclusive";
        }

        let capture_client: IAudioCaptureClient = client
            .GetService()
            .map_err(|e| anyhow!("GetService(IAudioCaptureClient): {e}"))?;

        let name = friendly_name(&device).unwrap_or_else(|| "Микрофон".to_string());
        let format = format!("{sample_rate} Гц · {n_channels} кан · {bits} бит · {mode}");
        let _ = events.send(MicEvent::CaptureStarted {
            device: name,
            format,
        });

        let channels = n_channels.max(1) as usize;
        if let Some(l) = &live {
            l.rate.store(sample_rate, Ordering::Relaxed);
        }

        // Открываем WAV, если запрошена запись.
        let mut writer: Option<WavWriter> = match &record {
            Some(path) => match WavWriter::create(path, n_channels, sample_rate) {
                Ok(w) => {
                    let _ = events.send(MicEvent::Log(format!(
                        "Запись в файл: {}",
                        path.display()
                    )));
                    Some(w)
                }
                Err(e) => {
                    let _ = events.send(MicEvent::Error(format!("Не удалось создать WAV: {e}")));
                    None
                }
            },
            None => None,
        };
        // Переиспользуемый буфер конвертации в PCM-16.
        let mut pcm: Vec<i16> = Vec::new();

        client.Start()?;

        let mut shown = 0f32;
        while !stop.load(Ordering::Relaxed) {
            let mut packet = capture_client.GetNextPacketSize()?;
            if packet == 0 {
                thread::sleep(Duration::from_millis(10));
                continue;
            }

            let mut frame_peak = 0f32;
            while packet != 0 {
                let mut pdata: *mut u8 = ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                capture_client.GetBuffer(&mut pdata, &mut frames, &mut flags, None, None)?;

                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT != 0;
                let sample_count = frames as usize * channels;

                if frames > 0 && !pdata.is_null() {
                    if is_float {
                        let s = std::slice::from_raw_parts(pdata as *const f32, sample_count);
                        if let Some(w) = writer.as_mut() {
                            pcm.clear();
                            pcm.extend(s.iter().map(|&v| {
                                (v.clamp(-1.0, 1.0) * 32767.0) as i16
                            }));
                            let _ = w.write_i16(&pcm);
                        }
                        if let Some(l) = &live {
                            if let Ok(mut b) = l.buf.lock() {
                                for f in 0..frames as usize {
                                    let mut acc = 0f32;
                                    for c in 0..channels {
                                        acc += s[f * channels + c];
                                    }
                                    b.push(acc / channels as f32);
                                }
                            }
                        }
                        if !silent {
                            for &v in s {
                                let a = v.abs();
                                if a > frame_peak {
                                    frame_peak = a;
                                }
                            }
                        }
                    } else if bits == 16 {
                        let s = std::slice::from_raw_parts(pdata as *const i16, sample_count);
                        if let Some(w) = writer.as_mut() {
                            let _ = w.write_i16(s);
                        }
                        if let Some(l) = &live {
                            if let Ok(mut b) = l.buf.lock() {
                                for f in 0..frames as usize {
                                    let mut acc = 0f32;
                                    for c in 0..channels {
                                        acc += s[f * channels + c] as f32 / 32768.0;
                                    }
                                    b.push(acc / channels as f32);
                                }
                            }
                        }
                        if !silent {
                            for &v in s {
                                let a = (v as f32 / 32768.0).abs();
                                if a > frame_peak {
                                    frame_peak = a;
                                }
                            }
                        }
                    } else if let Some(w) = writer.as_mut() {
                        // Неизвестный формат — пишем тишину, чтобы сохранить длительность.
                        pcm.clear();
                        pcm.resize(sample_count, 0);
                        let _ = w.write_i16(&pcm);
                    }
                }

                capture_client.ReleaseBuffer(frames)?;
                packet = capture_client.GetNextPacketSize()?;
            }

            // Быстрый подъём индикатора, плавный спад — приятнее визуально.
            shown = if frame_peak > shown {
                frame_peak
            } else {
                shown * 0.82 + frame_peak * 0.18
            };
            level.store(shown.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);

            thread::sleep(Duration::from_millis(16));
        }

        client.Stop()?;
        // ВНИМАНИЕ: pwfx уже освобождён после Initialize (см. выше). Второй CoTaskMemFree
        // здесь давал двойное освобождение → повреждение кучи (0xC0000374). Убрано.

        // Финализируем файл и сообщаем результат.
        if let (Some(w), Some(path)) = (writer.take(), record.as_ref()) {
            let seconds = w.seconds();
            match w.finalize() {
                Ok(bytes) => {
                    let _ = events.send(MicEvent::Log(format!(
                        "Сохранено: {} — {:.1} с, {} КБ",
                        path.display(),
                        seconds,
                        bytes / 1024
                    )));
                }
                Err(e) => {
                    let _ = events.send(MicEvent::Error(format!("Ошибка записи WAV: {e}")));
                }
            }
        }
    }

    Ok(())
}

/// Получить строковый ID устройства (и освободить выданную COM-память).
unsafe fn device_id(device: &IMMDevice) -> Option<String> {
    let p = device.GetId().ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const c_void));
    s
}

/// Прочитать человекочитаемое имя устройства из его хранилища свойств.
/// PROPVARIANT в windows-crate сам корректно освобождается (Drop) и умеет Display.
unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;
    let prop = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
    let name = prop.to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}
