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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
/// Сколько последних слов фразы разрешено переписывать.
///
/// Дальше в прошлое не лезем. Whisper с каждым уточнением то ставит, то убирает запятую
/// в середине фразы, а сравнение идёт посимвольно от начала — из-за одной такой правки
/// стирался и набирался заново весь хвост, и текст в окне мигал. Начало фразы после
/// нескольких слов уже не меняется по смыслу, поэтому его замораживаем.
const TAIL_WORDS: usize = 4;
/// На сколько слов новая гипотеза может разойтись с показанной в начале фразы.
///
/// Whisper от уточнения к уточнению то склеивает, то разбивает начало («О, привет!» одним
/// сегментом или двумя), из-за чего номера слов съезжают. Столько сдвига прощаем при
/// поиске места склейки.
const WORD_SHIFT: usize = 3;
/// Где искать межсловную паузу, когда фраза упёрлась в лимит длины.
const GAP_SEARCH: Duration = Duration::from_millis(1500);
/// Сколько молчания заканчивает саму диктовку.
///
/// Хоткей легко зажать и забыть (а кнопкой мыши — тем более), и тогда микрофон остаётся
/// открытым, индикатор висит, а whisper молотит тишину. По истечении этого времени поток
/// заканчивает работу сам и просит приложение погасить захват.
const IDLE_LIMIT: Duration = Duration::from_secs(10);
/// Сколько тишины оставляем в буфере на случай, что речь уже началась.
///
/// Обрезали вплотную — и мягкое начало фразы уезжало вместе с тишиной: VAD признаёт речь
/// только после RISE кадров подряд выше порога, а всё, что было до этого, уже считалось
/// тишиной и выбрасывалось. whisper получал фразу без первых слов («сразу говорить…»
/// вместо «я начинаю сразу говорить…»).
const PREROLL: Duration = Duration::from_millis(400);

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
        // Не пора ли закончить самим (см. IDLE_LIMIT).
        let mut idle_stop = false;

        loop {
            let asked = stop.load(Ordering::Relaxed);
            let stopping = asked || idle_stop;
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

            // Молчание меряем по энергии записи, а не по разметке речь/тишина и не по
            // часам: разметка «звенит» и может защёлкнуться (тогда автовыход не сработал бы
            // вовсе), а часы не знают, был ли звук. См. `Vad::quiet_tail`.
            let quiet = secs(vad.quiet_tail());
            if !stopping && quiet >= IDLE_LIMIT {
                crate::logln!(
                    "stream: молчание {:.1}с — заканчиваю диктовку сам",
                    quiet.as_secs_f32()
                );
                idle_stop = true;
                continue; // следующий круг уже пойдёт как «стоп»: зафиксирует хвост и выйдет
            }

            if Instant::now() >= next_report {
                next_report = Instant::now() + REPORT_EVERY;
                crate::logln!(
                    "vad: rms={:.4} шум={:.4}±{:.4} порог={:.4} | фраза {:.1}с: речь {:.1}с, до {:.1}с, тишина {:.1}с, молчит {:.1}с",
                    vad.last_rms(),
                    vad.noise(),
                    vad.dev(),
                    vad.on(),
                    phrase.as_secs_f32(),
                    voiced.as_secs_f32(),
                    speech.as_secs_f32(),
                    silence.as_secs_f32(),
                    quiet.as_secs_f32()
                );
            }

            let boundary =
                stopping || (seg.has_speech && silence >= PAUSE) || phrase >= MAX_PHRASE;

            if !boundary {
                if !seg.has_speech {
                    // Одна тишина: сдвигаем окно, чтобы whisper не жевал её каждый раз,
                    // но оставляем запас — начало фразы может быть уже в этой «тишине».
                    if phrase > PAUSE {
                        let preroll = PREROLL.as_millis() as usize * rate / 1000;
                        committed += seg.analysed.saturating_sub(preroll);
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
                let chunk = snapshot(&live, committed, cut);
                let sound = audio::stats(&chunk, rate as u32);
                let hyp = hypothesis(&chunk, rate, &wav, &model_file, &lang);
                // Не душим GPU: тяжёлой модели даём время между уточнениями.
                next_draft = Instant::now() + DRAFT_EVERY.max(t0.elapsed().mul_f32(1.5));
                drafts += 1;
                if let Some(text) = hyp {
                    crate::logln!(
                        "stream: черновик {:.1}с → {:.2}с | {sound} | {text:?}",
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
            //
            // На лимите длины речь ещё идёт, и резать «где застали» нельзя: разрез
            // попадает посреди слова. Ищем ближайшую межсловную паузу и режем по ней —
            // после `commit()` фраза уже не исправится.
            let limit = !stopping && silence < PAUSE && phrase >= MAX_PHRASE;
            let gap = limit
                .then(|| vad.quietest(committed, GAP_SEARCH.as_millis() as usize * rate / 1000))
                .flatten()
                // Нулевой разрез не сдвинул бы окно — это вечный цикл. Такой не берём.
                .filter(|&g| g > 0);
            let reason = match (stopping, limit, gap.is_some()) {
                (true, ..) => "стоп",
                (_, true, true) => "лимит длины, разрез по паузе",
                (_, true, false) => "лимит длины",
                _ => "пауза",
            };
            // Кусок для распознавания и на сколько сдвинуть окно. По паузе режем ровно в
            // тихой точке: слева и справа от неё звук чистый, поэтому ни хвост, ни
            // перекрытие не нужны.
            let (cut, advance) = match gap {
                Some(g) => (g, g),
                None => (
                    cut_len(&seg, rate),
                    // Хвост тишины оставляем в буфере: если VAD ошибся и принял начало
                    // следующего слова за тишину, оно не должно пропасть со сдвигом окна.
                    (seg.speech_end + rate * 3 / 10).min(seg.analysed),
                ),
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
                let chunk = snapshot(&live, committed, cut);
                let sound = audio::stats(&chunk, rate as u32);
                let hyp = hypothesis(&chunk, rate, &wav, &model_file, &lang);
                if let Some(text) = hyp {
                    if !text.is_empty() {
                        target.update(&text);
                    }
                }
                crate::logln!(
                    "stream[{phrases}]: граница ({reason}), речь {:.1}с из {:.1}с, отправлено {:.1}с → {:.2}с | {sound} | окно {committed}..+{cut} | {:?}",
                    voiced.as_secs_f32(),
                    phrase.as_secs_f32(),
                    secs(cut).as_secs_f32(),
                    t0.elapsed().as_secs_f32(),
                    target.draft()
                );
            }
            target.commit();
            committed += advance;
            next_draft = Instant::now();
            if stopping {
                break;
            }
        }

        crate::logln!("stream: завершено, фраз: {phrases}, уточнений: {drafts}");
        if let Ok(mut s) = status.lock() {
            s.busy = false;
            s.state = if idle_stop {
                format!("Тишина {}с — захват остановлен", IDLE_LIMIT.as_secs())
            } else {
                "Готово ✓".into()
            };
            // Останов по тишине приложение должно довести до конца: погасить микрофон,
            // снять состояние «диктую» и убрать индикатор — про это знает только оно.
            s.auto_stop = idle_stop;
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
    /// То же по словам: начало фразы заморожено, правим только хвост.
    words: Vec<String>,
    /// Весь текст сеанса (для окна приложения).
    full: String,
    /// Отметка нашей последней вставки — по ней понимаем, вмешался ли пользователь.
    stamp: u64,
    /// Размер последней правки (стёрто, вставлено) — для лога и проверки.
    last_edit: (usize, usize),
    status: SharedDictation,
    ctx: egui::Context,
}

impl Target {
    fn new(insert: bool, status: SharedDictation, ctx: egui::Context) -> Self {
        Self {
            insert,
            prior: false,
            shown: String::new(),
            words: Vec::new(),
            full: String::new(),
            stamp: userinput::mark(),
            last_edit: (0, 0),
            status,
            ctx,
        }
    }

    fn draft(&self) -> &str {
        self.shown.trim_start()
    }

    /// Привести вставленный черновик к новой версии фразы.
    fn update(&mut self, phrase: &str) {
        let fresh: Vec<&str> = phrase.split_whitespace().collect();
        if fresh.is_empty() {
            return;
        }
        // Всё, кроме последних слов, оставляем как есть — правки в глубине фразы
        // (обычно скачущая запятая) не стоят перенабора всего хвоста.
        let frozen = self.words.len().saturating_sub(TAIL_WORDS);
        let Some(tail) = tail_start(&self.words[..frozen], &fresh) else {
            crate::logln!("stream: уточнение пропущено — не сошлось с началом показанной фразы");
            return;
        };
        let mut next: Vec<String> = self.words[..frozen].to_vec();
        next.extend(fresh[tail..].iter().map(|s| s.to_string()));
        let body = next.join(" ");
        let want = if self.prior {
            format!(" {body}")
        } else {
            body
        };
        if want == self.shown {
            return;
        }
        let keep = common_prefix(&self.shown, &want);
        let mut back = self.shown.chars().count() - keep;
        let mut add: String = want.chars().skip(keep).collect();
        // Вставку нельзя начинать с пробела. В поле ввода на contenteditable (мессенджер в
        // браузере) ведущий пробел вставки пропадает, и слова слипаются: черновик
        // «я начинаю сразу говорить» вставляется как «сразу» + « говорить», а в окне
        // получается «сразуговорить». В Блокноте этого не видно — потому и вылезло не сразу.
        // Сдвигаем границу правки на один символ назад: пробел оказывается ВНУТРИ вставки,
        // где его уже никто не съедает, а лишний стёртый символ мы тут же печатаем заново.
        if add.starts_with(' ') && keep > 0 {
            if let Some(prev) = want.chars().nth(keep - 1) {
                back += 1;
                add.insert(0, prev);
            }
        }
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
        self.words = next;
        self.last_edit = (back, add.chars().count());
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
        self.words.clear();
        self.prior = true;
    }
}

/// С какого слова новой гипотезы начинается ещё не замороженный хвост фразы.
///
/// Брать `fresh[frozen..]` по номеру нельзя. Whisper между уточнениями то добавляет, то
/// убирает слово в начале фразы, и склейка по индексу тогда либо дублирует слово, либо
/// съедает его: «Это система из маленьких конструкций.» превратилось в «Это маленьких
/// конструкторов.» — «система из» пропали, а правка выросла с 5 символов до 33 (чем больше
/// правка, тем больше шансов, что часть удалений потеряется по пути в окно).
///
/// Поэтому ищем последнее замороженное слово («якорь») рядом с ожидаемым местом и режем
/// сразу за ним. Идём от ожидаемой позиции наружу: ближайшее совпадение вернее дальнего,
/// иначе повторяющееся во фразе слово утащило бы разрез не туда. Не нашли вовсе — склеивать
/// наугад нельзя, уточнение лучше пропустить и дождаться следующего.
fn tail_start(frozen: &[String], fresh: &[&str]) -> Option<usize> {
    let Some(anchor) = frozen.last() else {
        return Some(0); // ничего не заморожено — фраза набирается целиком
    };
    let want = frozen.len() - 1;
    (0..=WORD_SHIFT)
        .flat_map(|d| [want + d, want.saturating_sub(d)])
        .find(|&i| fresh.get(i).is_some_and(|w| same_word(w, anchor)))
        .map(|i| i + 1)
}

/// Одно ли это слово «по существу»: без пунктуации, без регистра и не различая е/ё.
///
/// Якорь склейки — обычное слово из гипотезы, а whisper от уточнения к уточнению меняет у
/// слов ровно оформление: «привет!»/«привет.», «пришёл»/«пришел», `"I'm`/`«I'm`. Сравнивать
/// буквально нельзя — якорь «терялся» на ровном месте, и уточнение пропадало целиком,
/// оставляя в окне заведомо неверный текст до следующего круга.
fn same_word(a: &str, b: &str) -> bool {
    let core = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .map(|c| if c == 'ё' { 'е' } else { c })
            .collect::<String>()
    };
    let (ca, cb) = (core(a), core(b));
    if ca.is_empty() || cb.is_empty() {
        return a == b; // сплошная пунктуация — сравнивать нечего, кроме неё самой
    }
    ca == cb
}

/// Прогнать последовательность «черновиков» через ту же логику правки без вставки:
/// показывает, сколько символов пришлось бы стереть и дописать на каждом шаге.
/// Нужно, чтобы проверять числом, что текст в окне не переписывается целиком.
pub fn simulate(drafts: &[String]) -> Vec<(usize, usize, String)> {
    let ctx = egui::Context::default();
    let mut target = Target::new(false, crate::dictation::new_shared(), ctx);
    let mut out = Vec::new();
    for d in drafts {
        target.last_edit = (0, 0);
        target.update(d);
        out.push((target.last_edit.0, target.last_edit.1, target.shown.clone()));
    }
    out
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
    let got = b.get(from..end).map(<[f32]>::to_vec).unwrap_or_default();
    if got.len() < len {
        // Окно ушло за конец буфера: либо захват отстал, либо `committed` уехал вперёд и
        // VAD размечает речь по кадрам, которых в буфере уже/ещё нет. Снаружи это выглядит
        // как «диктовка молчит», поэтому пишем громко, а не глотаем.
        crate::logln!(
            "stream: ОКНО КОРОЧЕ ЗАПРОСА — просили {len}, взяли {} (от {from}, в буфере {})",
            got.len(),
            b.len()
        );
    }
    got
}

/// Распознать кусок (пусто — если тишина/мусор/ошибка).
fn hypothesis(
    seg: &[f32],
    rate: usize,
    wav: &std::path::Path,
    model_file: &str,
    lang: &str,
) -> Option<String> {
    let secs = seg.len() as f32 / rate.max(1) as f32;
    if let Err(e) = audio::write_16k_wav_from_mono(seg, rate as u32, wav) {
        crate::logln!("stream: не записать wav ({} сэмплов @{rate} Гц): {e}", seg.len());
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
    let junk = is_hallucination(&clean);
    if junk || clean.is_empty() {
        // Ровно тот случай, который снаружи выглядит как «диктовка не работает»: звук идёт,
        // индикатор дёргается, а текста нет. По тексту ответа причину не отличить, поэтому
        // печатаем признаки звука и сохраняем сам кусок — его можно послушать.
        crate::logln!(
            "stream: ПУСТО {} — whisper вернул {clean:?} | звук {secs:.2}с @{rate} Гц: {}",
            if junk { "(заготовка)" } else { "(пустой ответ)" },
            audio::stats(seg, rate as u32)
        );
        dump_chunk(seg, rate);
        return Some(String::new());
    }
    Some(clean)
}

/// Сколько «пустых» кусков сохраняем за запуск приложения.
const DUMPS: usize = 8;

/// Сохранить кусок, на котором распознавание ничего не дало.
///
/// Без записи причина неотличима: по логу видно только, что текста нет. С файлом можно
/// послушать звук и прогнать его вручную через whisper. Число ограничено — сеанс не должен
/// забить диск, а для разбора хватает первых.
fn dump_chunk(seg: &[f32], rate: usize) {
    static SAVED: AtomicUsize = AtomicUsize::new(0);
    let n = SAVED.fetch_add(1, Ordering::Relaxed);
    if n >= DUMPS {
        return;
    }
    let dir = models::app_dir().join("debug");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::logln!("stream: не создать {}: {e}", dir.display());
        return;
    }
    let path = dir.join(format!("empty_{n:02}.wav"));
    match audio::write_16k_wav_from_mono(seg, rate as u32, &path) {
        Ok(()) => crate::logln!("stream: пустой кусок сохранён — {}", path.display()),
        Err(e) => crate::logln!("stream: не сохранить кусок: {e}"),
    }
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
