//! Каталог бесплатных моделей Whisper (ggml) + пути и состояние загрузок.
//!
//! Все модели — открытые (OpenAI Whisper, лицензия MIT), скачиваются с Hugging Face
//! без ключа и работают полностью офлайн. Проценты точности — ориентировочные,
//! для сравнения моделей между собой (не строгий бенчмарк).

pub mod download;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Базовый URL ggml-моделей whisper.cpp на Hugging Face.
const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// Описание модели для вкладки «Модели».
pub struct ModelInfo {
    /// Короткий идентификатор.
    pub id: &'static str,
    /// Имя файла (оно же в URL).
    pub file: &'static str,
    /// Размер, МБ (приблизительно).
    pub size_mb: u32,
    /// Ярлык скорости на CPU.
    pub speed: &'static str,
    /// Ориентировочная точность для русского, %.
    pub accuracy: u8,
    /// Человекочитаемое описание/рекомендация.
    pub desc: &'static str,
}

impl ModelInfo {
    pub fn url(&self) -> String {
        format!("{HF_BASE}{}", self.file)
    }
}

/// Список предлагаемых моделей (от лёгких к тяжёлым).
pub const CATALOG: &[ModelInfo] = &[
    ModelInfo {
        id: "tiny",
        file: "ggml-tiny.bin",
        size_mb: 75,
        speed: "очень быстро",
        accuracy: 70,
        desc: "Для слабых ПК и проб. Годится для коротких простых фраз.",
    },
    ModelInfo {
        id: "base",
        file: "ggml-base.bin",
        size_mb: 148,
        speed: "быстро",
        accuracy: 78,
        desc: "Хороший баланс для старта: шустро и приемлемо по качеству.",
    },
    ModelInfo {
        id: "small",
        file: "ggml-small.bin",
        size_mb: 488,
        speed: "средне",
        accuracy: 87,
        desc: "Заметно точнее base. Рекомендуется для повседневной диктовки.",
    },
    ModelInfo {
        id: "medium",
        file: "ggml-medium.bin",
        size_mb: 1536,
        speed: "медленно",
        accuracy: 93,
        desc: "Отличное качество, но тяжёлая для CPU.",
    },
    ModelInfo {
        id: "large-v3-turbo",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size_mb: 574,
        speed: "быстро (turbo)",
        accuracy: 95,
        desc: "Лучшее качество/скорость (квантизованная). Рекомендуется, если хватает места.",
    },
];

pub fn by_id(id: &str) -> Option<&'static ModelInfo> {
    CATALOG.iter().find(|m| m.id == id)
}

/// Каталог приложения (рядом с .exe), где лежат models/ и bin/.
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn models_dir() -> PathBuf {
    app_dir().join("models")
}

pub fn bin_dir() -> PathBuf {
    app_dir().join("bin")
}

pub fn model_path(file: &str) -> PathBuf {
    models_dir().join(file)
}

pub fn is_downloaded(file: &str) -> bool {
    model_path(file).is_file()
}

/// Найти исполняемый whisper.cpp в bin/ (новое имя whisper-cli.exe или старое main.exe).
pub fn whisper_exe() -> Option<PathBuf> {
    for name in ["whisper-cli.exe", "main.exe", "whisper.exe"] {
        let p = bin_dir().join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Тип установленного движка.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Cpu,
    Gpu,
}

/// Есть ли сохранённый архив движка (для переключения без повторной загрузки).
pub fn engine_zip_cached(gpu: bool) -> bool {
    bin_dir()
        .join(if gpu { "_whisper_gpu.zip" } else { "_whisper_cpu.zip" })
        .is_file()
}

/// Какой движок сейчас активен в bin/.
/// GPU — ТОЛЬКО если есть сам CUDA-бэкенд `ggml-cuda*.dll` (рантайм cudart/cublas без него
/// бесполезен — whisper всё равно посчитает на CPU).
pub fn active_engine() -> Option<Engine> {
    whisper_exe()?;
    Some(if has_cuda_backend() {
        Engine::Gpu
    } else {
        Engine::Cpu
    })
}

/// Есть ли в bin/ загружаемый CUDA-бэкенд whisper (`ggml-cuda*.dll`).
pub fn has_cuda_backend() -> bool {
    std::fs::read_dir(bin_dir())
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.starts_with("ggml-cuda") && n.ends_with(".dll")
            })
        })
        .unwrap_or(false)
}

/// Прогресс/состояние текущей загрузки (шарится с фоновым потоком).
#[derive(Default, Clone)]
pub struct DownloadState {
    /// Что качается сейчас (id модели или "whisper.exe"); None — простаиваем.
    pub active: Option<String>,
    pub downloaded: u64,
    pub total: u64,
    pub message: String,
    pub error: Option<String>,
}

impl DownloadState {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.downloaded as f64 / self.total as f64) as f32
        }
    }
    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }
}

pub type SharedDownload = Arc<Mutex<DownloadState>>;

pub fn new_shared() -> SharedDownload {
    Arc::new(Mutex::new(DownloadState::default()))
}
