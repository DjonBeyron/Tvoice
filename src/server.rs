//! Резидентный whisper-server: модель загружается ОДИН раз и висит в памяти,
//! распознавание идёт по HTTP — без перезагрузки модели на каждую фразу (быстро).
//!
//! Если whisper-server.exe отсутствует (старая загрузка движка) — откат на whisper-cli.

use std::net::{SocketAddr, TcpStream};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use crate::models;

/// Job Object с флагом KILL_ON_JOB_CLOSE: все назначенные процессы (whisper-server) умрут,
/// как только закроется наш процесс (в т.ч. при краше/kill), — без осиротевших серверов.
fn job_handle() -> isize {
    static JOB: OnceLock<isize> = OnceLock::new();
    *JOB.get_or_init(|| unsafe {
        match CreateJobObjectW(None, windows::core::PCWSTR::null()) {
            Ok(h) => {
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let _ = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                h.0 as isize
            }
            Err(_) => 0,
        }
    })
}

fn assign_to_job(child: &Child) {
    let j = job_handle();
    if j != 0 {
        unsafe {
            let _ = AssignProcessToJobObject(
                HANDLE(j as *mut core::ffi::c_void),
                HANDLE(child.as_raw_handle()),
            );
        }
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Server {
    child: Child,
    port: u16,
    model: String,
    lang: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

fn server_exe() -> Option<std::path::PathBuf> {
    let p = models::bin_dir().join("whisper-server.exe");
    p.is_file().then_some(p)
}

/// Распознать через резидентный сервер (или через whisper-cli, если сервера нет).
pub fn transcribe(wav16: &Path, model_file: &str, lang: &str) -> Result<String> {
    if server_exe().is_none() {
        // Откат на разовый запуск CLI.
        return crate::transcribe::transcribe(wav16, model_file, lang);
    }
    let mut guard = SERVER.lock().unwrap();
    let restart = match guard.as_ref() {
        Some(s) => s.model != model_file || s.lang != lang,
        None => true,
    };
    if restart {
        *guard = None; // сначала гасим старый (Drop убьёт процесс)
        *guard = Some(start(model_file, lang)?);
    }
    let port = guard.as_ref().unwrap().port;
    post_inference(port, wav16)
}

/// Прогреть сервер заранее (в фоне), чтобы первая диктовка тоже была быстрой.
pub fn prewarm(model_file: String, lang: String) {
    if server_exe().is_none() {
        return;
    }
    std::thread::spawn(move || {
        let mut guard = SERVER.lock().unwrap();
        let ok = guard
            .as_ref()
            .map(|s| s.model == model_file && s.lang == lang)
            .unwrap_or(false);
        if !ok {
            *guard = None;
            match start(&model_file, &lang) {
                Ok(s) => *guard = Some(s),
                Err(e) => crate::logln!("server: прогрев не удался: {e}"),
            }
        }
    });
}

/// Погасить сервер (вызывать при выходе — статик не вызывает Drop сам).
pub fn shutdown() {
    if let Ok(mut g) = SERVER.lock() {
        *g = None;
    }
}

fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// Аргументы декодирования боевого сервера. Собраны в одном месте, чтобы стенд
/// качества мог сравнивать их с другими наборами, а не догадываться.
pub fn decoding_args() -> Vec<String> {
    ["-bs", "1", "-nf"].iter().map(|s| s.to_string()).collect()
}

fn start(model_file: &str, lang: &str) -> Result<Server> {
    start_with(model_file, lang, &decoding_args())
}

/// Разовый сервер с заданными аргументами декодирования: распознать файл и погасить.
/// Возвращает текст и время самого запроса (без запуска сервера).
pub fn measure(
    model_file: &str,
    lang: &str,
    extra: &[String],
    wav: &Path,
) -> Result<(String, f32)> {
    let s = start_with(model_file, lang, extra)?;
    let t0 = Instant::now();
    let text = post_inference(s.port, wav)?;
    Ok((text, t0.elapsed().as_secs_f32()))
}

fn start_with(model_file: &str, lang: &str, extra: &[String]) -> Result<Server> {
    let exe = server_exe().ok_or_else(|| anyhow!("whisper-server.exe не найден"))?;
    let model = models::model_path(model_file);
    if !model.is_file() {
        return Err(anyhow!("модель не найдена: {model_file}"));
    }
    let port = free_port()?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .to_string();

    let mut cmd = Command::new(&exe);
    cmd.arg("-m")
        .arg(&model)
        .arg("-l")
        .arg(lang)
        .arg("-t")
        .arg(&threads)
        .args(extra)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped()); // ловим вывод, чтобы узнать бэкенд (CUDA/CPU) и тайминги

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    crate::logln!("server: запуск (model={model_file}, lang={lang}, порт={port})…");
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow!("не удалось запустить whisper-server: {e}"))?;
    assign_to_job(&child);

    // Читаем stderr сервера в фоне и логируем важное (бэкенд/устройство/тайминги).
    if let Some(err) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let l = line.to_lowercase();
                if l.contains("cuda")
                    || l.contains("gpu")
                    || l.contains("backend")
                    || l.contains("device")
                    || l.contains("blas")
                    || l.contains("total time")
                    || l.contains("encode time")
                    || l.contains("error")
                {
                    crate::logln!("whisper-server: {}", line.trim());
                }
            }
        });
    }

    // Сервер открывает порт ТОЛЬКО после загрузки модели → ждём коннекта = готовности.
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let t0 = Instant::now();
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok() {
            break;
        }
        if t0.elapsed() > Duration::from_secs(120) {
            return Err(anyhow!("whisper-server не поднялся за 120с"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    crate::logln!("server: готов за {:.1}с", t0.elapsed().as_secs_f32());

    Ok(Server {
        child,
        port,
        model: model_file.to_string(),
        lang: lang.to_string(),
    })
}

fn post_inference(port: u16, wav: &Path) -> Result<String> {
    let bytes = std::fs::read(wav)?;
    let boundary = "----tvoiceBoundary8f3ac21";
    let mut body: Vec<u8> = Vec::with_capacity(bytes.len() + 512);

    let part = |name: &str, extra: &str, data: &[u8], body: &mut Vec<u8>| {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"{extra}\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    };

    part(
        "file",
        "; filename=\"audio.wav\"\r\nContent-Type: audio/wav",
        &bytes,
        &mut body,
    );
    part("response_format", "", b"text", &mut body);
    part("temperature", "", b"0.0", &mut body);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let url = format!("http://127.0.0.1:{port}/inference");
    let resp = ureq::post(&url)
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send_bytes(&body)
        .map_err(|e| anyhow!("запрос к серверу: {e}"))?;

    let text = resp.into_string()?;
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}
