//! Короткие сигналы входа в захват и выхода из него.
//!
//! Играем через MCI (`winmm`): mp3 он открывает сам по расширению. Файл — `rec.mp3` рядом
//! с .exe; нет файла — нет сигналов, и это же самый простой способ их отключить.
//!
//! Сигнал выхода — тот же звук, развёрнутый во времени. MCI играть назад не умеет, поэтому
//! один раз декодируем mp3 в PCM (`mp3`, через Media Foundation), переворачиваем сэмплы и
//! кладём рядом WAV — его MCI играет обычным образом. Готовый файл кэшируем и пересобираем
//! только когда `rec.mp3` изменился, иначе каждый запуск платил бы за декодирование.
//!
//! Две вещи, от которых зависит «мгновенно»:
//!
//! * файлы открываем ОДИН раз заранее ([`prewarm`]), а не на нажатии: открытие — самая
//!   долгая часть, и на первое нажатие она бы и легла;
//! * играем в своём потоке. Замер: `play` возвращает управление через ~13мс, а поток
//!   хоткея столько ждать не должен — иначе сигнал задерживал бы саму диктовку, ради
//!   отзывчивости которой он и нужен.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Media::Multimedia::mciSendStringW;

/// Имя файла сигнала (рядом с .exe).
const FILE: &str = "rec.mp3";
/// Развёрнутая копия — во временной папке: это производный файл, а не настройка.
const REVERSED: &str = "rec_reversed.wav";

/// Идёт ли захват. От этого зависит, какой сигнал играть на нажатии хоткея: хоткей в
/// потоковом режиме — переключатель, и одно и то же нажатие может и входить, и выходить.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Какой сигнал играть.
#[derive(Clone, Copy)]
enum Cue {
    Start,
    Stop,
}

/// Сообщить, идёт ли сейчас захват (вызывает приложение каждый кадр).
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

/// Подготовить сигналы заранее — вызывать на старте приложения.
pub fn prewarm() {
    let _ = player();
}

/// Сигнал входа в захват — вызывается из потока хоткея на каждое нажатие.
///
/// Если захват уже идёт, нажатие означает выход, и здесь мы молчим: выход озвучивает
/// [`play_exit`]. Иначе повторное нажатие играло бы сигнал входа.
pub fn play_enter() {
    if !ACTIVE.load(Ordering::Relaxed) {
        play(Cue::Start);
    }
}

/// Сигнал выхода из захвата — тот же звук, развёрнутый во времени.
///
/// Вызывается из одного места (`stop_dictation`), через которое проходят все способы
/// остановки: повторное нажатие, отпускание в батч-режиме, меню трея и молчание. Отсюда
/// же и задержка на кадр — для сигнала «закончили» она незаметна, а гарантия «ровно один
/// раз на каждый выход» важнее.
pub fn play_exit() {
    play(Cue::Stop);
}

fn play(cue: Cue) {
    if let Some(tx) = player() {
        let _ = tx.send(cue);
    }
}

/// Канал к потоку-проигрывателю. `None` — сигналов не будет (нет файла или не открылся).
fn player() -> Option<&'static Sender<Cue>> {
    static P: OnceLock<Option<Sender<Cue>>> = OnceLock::new();
    P.get_or_init(|| {
        let source = crate::models::app_dir().join(FILE);
        if !source.is_file() {
            crate::logln!("sound: {} нет — сигналы не играем", source.display());
            return None;
        }
        spawn(source)
    })
    .as_ref()
}

fn spawn(source: PathBuf) -> Option<Sender<Cue>> {
    let (tx, rx) = channel::<Cue>();
    std::thread::Builder::new()
        .name("tvoice-sound".into())
        .spawn(move || {
            let start = Device::open("tvoiceRecStart", &source);
            // Развёрнутую копию готовим здесь же: декодирование занимает десятки
            // миллисекунд, и держать на них поток запуска приложения незачем.
            let stop = match reversed_file(&source) {
                Ok(path) => Device::open("tvoiceRecStop", &path),
                Err(e) => {
                    crate::logln!("sound: не подготовить обратный сигнал ({e}) — будет тишина");
                    None
                }
            };
            if start.is_none() && stop.is_none() {
                return;
            }
            while let Ok(cue) = rx.recv() {
                let device = match cue {
                    Cue::Start => start.as_ref(),
                    Cue::Stop => stop.as_ref(),
                };
                if let Some(d) = device {
                    d.play();
                }
            }
        })
        .ok()?;
    Some(tx)
}

/// Открытое MCI-устройство под своим псевдонимом.
struct Device {
    alias: &'static str,
    path: PathBuf,
}

impl Device {
    fn open(alias: &'static str, path: &Path) -> Option<Self> {
        let d = Self {
            alias,
            path: path.to_path_buf(),
        };
        if !d.reopen() {
            crate::logln!("sound: не открыть {} — сигнал отключён", path.display());
            return None;
        }
        crate::logln!("sound: готов {} — {}", alias, path.display());
        Some(d)
    }

    fn reopen(&self) -> bool {
        mci(&format!("open \"{}\" alias {}", self.path.display(), self.alias))
    }

    fn play(&self) {
        // «from 0» и играет, и перематывает: частые нажатия просто начинают заново.
        if mci(&format!("play {} from 0", self.alias)) {
            return;
        }
        // Устройство вывода могло уйти (переключили наушники) — переоткрываем.
        crate::logln!("sound: устройство вывода сменилось — переоткрываю {}", self.alias);
        mci(&format!("close {}", self.alias));
        if self.reopen() {
            mci(&format!("play {} from 0", self.alias));
        }
    }
}

/// Путь к развёрнутой копии; при необходимости создаёт её.
///
/// Пересобираем, только если исходник новее готового файла: декодирование не бесплатное,
/// а `rec.mp3` меняется редко.
fn reversed_file(source: &Path) -> anyhow::Result<PathBuf> {
    let dir = crate::models::app_dir().join("temp");
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(REVERSED);
    if is_fresh(source, &out) {
        return Ok(out);
    }

    let pcm = crate::mp3::decode(source)?;
    let channels = pcm.channels.max(1) as usize;
    let mut samples = pcm.samples;
    let decoded = samples.len() / channels;
    // Кодер mp3 добавляет к потоку служебные кадры, и декодер отдаёт их как тишину: замер
    // на rec.mp3 — 120мс звука против 181мс на выходе. После разворота эта тишина окажется
    // В НАЧАЛЕ, и сигнал выхода запаздывал бы на глаз. Режем тишину с двух концов.
    trim_silence(&mut samples, channels);

    // Переворачиваем кадрами, а не сэмплами: у стерео иначе поменялись бы местами каналы.
    let mut frames = samples.len() / channels;
    for f in 0..frames / 2 {
        let (a, b) = (f * channels, (frames - 1 - f) * channels);
        for c in 0..channels {
            samples.swap(a + c, b + c);
        }
    }

    // Развёрнутый звук кончается там, где у исходника была атака, то есть обрывается на
    // полной громкости — на слух это щелчок. Подрезаем хвост и сводим остаток к нулю.
    //
    // Громкость запоминаем до обработки и возвращаем после: и подрезка, и спад бьют именно
    // по самому громкому месту (оно теперь в конце), из-за чего сигнал выхода выходил на
    // 14 дБ тише сигнала входа. Нам нужно ровное окончание, а не тихий звук.
    let loudness = peak(&samples);
    let cut = trim_tail(&mut samples, channels);
    fade_out(&mut samples, channels, pcm.rate);
    match_peak(&mut samples, loudness);

    let mut w = crate::mic::wav::WavWriter::create(&out, pcm.channels, pcm.rate)?;
    w.write_i16(&samples)?;
    w.finalize()?;
    frames = samples.len() / channels;
    crate::logln!(
        "sound: обратный сигнал собран — {} ({} кадров из {}, {:.0} мс, срезано {:.0} мс, {} Гц, {} кан)",
        out.display(),
        frames,
        decoded,
        frames as f32 * 1000.0 / pcm.rate as f32,
        cut as f32 * 1000.0 / pcm.rate as f32,
        pcm.rate,
        pcm.channels
    );
    Ok(out)
}

/// Какую долю развёрнутого звука подрезать с конца.
const TAIL_CUT: f32 = 0.15;
/// Какую долю остатка сводим к нулю в конце.
///
/// Доля, а не абсолютное время: сигнал короткий (у rec.mp3 слышимая часть 13мс), и
/// фиксированные 12мс оказались длиннее всего звука — затухание гасило файл целиком,
/// пик падал с −13.9 до −38 дБ.
const FADE_PART: f32 = 0.30;
/// …но не дольше этого: на длинном звуке доля дала бы неоправданно долгий спад.
const FADE_MAX: Duration = Duration::from_millis(20);

/// Подрезать конец. Возвращает, сколько кадров убрали.
///
/// У развёрнутого звука в конце оказывается атака исходника — самая громкая часть. Даже со
/// сведением к нулю она звучит резко, поэтому сначала снимаем небольшую долю.
fn trim_tail(samples: &mut Vec<i16>, channels: usize) -> usize {
    let frames = samples.len() / channels;
    let cut = (frames as f32 * TAIL_CUT) as usize;
    if cut == 0 || cut >= frames {
        return 0;
    }
    samples.truncate((frames - cut) * channels);
    cut
}

/// Свести громкость к нулю в конце, чтобы звук не обрывался щелчком.
fn fade_out(samples: &mut [i16], channels: usize, rate: u32) {
    let frames = samples.len() / channels;
    if frames == 0 {
        return;
    }
    let cap = FADE_MAX.as_millis() as usize * rate as usize / 1000;
    let span = ((frames as f32 * FADE_PART) as usize).min(cap).max(1);
    for i in 0..span {
        // Спад квадратичный, а не линейный. У этого звука амплитуда к концу РАСТЁТ (там
        // атака исходника), и линейный спад она почти компенсировала: последняя миллисекунда
        // оказывалась всего на 2.7 дБ ниже пика — обрыв оставался слышен.
        let t = (span - i) as f32 / span as f32;
        let gain = t * t;
        let f = frames - span + i;
        for c in 0..channels {
            let s = &mut samples[f * channels + c];
            *s = (*s as f32 * gain) as i16;
        }
    }
}

/// Наибольшая амплитуда.
fn peak(samples: &[i16]) -> i32 {
    samples.iter().map(|s| (*s as i32).abs()).max().unwrap_or(0)
}

/// Привести громкость к заданному пику (тише — усилить, громче — приглушить).
fn match_peak(samples: &mut [i16], target: i32) {
    let now = peak(samples);
    if now == 0 || target == 0 {
        return;
    }
    let gain = target as f32 / now as f32;
    for s in samples.iter_mut() {
        *s = (*s as f32 * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
    }
}

/// Ниже какой амплитуды кадр считаем тишиной (≈ −48 дБ от предела).
const SILENCE: i16 = 128;

/// Убрать тишину с обоих концов (по кадрам, целиком по всем каналам).
fn trim_silence(samples: &mut Vec<i16>, channels: usize) {
    let frames = samples.len() / channels;
    let loud = |f: usize| {
        samples[f * channels..(f + 1) * channels]
            .iter()
            .any(|s| s.saturating_abs() > SILENCE)
    };
    let first = (0..frames).find(|&f| loud(f));
    let Some(first) = first else {
        return; // сплошная тишина — резать нечего, пусть остаётся как есть
    };
    let last = (0..frames).rev().find(|&f| loud(f)).unwrap_or(frames - 1);
    samples.truncate((last + 1) * channels);
    samples.drain(..first * channels);
}

/// Готовый файл не старше исходника?
fn is_fresh(source: &Path, out: &Path) -> bool {
    let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (stamp(source), stamp(out)) {
        (Some(src), Some(dst)) => dst >= src,
        _ => false,
    }
}

/// Отправить команду MCI. `true` — успех (MCI отвечает нулём).
fn mci(cmd: &str) -> bool {
    let text = HSTRING::from(cmd);
    unsafe { mciSendStringW(PCWSTR(text.as_ptr()), None, None) == 0 }
}
