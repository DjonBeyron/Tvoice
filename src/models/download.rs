//! Фоновая загрузка моделей и бинарника whisper.cpp с индикацией прогресса.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::thread;

use super::{bin_dir, model_path, models_dir, ModelInfo, SharedDownload};

const WHISPER_RELEASES_API: &str =
    "https://api.github.com/repos/ggerganov/whisper.cpp/releases/latest";

/// Запустить загрузку модели в фоне. Вызывать только когда загрузок нет.
pub fn start_model(info: &'static ModelInfo, state: SharedDownload, ctx: egui::Context) {
    {
        let mut s = state.lock().unwrap();
        *s = super::DownloadState {
            active: Some(info.id.to_string()),
            message: format!("Загрузка {}…", info.file),
            ..Default::default()
        };
    }
    let url = info.url();
    let dest = model_path(info.file);
    thread::spawn(move || {
        let result = download_to_file(&url, &dest, &state, &ctx);
        finish(&state, &ctx, result);
    });
}

/// Запустить загрузку и распаковку бинарника whisper.cpp в bin/.
/// `gpu = true` — сборка NVIDIA CUDA (cuBLAS), иначе CPU.
pub fn start_whisper_binary(gpu: bool, state: SharedDownload, ctx: egui::Context) {
    {
        let mut s = state.lock().unwrap();
        *s = super::DownloadState {
            active: Some(if gpu { "whisper-gpu" } else { "whisper.exe" }.to_string()),
            message: "Поиск релиза whisper.cpp…".to_string(),
            ..Default::default()
        };
    }
    thread::spawn(move || {
        let result = fetch_and_extract_binary(gpu, &state, &ctx);
        finish(&state, &ctx, result);
    });
}

fn finish(state: &SharedDownload, ctx: &egui::Context, result: anyhow::Result<()>) {
    let mut s = state.lock().unwrap();
    s.active = None;
    match result {
        Ok(()) => s.message = "Готово.".to_string(),
        Err(e) => {
            s.error = Some(e.to_string());
            s.message = format!("Ошибка: {e}");
        }
    }
    ctx.request_repaint();
}

/// Загрузка URL в файл с прогрессом, **докачкой** (HTTP Range из `.part`) и
/// **автоповтором** при обрывах связи. Для больших файлов (GPU-сборка ~680 МБ) это критично.
fn download_to_file(
    url: &str,
    dest: &Path,
    state: &SharedDownload,
    ctx: &egui::Context,
) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("part");

    let agent = ureq::builder()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(45))
        .build();

    // Повторяем попытки, пока файл не докачан. Обрыв → пауза → продолжаем с места.
    let mut stalls = 0u32;
    loop {
        let before = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        let result = download_once(&agent, url, &tmp, state, ctx);
        let after = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);

        match result {
            Ok(true) => break, // докачано полностью
            Ok(false) | Err(_) => {
                // Считаем «застоем» попытку без прогресса.
                if after > before {
                    stalls = 0;
                } else {
                    stalls += 1;
                }
                if stalls >= 20 {
                    anyhow::bail!("не удалось докачать: соединение постоянно обрывается");
                }
                {
                    let mut s = state.lock().unwrap();
                    s.message = format!(
                        "Обрыв связи — докачиваю ({} из {})…",
                        human(after),
                        if s.total > 0 { human(s.total) } else { "?".into() }
                    );
                }
                ctx.request_repaint();
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    }

    fs::rename(&tmp, dest)?;
    ctx.request_repaint();
    Ok(())
}

/// Одна попытка скачивания/докачки. Возвращает `Ok(true)`, если файл полностью получен.
fn download_once(
    agent: &ureq::Agent,
    url: &str,
    tmp: &Path,
    state: &SharedDownload,
    ctx: &egui::Context,
) -> anyhow::Result<bool> {
    let existing = fs::metadata(tmp).map(|m| m.len()).unwrap_or(0);

    let mut req = agent.get(url).set("User-Agent", "tvoice");
    if existing > 0 {
        req = req.set("Range", &format!("bytes={existing}-"));
    }
    let resp = req.call().map_err(|e| anyhow::anyhow!("сеть: {e}"))?;

    let resuming = resp.status() == 206;
    let (mut file, mut done, total) = if resuming {
        let total = content_range_total(resp.header("Content-Range")).unwrap_or_else(|| {
            existing
                + resp
                    .header("Content-Length")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0)
        });
        (OpenOptions::new().append(true).open(tmp)?, existing, total)
    } else {
        // Сервер проигнорировал Range (200) — начинаем файл заново.
        let total = resp
            .header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        (File::create(tmp)?, 0u64, total)
    };

    {
        let mut s = state.lock().unwrap();
        s.total = total;
        s.downloaded = done;
    }

    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 128 * 1024];
    let mut last_repaint = std::time::Instant::now();
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(_) => {
                // Обрыв чтения — сохраняем, что успели; повтор снаружи докачает.
                let _ = file.flush();
                return Ok(false);
            }
        };
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        {
            let mut s = state.lock().unwrap();
            s.downloaded = done;
        }
        if last_repaint.elapsed().as_millis() >= 50 {
            ctx.request_repaint();
            last_repaint = std::time::Instant::now();
        }
    }
    file.flush()?;
    // Полностью, если знаем размер и достигли его (или размер неизвестен, но поток закрылся).
    Ok(total == 0 || done >= total)
}

/// Достаём общий размер из заголовка Content-Range: `bytes 100-999/1000`.
fn content_range_total(header: Option<&str>) -> Option<u64> {
    header?.rsplit('/').next()?.trim().parse().ok()
}

fn human(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    format!("{mb:.1} МБ")
}

/// Узнать URL zip-ассета CPU-сборки из последнего релиза whisper.cpp.
pub fn find_binary_zip_url() -> anyhow::Result<String> {
    let json: serde_json::Value = ureq::get(WHISPER_RELEASES_API)
        .set("User-Agent", "tvoice")
        .call()
        .map_err(|e| anyhow::anyhow!("GitHub API: {e}"))?
        .into_json()?;

    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("в релизе нет ассетов"))?;

    let named: Vec<(String, String)> = assets
        .iter()
        .filter_map(|a| {
            let name = a["name"].as_str()?.to_string();
            let url = a["browser_download_url"].as_str()?.to_string();
            Some((name, url))
        })
        .collect();

    let is_gpu = |n: &str| {
        let n = n.to_lowercase();
        ["cublas", "cuda", "clblast", "blas", "vulkan", "hip"]
            .iter()
            .any(|g| n.contains(g))
    };

    // 1) точное каноничное имя; 2) любой x64-zip без GPU; 3) любой x64-zip.
    let pick = named
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("whisper-bin-x64.zip"))
        .or_else(|| {
            named
                .iter()
                .find(|(n, _)| {
                    let l = n.to_lowercase();
                    l.contains("x64") && l.ends_with(".zip") && !is_gpu(&l)
                })
        })
        .or_else(|| {
            named
                .iter()
                .find(|(n, _)| {
                    let l = n.to_lowercase();
                    l.contains("x64") && l.ends_with(".zip")
                })
        });

    pick.map(|(_, url)| url.clone())
        .ok_or_else(|| anyhow::anyhow!("не найден Windows x64 zip в релизе"))
}

/// URL сборки NVIDIA CUDA (cuBLAS) x64 — предпочитаем свежий CUDA 12.x.
pub fn find_gpu_zip_url() -> anyhow::Result<String> {
    let json: serde_json::Value = ureq::get(WHISPER_RELEASES_API)
        .set("User-Agent", "tvoice")
        .call()
        .map_err(|e| anyhow::anyhow!("GitHub API: {e}"))?
        .into_json()?;
    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("в релизе нет ассетов"))?;
    let named: Vec<(String, String)> = assets
        .iter()
        .filter_map(|a| {
            Some((
                a["name"].as_str()?.to_string(),
                a["browser_download_url"].as_str()?.to_string(),
            ))
        })
        .collect();

    let is_cuda_x64 = |n: &str| {
        let l = n.to_lowercase();
        l.contains("cublas") && l.contains("x64") && l.ends_with(".zip")
    };
    // Предпочитаем CUDA 12.x, иначе любую cuBLAS x64.
    let pick = named
        .iter()
        .find(|(n, _)| is_cuda_x64(n) && n.contains("12."))
        .or_else(|| named.iter().find(|(n, _)| is_cuda_x64(n)));

    pick.map(|(_, u)| u.clone())
        .ok_or_else(|| anyhow::anyhow!("в релизе нет NVIDIA CUDA сборки для Windows x64"))
}

fn fetch_and_extract_binary(
    gpu: bool,
    state: &SharedDownload,
    ctx: &egui::Context,
) -> anyhow::Result<()> {
    let bin = bin_dir();
    fs::create_dir_all(&bin)?;
    // Раздельные имена для CPU и GPU — чтобы не перепутать и переиспользовать нужный.
    let zip_path = bin.join(if gpu {
        "_whisper_gpu.zip"
    } else {
        "_whisper_cpu.zip"
    });

    // Если полный архив уже скачан ранее — переиспользуем (переключение без загрузки, даже офлайн).
    let have_zip = zip_path.exists()
        && File::open(&zip_path)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
            .is_some();
    if have_zip {
        crate::logln!("движок: беру уже скачанный архив {}", zip_path.display());
        let mut s = state.lock().unwrap();
        s.message = "Использую скачанный архив…".to_string();
    } else {
        let url = if gpu {
            find_gpu_zip_url()?
        } else {
            find_binary_zip_url()?
        };
        crate::logln!("движок: скачиваю {} — {url}", if gpu { "GPU/CUDA" } else { "CPU" });
        {
            let mut s = state.lock().unwrap();
            s.message = "Загрузка whisper.cpp…".to_string();
        }
        ctx.request_repaint();
        download_to_file(&url, &zip_path, state, ctx)?;
    }
    crate::logln!(
        "движок: архив {} — распаковываю",
        human(fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0))
    );

    // ВАЖНО: гасим резидентный whisper-server, иначе его файлы (whisper-server.exe, ggml*.dll,
    // whisper.dll) заняты работающим процессом и не перезапишутся (os error 32).
    crate::server::shutdown();
    std::thread::sleep(std::time::Duration::from_millis(400));

    {
        let mut s = state.lock().unwrap();
        s.message = "Распаковка…".to_string();
    }
    ctx.request_repaint();

    let file = File::open(&zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| anyhow::anyhow!("zip: {e}"))?;
    let mut extracted = 0;
    let mut failed = 0;
    // Устойчиво: сбой одной записи не должен рушить всю распаковку (в GPU-архиве
    // есть огромные CUDA-DLL; главное — извлечь whisper-cli.exe и рабочие библиотеки).
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                crate::logln!("движок: запись {i} не читается: {e}");
                failed += 1;
                continue;
            }
        };
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if !(lower.ends_with(".exe") || lower.ends_with(".dll")) {
            continue;
        }
        let file_name = Path::new(&name)
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_else(|| name.clone().into());
        let out = bin.join(&file_name);
        let res = File::create(&out).and_then(|mut o| std::io::copy(&mut entry, &mut o).map(|_| ()));
        match res {
            Ok(()) => {
                extracted += 1;
                let fnl = file_name.to_string_lossy().to_lowercase();
                if fnl.contains("cuda") || fnl.contains("cublas") {
                    crate::logln!("движок: распакован {}", file_name.to_string_lossy());
                }
            }
            Err(e) => {
                crate::logln!("движок: не распаковалось {}: {e}", file_name.to_string_lossy());
                failed += 1;
            }
        }
    }
    let _ = fs::create_dir_all(models_dir());

    crate::logln!(
        "движок: распаковано {extracted}, ошибок {failed}; whisper_exe={:?}",
        super::whisper_exe()
    );

    // Архив НЕ удаляем — оставляем на диске, чтобы при повторной установке не качать заново.
    // (Можно удалить вручную: bin/_whisper_gpu.zip / _whisper_cpu.zip.)

    if super::whisper_exe().is_none() {
        anyhow::bail!("движок не установился (whisper-cli.exe не найден; распаковано {extracted}, ошибок {failed})");
    }
    if failed > 0 {
        anyhow::bail!("часть файлов не распаковалась ({failed}); нажмите ещё раз — архив уже сохранён, докачка не нужна");
    }
    if gpu {
        let cuda = super::has_cuda_backend();
        crate::logln!("движок GPU: CUDA-бэкенд (ggml-cuda.dll) присутствует: {cuda}");
        if !cuda {
            anyhow::bail!(
                "в GPU-сборке нет CUDA-бэкенда ggml-cuda.dll — ускорение работать не будет (распознавание останется на CPU)"
            );
        }
    }
    Ok(())
}
