//! Определение речи/тишины с адаптивным порогом.
//!
//! Фиксированный порог по амплитуде не работает: у одного микрофона фон 0.002, у другого
//! 0.02 — и во втором случае «тишина» не наступает никогда, паузы между фразами не находятся,
//! а whisper молотит шум и выдаёт галлюцинации.
//!
//! Считаем энергию (RMS) по кадрам 20мс. Уровень фона — низкий процентиль энергии за
//! последние несколько секунд, и считается он БЕЗ участия решения «речь/тишина».
//!
//! Прежняя схема (экспоненциальный трекер, обновляемый только на кадрах без речи, плюс
//! «первые 0.5с — это шум») имела два состояния, из которых не было выхода:
//!
//! * если в те первые 0.5с человек уже говорил, его голос становился оценкой фона, порог
//!   уезжал выше речи — а обновлять оценку разрешалось только при `rms < порога`, то есть
//!   она продолжала догонять голос. Диктовка глохла на весь сеанс;
//! * если фон поднимался выше порога, `in_speech` защёлкивался, оценка замирала, и речью
//!   считалось всё подряд — whisper получал шум и отдавал заготовку из титров.
//!
//! Снаружи оба выглядели одинаково: индикатор дёргается, текста нет. Процентиль по окну от
//! решения не зависит, поэтому обратной связи, которая защёлкивает оба состояния, больше нет:
//! в речи всегда есть провалы (смычки, промежутки между словами), и низкий процентиль
//! садится на них, а на сплошном фоне — на сам фон.
//!
//! Порог включения выше порога выключения (гистерезис), плюс два предохранителя: речь должна
//! продержаться несколько кадров, чтобы начаться, и ещё немного «звенит» после спада —
//! иначе паузы между словами рвут фразу.

/// Длительность кадра.
const FRAME_MS: usize = 20;
/// Окно оценки фона. Берём заведомо больше типичной фразы: в любых 5с речи есть провалы,
/// на которые садится процентиль, а короткое окно на сплошной речи задрало бы порог.
const WINDOW_MS: usize = 5000;
/// Какую долю самых тихих кадров окна считаем фоном.
const FLOOR_PCT: f32 = 0.05;
/// Во сколько раз речь должна быть громче фона, чтобы начаться.
///
/// Различаем не громкость, а форму: у белого шума энергия кадров ровная, поэтому пятый
/// процентиль почти равен его RMS (отношение ~1.1), а у речи между провалами и гласными
/// разница в разы. Поэтому запас можно держать небольшим. Ставили 3.0 — и на живом
/// микрофоне с фоном 0.03 речь в 0.07 (отношение 2.4) не проходила порог 0.09 вообще:
/// диктовка молчала целыми сеансами.
const ON_FACTOR: f32 = 2.0;
/// …и чтобы продолжаться (гистерезис).
const OFF_FACTOR: f32 = 1.4;
/// Абсолютные нижние границы порогов — на случай идеально тихого входа.
const MIN_ON: f32 = 0.008;
const MIN_OFF: f32 = 0.005;
/// Как часто пересчитываем процентиль (каждый кадр незачем — фон так быстро не меняется).
const RECALC_EVERY: usize = 5;
/// Сколько кадров подряд нужно превышать порог, чтобы это считалось началом речи (60мс).
const RISE: usize = 3;
/// Сколько кадров речь «звенит» после спада — мост через паузы между словами (300мс).
const HANG: usize = 15;

pub struct Vad {
    frame: usize,
    processed: usize,
    /// Кольцо энергий последних кадров — по нему оценивается фон.
    window: std::collections::VecDeque<f32>,
    /// Сколько кадров помещается в окно.
    capacity: usize,
    /// Оценка фона (низкий процентиль окна) и робастный разброс (для логов).
    floor: f32,
    spread: f32,
    since_recalc: usize,
    last_rms: f32,
    in_speech: bool,
    rise: usize,
    hang: usize,
    /// Речь/тишина по кадрам с начала записи.
    flags: Vec<bool>,
    /// Энергия по кадрам с начала записи — по ней ищем, где разрезать длинную фразу.
    energy: Vec<f32>,
}

impl Vad {
    pub fn new(rate: usize) -> Self {
        Self {
            frame: (rate * FRAME_MS / 1000).max(1),
            processed: 0,
            window: std::collections::VecDeque::new(),
            capacity: (WINDOW_MS / FRAME_MS).max(1),
            floor: 0.0,
            spread: 0.0,
            since_recalc: 0,
            last_rms: 0.0,
            in_speech: false,
            rise: 0,
            hang: 0,
            flags: Vec::new(),
            energy: Vec::new(),
        }
    }

    /// Разобрать новые сэмплы буфера захвата (всё, что появилось после прошлого вызова).
    pub fn feed(&mut self, buf: &[f32]) {
        while self.processed + self.frame <= buf.len() {
            let f = &buf[self.processed..self.processed + self.frame];
            self.processed += self.frame;
            let rms = (f.iter().map(|v| v * v).sum::<f32>() / f.len() as f32).sqrt();
            self.last_rms = rms;

            // Окно пополняем безусловно: оценка фона не должна зависеть от того, что мы
            // сами же решили про эти кадры, — именно эта обратная связь и защёлкивала VAD.
            if self.window.len() == self.capacity {
                self.window.pop_front();
            }
            self.window.push_back(rms);
            self.since_recalc += 1;
            if self.since_recalc >= RECALC_EVERY || self.floor == 0.0 {
                self.since_recalc = 0;
                self.recalc();
            }

            let on = self.on();
            let off = (self.floor * OFF_FACTOR).max(MIN_OFF);
            if self.in_speech {
                if rms >= off {
                    self.hang = HANG;
                } else if self.hang > 0 {
                    self.hang -= 1;
                } else {
                    self.in_speech = false;
                }
            } else if rms >= on {
                self.rise += 1;
                if self.rise >= RISE {
                    self.in_speech = true;
                    self.hang = HANG;
                }
            } else {
                self.rise = 0;
            }
            self.flags.push(self.in_speech);
            self.energy.push(rms);
        }
    }

    /// Пересчитать оценку фона: низкий процентиль окна.
    ///
    /// Разброс берём как «медиана минус фон» — робастная замена среднеквадратичному:
    /// нужен только для лога, но по нему видно, есть ли в окне вообще что-то громче фона.
    fn recalc(&mut self) {
        if self.window.is_empty() {
            return;
        }
        let mut v: Vec<f32> = self.window.iter().copied().collect();
        v.sort_by(f32::total_cmp);
        let i = ((v.len() as f32 * FLOOR_PCT) as usize).min(v.len() - 1);
        self.floor = v[i];
        self.spread = (v[v.len() / 2] - self.floor).max(0.0);
    }

    /// Порог включения речи.
    pub fn on(&self) -> f32 {
        (self.floor * ON_FACTOR).max(MIN_ON)
    }

    /// Оценённый уровень фона и его разброс (для лога).
    pub fn noise(&self) -> f32 {
        self.floor
    }

    pub fn dev(&self) -> f32 {
        self.spread
    }

    pub fn last_rms(&self) -> f32 {
        self.last_rms
    }

    /// Разбор участка записи, начиная с сэмпла `from`.
    pub fn segment(&self, from: usize) -> Segment {
        let f0 = (from / self.frame).min(self.flags.len());
        let flags = &self.flags[f0..];
        let silent_tail = flags.iter().rev().take_while(|&&s| !s).count();
        Segment {
            has_speech: flags.len() > silent_tail,
            speech_end: (flags.len() - silent_tail) * self.frame,
            silence: silent_tail * self.frame,
            analysed: flags.len() * self.frame,
            speech: flags.iter().filter(|&&s| s).count() * self.frame,
        }
    }

    /// Участки речи (начало, конец) в сэмплах — для разбора разметки глазами.
    pub fn spans(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = None;
        for (i, &sp) in self.flags.iter().enumerate() {
            match (sp, start) {
                (true, None) => start = Some(i),
                (false, Some(a)) => {
                    out.push((a * self.frame, i * self.frame));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(a) = start {
            out.push((a * self.frame, self.flags.len() * self.frame));
        }
        out
    }

    /// Самая тихая точка в конце участка — куда можно разрезать не кончившуюся фразу.
    ///
    /// Паузы между словами (50–150мс) короче «звона» HANG, поэтому в `flags` их не видно:
    /// там всё подряд помечено речью, и по ним место для разреза не найти. Ищем по энергии:
    /// смотрим последние `window` сэмплов участка и возвращаем начало самого тихого кадра.
    /// Разрез «где застали» приходился посреди слова, whisper отдавал его половинками в
    /// двух сегментах («готовых» → «готов» и «ых»), а фраза к тому моменту уже
    /// зафиксирована и не исправится.
    ///
    /// Позиция — в сэмплах от начала участка, как и всё в [`Segment`].
    pub fn quietest(&self, from: usize, window: usize) -> Option<usize> {
        let f0 = (from / self.frame).min(self.energy.len());
        let tail = &self.energy[f0..];
        let span = (window / self.frame).min(tail.len());
        if span == 0 {
            return None;
        }
        let base = tail.len() - span;
        let quietest = tail[base..]
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(i, _)| base + i)?;
        Some(quietest * self.frame)
    }
}

/// Результат разбора участка (всё в сэмплах, относительно начала участка).
pub struct Segment {
    pub has_speech: bool,
    /// Позиция конца речи (дальше — только тишина).
    pub speech_end: usize,
    /// Длина завершающей тишины.
    pub silence: usize,
    /// Сколько всего разобрано.
    pub analysed: usize,
    /// Сколько внутри участка кадров речи (без учёта их расположения).
    pub speech: usize,
}
