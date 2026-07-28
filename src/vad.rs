//! Определение речи/тишины с адаптивным порогом.
//!
//! Фиксированный порог по амплитуде не работает: у одного микрофона фон 0.002, у другого
//! 0.02 — и во втором случае «тишина» не наступает никогда, паузы между фразами не находятся,
//! а whisper молотит шум и выдаёт галлюцинации.
//!
//! Считаем энергию (RMS) по кадрам 20мс. Уровень фона — трекер «быстро вниз, очень медленно
//! вверх»: он мгновенно опускается к тишине и почти не растёт во время речи (иначе порог
//! догоняет голос, и речь перестаёт считаться речью — фраза рубится на куски).
//! Порог включения выше порога выключения (гистерезис), плюс два предохранителя:
//! речь должна продержаться несколько кадров, чтобы начаться, и ещё немного «звенит»
//! после спада — иначе паузы между словами рвут фразу.

/// Длительность кадра.
const FRAME_MS: usize = 20;
/// Во сколько разбросов шума речь должна быть выше его среднего, чтобы начаться.
const ON_DEVS: f32 = 5.0;
/// …и чтобы продолжаться (гистерезис).
const OFF_DEVS: f32 = 2.5;
/// Абсолютные нижние границы порогов — на случай идеально тихого входа.
const MIN_ON: f32 = 0.008;
const MIN_OFF: f32 = 0.005;
/// Скорость подстройки оценки шума (≈1с при кадре 20мс).
const ADAPT: f32 = 0.02;
/// Первые кадры считаем шумом безусловно: нужно с чего-то начать оценку.
const BOOTSTRAP: usize = 25;
/// Сколько кадров подряд нужно превышать порог, чтобы это считалось началом речи (60мс).
const RISE: usize = 3;
/// Сколько кадров речь «звенит» после спада — мост через паузы между словами (300мс).
const HANG: usize = 15;

pub struct Vad {
    frame: usize,
    processed: usize,
    /// Типичный уровень шума и его разброс — оцениваются только по кадрам без речи.
    mean: f32,
    dev: f32,
    seen: usize,
    last_rms: f32,
    in_speech: bool,
    rise: usize,
    hang: usize,
    /// Речь/тишина по кадрам с начала записи.
    flags: Vec<bool>,
}

impl Vad {
    pub fn new(rate: usize) -> Self {
        Self {
            frame: (rate * FRAME_MS / 1000).max(1),
            processed: 0,
            mean: 0.0,
            dev: 0.0,
            seen: 0,
            last_rms: 0.0,
            in_speech: false,
            rise: 0,
            hang: 0,
            flags: Vec::new(),
        }
    }

    /// Разобрать новые сэмплы буфера захвата (всё, что появилось после прошлого вызова).
    pub fn feed(&mut self, buf: &[f32]) {
        while self.processed + self.frame <= buf.len() {
            let f = &buf[self.processed..self.processed + self.frame];
            self.processed += self.frame;
            let rms = (f.iter().map(|v| v * v).sum::<f32>() / f.len() as f32).sqrt();
            self.last_rms = rms;
            self.seen += 1;

            // Оценку шума обновляем только на кадрах без речи (и в самом начале, пока
            // оценки ещё нет). Иначе она «догоняет» голос, и речь перестаёт им считаться.
            if self.seen <= BOOTSTRAP || (!self.in_speech && rms < self.on()) {
                let a = if self.seen <= BOOTSTRAP { 0.2 } else { ADAPT };
                self.mean += (rms - self.mean) * a;
                self.dev += ((rms - self.mean).abs() - self.dev) * a;
            }

            let on = self.on();
            let off = (self.mean + self.dev * OFF_DEVS).max(MIN_OFF);
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
        }
    }

    /// Порог включения речи.
    pub fn on(&self) -> f32 {
        (self.mean + self.dev * ON_DEVS).max(MIN_ON)
    }

    /// Оценённый уровень шума и его разброс (для лога).
    pub fn noise(&self) -> f32 {
        self.mean
    }

    pub fn dev(&self) -> f32 {
        self.dev
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
