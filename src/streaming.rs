//! Потоковая диктовка «как на iPhone»: слова появляются во время речи и уточняются на месте.
//!
//! Текущая фраза — это черновик. Каждые ~400мс распознаём её целиком и сравниваем с тем,
//! что уже вставлено: общий префикс оставляем, расхождение стираем Backspace'ами и дописываем
//! заново. Поэтому последнее слово не ждёт подтверждения (как было в схеме «вставляем только
//! то, что совпало дважды») — оно появляется сразу и при необходимости поправляется, а
//! пунктуация и заглавные буквы дозревают вместе с фразой.
//!
//! На паузе фраза фиксируется: дальше её уже не трогаем — переписывать разрешено только
//! незавершённую фразу, и только пока пользователь сам не притронулся к клавиатуре/мыши.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::dictation::SharedDictation;
use crate::mic::LiveCapture;
use crate::vad::{Segment, Vad};
use crate::{audio, inject, models, server, userinput};

/// Как часто просыпаемся смотреть на буфер.
const TICK: Duration = Duration::from_millis(100);
/// Базовый интервал между уточнениями черновика.
const DRAFT_EVERY: Duration = Duration::from_millis(400);
/// Тишина, завершающая фразу (сверх «звона» VAD в 300мс — итого пауза ~0.8с).
const PAUSE: Duration = Duration::from_millis(500);
/// Минимум речи, с которого есть смысл звать whisper (на огрызках он выдумывает).
const MIN_SPEECH: Duration = Duration::from_millis(700);
/// Минимум речи, чтобы участок вообще считался фразой, а не случайным звуком.
const MIN_PHRASE: Duration = Duration::from_millis(400);
/// Как часто пишем в лог состояние VAD.
const REPORT_EVERY: Duration = Duration::from_millis(700);
/// Предел длины фразы: ограничивает и объём перезаписи, и время распознавания.
const MAX_PHRASE: Duration = Duration::from_secs(12);
/// Пауза после сигнала «стоп» — дать потоку захвата дослать хвост.
const DRAIN: Duration = Duration::from_millis(250);
/// Предел одной правки: больше — значит, мы уже не понимаем, что в окне.
const MAX_REWRITE: usize = 120;

pub fn run(
    live: LiveCapture,
    stop: Arc<AtomicBool>,
    model_file: String,
    lang: String,
    insert: bool,
    status: SharedDictation,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        crate::logln!("stream: старт (model={model_file}, lang={lang}, insert={insert})");
        let tmp = models::app_dir().join("temp");
        let _ = std::fs::create_dir_all(&tmp);
        let wav = tmp.join("tvoice_stream.wav");

        // Настоящую частоту WASAPI проставляет, когда откроет устройство, — уже после
        // старта этого потока. Поэтому читаем её каждый цикл и пересобираем VAD, когда
        // она станет известна: на чужой частоте развалятся и длительности, и сам звук.
        let mut rate = 0usize;
        let mut vad = Vad::new(16_000);
        let mut target = Target::new(insert, status.clone(), ctx.clone());
        let mut committed = 0usize; // начало текущей фразы в сэмплах
        let mut phrases = 0u32;
        let mut drafts = 0u32;
        let mut drained = false;
        let mut next_draft = Instant::now();
        let mut next_report = Instant::now();

        loop {
            let stopping = stop.load(Ordering::Relaxed);
            if stopping && !drained {
                drained = true;
                thread::sleep(DRAIN); // хвост записи ещё в пути
            }

            let now_rate = live.rate.load(Ordering::Relaxed).max(1) as usize;
            if now_rate != rate {
                crate::logln!("stream: частота захвата {now_rate} Гц (кадр VAD {} сэмплов)", now_rate * 20 / 1000);
                rate = now_rate;
                vad = Vad::new(rate);
                committed = 0;
            }

            {
                let b = live.buf.lock().unwrap();
                vad.feed(&b);
            }
            let seg = vad.segment(committed);
            if seg.analysed == 0 {
                if stopping {
                    break;
                }
                thread::sleep(TICK);
                continue;
            }

            let secs = |n: usize| Duration::from_secs_f32(n as f32 / rate as f32);
            let silence = secs(seg.silence);
            let speech = secs(seg.speech_end);
            let voiced = secs(seg.speech);
            let phrase = secs(seg.analysed);

            if Instant::now() >= next_report {
                next_report = Instant::now() + REPORT_EVERY;
                crate::logln!(
                    "vad: rms={:.4} шум={:.4}±{:.4} порог={:.4} | фраза {:.1}с: речь {:.1}с, до {:.1}с, тишина {:.1}с",
                    vad.last_rms(),
                    vad.noise(),
                    vad.dev(),
                    vad.on(),
                    phrase.as_secs_f32(),
                    voiced.as_secs_f32(),
                    speech.as_secs_f32(),
                    silence.as_secs_f32()
                );
            }

            let boundary =
                stopping || (seg.has_speech && silence >= PAUSE) || phrase >= MAX_PHRASE;

            if !boundary {
                if !seg.has_speech {
                    // Одна тишина: сдвигаем окно, чтобы whisper не жевал её каждый раз.
                    if phrase > PAUSE {
                        committed += seg.analysed;
                    }
                    set_state(&status, &ctx, "Слушаю… (говорите)");
                    thread::sleep(TICK);
                    continue;
                }
                if voiced < MIN_SPEECH || Instant::now() < next_draft {
                    set_state(&status, &ctx, "Слушаю… (говорите)");
                    thread::sleep(TICK);
                    continue;
                }

                let t0 = Instant::now();
                let cut = cut_len(&seg, rate);
                let hyp = hypothesis(&snapshot(&live, committed, cut), rate, &wav, &model_file, &lang);
                // Не душим GPU: тяжёлой модели даём время между уточнениями.
                next_draft = Instant::now() + DRAFT_EVERY.max(t0.elapsed().mul_f32(1.5));
                drafts += 1;
                if let Some(text) = hyp {
                    crate::logln!(
                        "stream: черновик {:.1}с → {:.2}с: {text:?}",
                        secs(cut).as_secs_f32(),
                        t0.elapsed().as_secs_f32()
                    );
                    if !text.is_empty() {
                        target.update(&text);
                    }
                }
                continue;
            }

            // Граница фразы: последнее уточнение — и фиксируем.
            let reason = if stopping {
                "стоп"
            } else if phrase >= MAX_PHRASE {
                "лимит длины"
            } else {
                "пауза"
            };
            if seg.has_speech && voiced < MIN_PHRASE {
                // Короткий всплеск (стук, кашель, слог из соседней комнаты): распознавать
                // бессмысленно — whisper на таких огрызках выдумывает текст.
                crate::logln!(
                    "stream: всплеск {:.2}с ({reason}) — пропущен, не речь",
                    voiced.as_secs_f32()
                );
            } else if seg.has_speech {
                phrases += 1;
                let t0 = Instant::now();
                let cut = cut_len(&seg, rate);
                let hyp = hypothesis(&snapshot(&live, committed, cut), rate, &wav, &model_file, &lang);
                if let Some(text) = hyp {
                    if !text.is_empty() {
                        target.update(&text);
                    }
                }
                crate::logln!(
                    "stream[{phrases}]: граница ({reason}), речь {:.1}с из {:.1}с, отправлено {:.1}с → {:.2}с: {:?}",
                    voiced.as_secs_f32(),
                    phrase.as_secs_f32(),
                    secs(cut).as_secs_f32(),
                    t0.elapsed().as_secs_f32(),
                    target.draft()
                );
            }
            target.commit();
            // Хвост тишины оставляем в буфере: если VAD ошибся и принял начало следующего
            // слова за тишину, оно не должно пропасть вместе со сдвигом окна.
            committed += (seg.speech_end + rate * 3 / 10).min(seg.analysed);
            next_draft = Instant::now();
            if stopping {
                break;
            }
        }

        crate::logln!("stream: завершено, фраз: {phrases}, уточнений: {drafts}");
        if let Ok(mut s) = status.lock() {
            s.busy = false;
            s.state = "Готово ✓".into();
        }
        ctx.request_repaint();
        let _ = std::fs::remove_file(&wav);
    });
}

/// Черновик текущей фразы в окне-приёмнике: знает, что там сейчас написано,
/// и умеет привести это к новой версии минимальной правкой.
struct Target {
    insert: bool,
    /// Уже зафиксированные фразы — перед новой нужен пробел.
    prior: bool,
    /// Что сейчас вставлено для текущей фразы (ровно то, что видит пользователь).
    shown: String,
    /// Весь текст сеанса (для окна приложения).
    full: String,
    /// Отметка нашей последней вставки — по ней понимаем, вмешался ли пользователь.
    stamp: u64,
    status: SharedDictation,
    ctx: egui::Context,
}

impl Target {
    fn new(insert: bool, status: SharedDictation, ctx: egui::Context) -> Self {
        Self {
            insert,
            prior: false,
            shown: String::new(),
            full: String::new(),
            stamp: userinput::mark(),
            status,
            ctx,
        }
    }

    fn draft(&self) -> &str {
        self.shown.trim_start()
    }

    /// Привести вставленный черновик к новой версии фразы.
    fn update(&mut self, phrase: &str) {
        let want = if self.prior {
            format!(" {phrase}")
        } else {
            phrase.to_string()
        };
        if want == self.shown {
            return;
        }
        let keep = common_prefix(&self.shown, &want);
        let back = self.shown.chars().count() - keep;
        let add: String = want.chars().skip(keep).collect();
        if back > MAX_REWRITE {
            // Whisper переосмыслил фразу целиком: столько стирать нельзя — пропускаем
            // это уточнение (черновик остаётся прежним) и ждём следующего.
            crate::logln!("stream: правка на {back} символов пропущена (слишком много)");
            return;
        }

        if self.insert {
            // Стирать можно, только если после нашей же вставки никто не печатал и не
            // кликал: иначе курсор уже не там, и Backspace съел бы чужой текст.
            if back > 0 && userinput::touched_since(self.stamp) {
                crate::logln!("stream: пользователь вмешался — фраза зафиксирована как есть");
                self.commit();
                return;
            }
            inject::queue_replace(back, &add);
            self.stamp = userinput::mark();
        }
        self.shown = want;
        if let Ok(mut s) = self.status.lock() {
            s.last_text = format!("{}{}", self.full, self.shown);
        }
        self.ctx.request_repaint();
    }

    /// Фраза окончена: больше её не трогаем.
    fn commit(&mut self) {
        if self.shown.is_empty() {
            return;
        }
        self.full.push_str(&self.shown);
        self.shown.clear();
        self.prior = true;
    }
}

/// Сколько аудио отправляем в whisper: речь плюс небольшой хвост тишины.
/// Резать вплотную по границе речи нельзя — VAD может ошибиться на полслога,
/// и модель получит обрубок, из которого выдумает слово.
fn cut_len(seg: &Segment, rate: usize) -> usize {
    (seg.speech_end + rate / 2).min(seg.analysed)
}

/// Сколько первых символов у строк совпадает.
pub fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

/// Копия куска живого буфера от `from` длиной `len` (для отправки в whisper).
fn snapshot(live: &LiveCapture, from: usize, len: usize) -> Vec<f32> {
    let b = live.buf.lock().unwrap();
    let end = (from + len).min(b.len());
    b.get(from..end).map(<[f32]>::to_vec).unwrap_or_default()
}

/// Распознать кусок (пусто — если тишина/мусор/ошибка).
fn hypothesis(
    seg: &[f32],
    rate: usize,
    wav: &std::path::Path,
    model_file: &str,
    lang: &str,
) -> Option<String> {
    if audio::write_16k_wav_from_mono(seg, rate as u32, wav).is_err() {
        return None;
    }
    let text = match server::transcribe(wav, model_file, lang) {
        Ok(t) => t,
        Err(e) => {
            crate::logln!("stream: распознавание ОШИБКА: {e}");
            return None;
        }
    };
    let clean = text.trim().to_string();
    if is_hallucination(&clean) {
        crate::logln!("stream: отброшено как галлюцинация: {clean:?}");
        return Some(String::new());
    }
    Some(clean)
}

/// Whisper на тишине/шуме любит выдавать заготовки из титров — их вставлять нельзя.
fn is_hallucination(text: &str) -> bool {
    const JUNK: [&str; 7] = [
        "субтитры",
        "продолжение следует",
        "редактор субтитров",
        "dimatorzok",
        "amara.org",
        "subscribe",
        "спасибо за просмотр",
    ];
    let low = text.to_lowercase();
    JUNK.iter().any(|j| low.contains(j))
}

fn set_state(status: &SharedDictation, ctx: &egui::Context, text: &str) {
    if let Ok(mut s) = status.lock() {
        if s.state != text {
            s.busy = true;
            s.state = text.to_string();
            ctx.request_repaint();
        }
    }
}
