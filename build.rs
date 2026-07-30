//! Встраивание значка в сам `.exe`.
//!
//! Значок окна можно поставить и во время работы (`WM_SETICON`), но проводник, панель
//! задач и закреплённые ярлыки берут его из ресурса внутри файла. Без ресурса у
//! программы остаётся системный значок, как бы приложение ни настраивало окно.
//!
//! Ресурс собирает `windres` из mingw — тот же тулчейн, которым линкуется проект,
//! поэтому лишних зависимостей не появляется. Если его нет, сборка не падает:
//! программа просто останется без своего значка.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/tvoice.ico");
    println!("cargo:rerun-if-changed=assets/rec.mp3");
    println!("cargo:rerun-if-changed=build.rs");

    copy_runtime_assets();

    if !cfg!(windows) {
        return;
    }
    let ico = Path::new("assets/tvoice.ico");
    if !ico.is_file() {
        println!("cargo:warning=assets/tvoice.ico не найден — значок не встроен");
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc = out_dir.join("tvoice.rc");
    let res = out_dir.join("tvoice_icon.o");

    // Идентификатор 1: под этим номером Windows ищет значок приложения.
    let ico_path = std::fs::canonicalize(ico)
        .unwrap_or_else(|_| ico.to_path_buf())
        .display()
        .to_string()
        .replace('\\', "/")
        .replace("//?/", "");
    std::fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).expect("не записать .rc");

    let windres = which_windres();
    let status = Command::new(&windres)
        .arg("-i")
        .arg(&rc)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&res)
        .status();

    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg={}", res.display()),
        Ok(s) => println!("cargo:warning=windres завершился с кодом {s} — значок не встроен"),
        Err(e) => println!("cargo:warning=не запустить {windres}: {e} — значок не встроен"),
    }
}

/// Положить рядом с .exe файлы, которые программа читает во время работы.
///
/// Сигналы диктовки ищутся рядом с исполняемым файлом (`models::app_dir`), поэтому в
/// репозитории они лежат в `assets`, а сюда копируются при сборке. Без этого после
/// `cargo clean` или на свежей машине звуки молча исчезали бы: сама папка `target`
/// в репозиторий не входит.
fn copy_runtime_assets() {
    let Ok(out) = std::env::var("OUT_DIR") else {
        return;
    };
    // OUT_DIR — это target/<profile>/build/<crate>-<hash>/out; нужен target/<profile>.
    let Some(target_dir) = Path::new(&out).ancestors().nth(3) else {
        return;
    };
    for name in ["rec.mp3"] {
        let from = Path::new("assets").join(name);
        if !from.is_file() {
            println!("cargo:warning=нет {} — сигнал не будет играть", from.display());
            continue;
        }
        let to = target_dir.join(name);
        // Не перезаписываем файл, который пользователь заменил своим: сравниваем время.
        let newer = |a: &Path, b: &Path| match (modified(a), modified(b)) {
            (Some(x), Some(y)) => x > y,
            _ => true,
        };
        if to.exists() && !newer(&from, &to) {
            continue;
        }
        if let Err(e) = std::fs::copy(&from, &to) {
            println!("cargo:warning=не скопировать {}: {e}", from.display());
        }
    }
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Путь к `windres`: сначала рядом с линкером проекта, потом просто из PATH.
fn which_windres() -> String {
    let fixed = "C:/PITH/tools/mingw64/bin/windres.exe";
    if Path::new(fixed).is_file() {
        return fixed.to_string();
    }
    "windres".to_string()
}
