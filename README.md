<div align="center">

<img src="assets/tvoice.ico" width="88" alt="TVOICE">

# TVOICE

**Offline voice dictation for Windows 11.**
Press a key, speak, and the text lands in whatever window you were typing in.
Nothing leaves your machine.

### [![Download for Windows](https://img.shields.io/github/v/release/DjonBeyron/Tvoice?label=%E2%AC%87%20DOWNLOAD%20FOR%20WINDOWS&labelColor=2ea043&color=1f6feb&style=for-the-badge&logo=windows&logoColor=white)](https://github.com/DjonBeyron/Tvoice/releases/latest)

Free · offline · no account — installer, no administrator rights needed

[![CI](https://github.com/DjonBeyron/Tvoice/actions/workflows/ci.yml/badge.svg)](https://github.com/DjonBeyron/Tvoice/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Interface: RU / EN](https://img.shields.io/badge/interface-RU%20%2F%20EN-7c5cff)](#usage)

**[Русская версия](README.ru.md)**

<img src="docs/screenshot.png" width="620" alt="TVOICE main screen">

</div>

---

## Download

### **[⬇ Latest installer for Windows](https://github.com/DjonBeyron/Tvoice/releases/latest)**

The version badge above always shows what is currently published. Take
`TVOICE-<version>-setup.exe` from the release page and run it — **no administrator rights
needed**, it installs into your user profile.

> Recognition needs an engine and a language model, which are **not bundled** (hundreds of
> megabytes). The app downloads them for you — see [First run](#first-run).

## What it does

- **Streaming dictation.** Words show up while you are still talking and get corrected in
  place, instead of appearing after the phrase is over.
- **Fully offline.** Recognition runs locally through
  [whisper.cpp](https://github.com/ggerganov/whisper.cpp). No audio and no text leaves the
  machine, no account, no API key.
- **Types into any window.** Text goes wherever the caret is — editor, browser, messenger,
  terminal.
- **Any hotkey, mouse buttons included.** A middle mouse button works as well as a key
  combination.
- **Lives in the tray**, and can start with Windows, minimised.
- **Stops by itself** after ten seconds of silence.

## Requirements

| | |
|---|---|
| OS | Windows 10 1809+ or Windows 11, 64-bit |
| Microphone | any input device; permission is requested on first run |
| Disk | ~10 MB for the app, plus ~700 MB for the engine and 75 MB – 1.5 GB for the model |
| GPU | optional — whisper.cpp uses CUDA when an NVIDIA card is present, CPU otherwise |

The app uses Media Foundation, present in every regular Windows edition. On `N`/`KN`
editions install the Media Feature Pack first.

## Installation

1. Download `TVOICE-<version>-setup.exe` from the
   [latest release](https://github.com/DjonBeyron/Tvoice/releases/latest).
2. Run it. The installer offers a Start Menu shortcut, an optional desktop shortcut and an
   optional *start with Windows* entry.
3. It installs into `%LOCALAPPDATA%\Programs\TVOICE`.

That location is deliberate, not laziness: the app keeps its settings, log, engine and
models next to its own executable, and `Program Files` is not writable for a normal user —
installing there would break both settings and model downloads.

Uninstall from **Settings → Apps** as usual. Downloaded models are **kept**, so a
reinstall does not have to fetch gigabytes again; delete the install folder by hand if you
want them gone too.

## First run

1. Open **Settings → Engine and models**.
2. Press **Download engine** — fetches a whisper.cpp build, with CUDA support if your
   machine has an NVIDIA GPU.
3. Pick a model and download it:

   | Model | Size | Good for |
   |---|---|---|
   | `tiny` | 75 MB | quick checks, weak machines |
   | `base` | 148 MB | short notes |
   | `small` | 488 MB | balanced |
   | **`large-v3-turbo`** | 574 MB | **recommended** — near-large quality, still fast |
   | `medium` | 1.5 GB | maximum quality, slower |

4. Back on the **Dictation** screen, check the hotkey and press **Start dictation**.

## Usage

**Hotkey.** `Ctrl + Alt + Space` by default. Change it in **Settings → Input**: press
*Set*, then hold the combination you want. Mouse buttons are accepted.

**Streaming mode** (the default) makes the hotkey a **toggle** — press to start, press
again to stop. Hold-to-talk is not used here on purpose: the held keys would leak their
characters into the target window.

**Interface language.** Russian or English, switched in **Settings → Microphone and system
→ Interface language** and applied immediately, without a restart. The installer asks for a
language at the start and the program opens in it; if no choice was ever made, it follows the
Windows display language. This is separate from the *recognition* language on the Dictation
screen — the language you speak.

**Indicator.** A small pill near the caret pulses with your voice. Position and size are
configurable.

**Auto-stop.** Ten seconds of silence closes the capture and hides the indicator, so a
forgotten hotkey does not keep the microphone open.

**Sounds.** `rec.mp3` next to the executable plays when capture starts; the same sound,
reversed, plays when it stops. Delete the file for silent operation, or replace it with
your own — the reversed version is rebuilt automatically.

## Diagnostics

The binary doubles as its own test bench. All of these run headless, without the GUI:

```bash
tvoice --probe                              # microphone permissions and device list
tvoice --vad-file <file.wav>                # voice-activity detection over a file
tvoice --idle-test [file.wav]               # verify the 10-second auto-stop
tvoice --sound-test [n]                     # start/stop cues and their latency
tvoice --stt-bench <wav> <reference text>   # recognition quality, by the numbers
tvoice --rewrite-sim <draft> <draft> …      # how much text a refinement rewrites
tvoice --autostart status|on|off            # the "start with Windows" registry entry
```

Everything is logged to `tvoice.log` next to the executable: voice-activity thresholds,
every recognition draft, every insertion. When dictation misbehaves, that file usually
says why.

## How it works

| Stage | Where | Notes |
|---|---|---|
| Microphone | [`src/mic/`](src/mic) | three layers: privacy toggles in the registry, WinRT `MediaCapture` for the consent dialog, and raw WASAPI as a fallback — some drivers reject shared-mode capture with `E_INVALIDARG`, so exclusive mode is attempted next |
| Speech detection | [`src/vad.rs`](src/vad.rs) | noise floor as a low percentile of a 5-second energy window, computed independently of the speech/silence decision so it cannot latch |
| Recognition | [`src/server.rs`](src/server.rs) | a resident `whisper-server`: the model is loaded once and stays in memory, requests go over HTTP. Bound to a Job Object so it dies with the app, even on a crash |
| Dictation loop | [`src/streaming.rs`](src/streaming.rs) | the current phrase is a draft, re-recognised every ~400 ms and corrected in place with the minimum possible edit |
| Insertion | [`src/inject.rs`](src/inject.rs) | clipboard + `Ctrl+V` by default (atomic), per-character Unicode input as a fallback for terminals |
| Indicator | [`src/overlay.rs`](src/overlay.rs), [`src/hud.rs`](src/hud.rs) | a native layered Win32 window in its own thread — click-through, never takes focus |

Comments in the source explain **why** something is done a particular way; the awkward
decisions usually encode a measurement or a Windows quirk. Please keep that style.

## Building from source

The toolchain is pinned in `rust-toolchain.toml` (Rust **GNU**), so `rustup` picks it up
automatically. You also need **mingw-w64** on `PATH` — the build embeds the application
icon with `windres`.

```bash
cargo build --release
cargo run -- --probe        # headless check that the microphone core works
```

If mingw-w64 is not on `PATH`, copy `.cargo/config.toml.example` to `.cargo/config.toml`
and fix the paths. If your Windows profile name contains spaces or non-ASCII characters,
move `CARGO_HOME` and `RUSTUP_HOME` to a short ASCII path first — mingw's linker cannot
handle spaces in toolchain paths.

To build the installer too (needs [Inno Setup 6](https://jrsoftware.org/isdl.php)):

```bash
powershell -ExecutionPolicy Bypass -File scripts/build-installer.ps1
```

## Releasing

The version lives in `Cargo.toml` and nowhere else. Tag it and CI does the rest — builds
the app, packages the installer, publishes a release with the artifact and checksums
attached:

```bash
git tag v1.21.1 && git push origin v1.21.1
```

The workflow refuses to publish when the tag and `Cargo.toml` disagree.

## Contributing

Issues and pull requests are welcome. Two house rules:

- bump the version in `Cargo.toml` with every change (manual SemVer);
- keep source files under roughly 400 lines — split a module rather than grow it.

The code is deliberately **not** rustfmt-formatted: the layout is hand-tuned around the
comments, so please do not reformat wholesale.

## License

[MIT](LICENSE).

Recognition is performed by [whisper.cpp](https://github.com/ggerganov/whisper.cpp) with
[OpenAI Whisper](https://github.com/openai/whisper) models, downloaded at runtime under
their own licences. The bundled fallback font is
[DejaVu Sans](https://dejavu-fonts.github.io/).
