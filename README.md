<p align="center">
  <img src="assets/logo.png" width="112" alt="Rewind">
</p>

<h1 align="center">Rewind</h1>

<p align="center">
  Instant replay for your screen.<br>
  <a href="https://rewindtake.com">rewindtake.com</a> · <a href="https://github.com/tnrjns/Rewind/releases/latest">Download</a>
</p>

<p align="center">
  <a href="https://github.com/tnrjns/Rewind/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/tnrjns/Rewind/ci.yml?branch=main&label=CI&style=flat-square" alt="CI"></a>
  <a href="https://github.com/tnrjns/Rewind/releases/latest"><img src="https://img.shields.io/github/v/release/tnrjns/Rewind?style=flat-square&color=555" alt="Latest release"></a>
  <a href="https://github.com/tnrjns/Rewind/releases"><img src="https://img.shields.io/github/downloads/tnrjns/Rewind/total?style=flat-square&color=555" alt="Downloads"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2011%20%7C%20macOS%2013%2B-555?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/core-Rust%20in%20progress-B7410E?style=flat-square" alt="Core: Rust in progress">
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/license-FSL--1.1--ALv2-555?style=flat-square" alt="License"></a>
</p>

Rewind always keeps the last 15–60 seconds of your screen in a buffer. Press a hotkey and those seconds are saved as a clip, with a sound to confirm. It runs on Windows 11 and macOS 13 or newer on Apple Silicon. Linux is coming.

> **Rewind is being rewritten in Rust.** The capture engine and desktop shell are being ported crate by crate for a smaller, faster app with no Python runtime or Qt stack. The releases below are still the Python build; see [Rewriting the core in Rust](#rewriting-the-core-in-rust) for what is done and what is not.

## How it's different from OBS
OBS Studio is built for streaming and long recordings: scenes, sources, filters and plugins. Rewind does one thing. It keeps the last 15–60 seconds of your screen ready, and one key turns them into a clip.

- **Nothing to set up.** Open it and Instant Replay is already running. There are no scenes or sources to build.
- **Always on, barely there.** It lives in the tray (Windows) or menu bar (Mac). A small status pill shows that it's running, and it never shows up in your clips.
- **One key per job.** Save the replay, start or stop a recording, or take a screenshot, each with its own hotkey that works even in a game.
- **Lighter.** Measured on the same Mac with the same settings:

| | Rewind 1.1.0 | OBS Studio 33.0.1 |
|---|---|---|
| CPU | 14% of one core | 19% of one core |
| Memory | 308 MB | 428 MB |
| App size | 74 MB | 487 MB |

Both kept a 30 second replay buffer of a 1080p, 60 fps screen with Apple's hardware H.264 encoder at 6 Mbps, on an Apple M6 Mac running macOS 27. Rewind recorded desktop audio; OBS used a single screen capture source. CPU and memory are averaged over 30 seconds and count every process each app runs.

These are **Rewind against OBS**, both in Python-era builds — not Python against Rust.
The Rust port's figures will be published when it ships (see [Rewriting the core in
Rust](#rewriting-the-core-in-rust)).

If you stream, or need scenes and overlays, use OBS. If you just want to catch the moment that already happened, use Rewind.

## Download
- **Windows 11:** get `Installer.exe` from the [latest release](https://github.com/tnrjns/Rewind/releases/latest) and run it. It sets up FFmpeg for you.
- **macOS 13+ (Apple Silicon):** get `Rewind.dmg` from the [latest release](https://github.com/tnrjns/Rewind/releases/latest), open it and drag Rewind into Applications. The app isn't signed by Apple yet, so the first time, run this once in Terminal:
  ```
  xattr -dr com.apple.quarantine /Applications/Rewind.app
  ```
  When Rewind opens, macOS asks for Screen Recording permission. Turn Rewind on in System Settings → Privacy & Security → Screen & System Audio Recording, then reopen it. Rewind downloads FFmpeg the first time Instant Replay turns on.

## Default hotkeys
| Action | Windows | macOS |
|---|---|---|
| Turn Instant Replay on/off | <kbd>Alt</kbd> + <kbd>F9</kbd> | <kbd>⌃</kbd><kbd>⌥</kbd><kbd>I</kbd> |
| Save clip | <kbd>Alt</kbd> + <kbd>F10</kbd> | <kbd>⌃</kbd><kbd>⌥</kbd><kbd>S</kbd> |
| Start/stop screen recording | <kbd>Alt</kbd> + <kbd>F11</kbd> | <kbd>⌃</kbd><kbd>⌥</kbd><kbd>R</kbd> |
| Take screenshot | <kbd>Alt</kbd> + <kbd>F12</kbd> | <kbd>⌃</kbd><kbd>⌥</kbd><kbd>P</kbd> |
| Open Rewind | <kbd>Alt</kbd> + <kbd>0</kbd> | <kbd>⌃</kbd><kbd>⌥</kbd><kbd>0</kbd> |

You can change all five on the **Hotkeys** page. Click a box, press the new combo, and it takes effect right away. **Reset to defaults** puts all five back. Rewind tells you when a combo is taken. On Windows, NVIDIA's overlay often owns Alt+F10 already, so turn off NVIDIA Instant Replay or pick a different combo. On Mac, a combo needs ⌃ or ⌘, because macOS blocks ones that only use ⌥ and ⇧.

## Run from source (Windows)
1. Unzip the `Rewind` folder somewhere permanent, like `Documents\Rewind`.
2. Double-click **install.bat**. It installs the Python packages, downloads FFmpeg, adds Desktop and Start-menu shortcuts, then opens Rewind.
   - If Python isn't installed, the script installs it and asks you to run `install.bat` again.
3. After that, open Rewind from the Desktop shortcut or the Start menu. While it's running in the tray, press **Alt + 0** from anywhere to bring the window up. Turn on *Launch when Windows starts* (General page) so Alt + 0 always works.
   - Shortcut missing? Double-click **Create Shortcut.bat**, or click *Create shortcut* on Rewind's General page.

## Run from source (macOS)
```
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
mkdir -p bin && swiftc -O macos/rewind-audio.swift -o bin/rewind-audio
.venv/bin/python main.py
```
`bin/rewind-audio` is a small ScreenCaptureKit helper that records the Mac's own sound. Build `Rewind.dmg` with `./build_macos.sh`.

Both of these build the **Python** app, which is what the releases contain. To run the
Rust capture engine and shell instead, see [Rewriting the core in
Rust](#rewriting-the-core-in-rust).

## Features
- **Clip length:** 15–60 seconds, in 5-second steps (slider or quick chips)
- **Bitrate:** 5–150 Mbps (slider or presets), with an estimate of the file size
- **Encoder:** the graphics chip does the work, found on its own: NVIDIA NVENC, AMD AMF or Intel Quick Sync on Windows, Apple VideoToolbox on Mac. If none works, it uses the CPU (x264).
- **Codec:** H.264 or HEVC. Frame rate: 30, 60, 120 or 144. Resolution: native, 1440p, 1080p or 720p. Pick a monitor if you have more than one.
- **Audio:** the sound your computer plays, plus your microphone if you want it, with a volume control for each
- **Full recordings:** open the **Recording** page for a live preview of the selected monitor and start/stop controls, or press the record hotkey. Rewind saves an MP4 with desktop audio and optional microphone audio, then resumes Instant Replay if it was on before. The preview is a desktop thumbnail and may not show exclusive-fullscreen content.
- **Trim:** right-click a clip on the Clips page and pick Trim. The cut is saved as a new file and the original stays.
- **Copy after save:** turn on "Copy each clip when it saves" on the Replay page and every new clip lands on your clipboard, ready to paste into Discord.
- **Updates:** Rewind checks for a new version when it opens. From 1.3.0 on, General → Update installs it and reopens Rewind.
- **Clip sound:** chime, pop, blip or your own .wav, with adjustable volume
- **Screenshots:** press the screenshot hotkey, or use the button on Home or in the tray / menu bar menu. They're saved as PNG or JPG in the same folder as your clips, with a shutter sound and a neutral "Screenshot" badge. They work over games because they use the same capture method as the recorder.
- **Status icon:** a small pill tucked into the bottom-right corner by default. It says whether Rewind is ON or OFF. It changes to a green "Clip saved" when a clip lands. Clicks pass through it, it never takes focus, and it's hidden from screen capture, so it never shows up in your clips.
- **Clips page:** thumbnails of your clips and screenshots, with filters for All, Clips and Screenshots. Double-click one to play it. Right-click to copy it (you can paste the file straight into Discord), rename it or delete it.
- **Tray / menu bar icon:** closing the window keeps Rewind running in the background. It can also start when you log in.

Clips and screenshots are saved to `Videos\Rewind` on Windows and `Movies/Rewind` on macOS. You can change the folder on the Replay page.

## Rewriting the core in Rust

Rewind is moving from Python to Rust. The capture engine and the desktop shell are
being ported crate by crate, and the two halves will ship together once the shell
reaches feature parity. Until then, the Python app is what the releases contain.

### Why

The recorder spends its life in a loop that must never miss a frame: read a capture
buffer, hand it to the encoder, and keep one second ahead of a hotkey. In Python that
loop runs alongside a GIL, a Qt event loop, and `subprocess` pipes to FFmpeg, with audio
arriving on library callback threads that have to be handed back to Python. Changing a
setting restarts that whole arrangement. It works, and it ships — but it carries a
Python runtime and a Qt stack to do it.

Rust changes the shape of the problem rather than the encoder. FFmpeg still does the
capture and the encoding — the same NVENC / AMF / Quick Sync / VideoToolbox calls, the
same bitrates — so capture cost is unchanged. What changes is everything around it:

- **No interpreter, no GIL.** The capture loop is a thread with no lock on the hot path.
- **A single small binary** instead of a Python runtime, a Qt stack and a frozen
  interpreter. The app no longer ships Qt at all.
- **Threads instead of callbacks into an interpreter.** Audio arrives on CoreAudio or
  WASAPI callbacks and goes straight into a ring buffer with no interpreter in the way.
- **Capture and audio start concurrently**, so a recording begins in about two seconds
  instead of waiting for the two to open in turn.
- **Saving a clip is still a stream copy.** It was a stream copy before too; what
  changed is that the duration, the audio alignment and the fallback paths are now
  covered by tests that run against real FFmpeg.

### What that does and does not mean

Honest limits, because they matter more than a slogan:

- **Capture performance is FFmpeg's, not the language's.** If the encoder is the
  bottleneck, the language does not help.
- **No measured before/after numbers yet.** The CPU, memory and size figures above are
  Rewind 1.1.0 against OBS, not Python against Rust. Publishing a Python-vs-Rust table
  before the shell is finished would be measuring a half-built app, so there isn't one.
  It will be published with the release that switches the default.
- **Behaviour is being kept identical**, including the awkward parts: macOS refusing
  Option/Shift-only hotkeys, loopback devices delivering nothing while the desktop is
  quiet, clips cutting on one-second keyframes. `settings.json`, `clips.json` and the
  clips folder are byte-compatible, so upgrading keeps your settings and your library.

### Where it lives

| Path | What it is |
|---|---|
| `rust/rewind-engine` | Capture: replay ring buffer, clips, recordings, screenshots, trim |
| `rust/rewind-app` | Shell: global hotkeys, tray, status pill, and the `rewind` binary |
| `app/` | The shipping Python app, unchanged until cutover |

Both crates are real: `cargo test --workspace` runs 135 tests on macOS and 136 on
Windows, and CI builds and tests them on both alongside the Python suite.
`rust/rewind-engine/README.md` covers the design and the platform details; each crate
documents the behaviours that were subtle enough to get wrong once.

### Checking it yourself

```bash
cd rust
cargo test --workspace          # 135 tests (136 on Windows), no display needed
cargo run -p rewind-engine --release --example capture_diagnostic
```

The diagnostic tries every capture path on your machine and prints FFmpeg's own reason
for any that fail. If you find a machine where the Rust build misbehaves, that output is
the most useful thing you can send.


## Make a standalone .exe (optional)
Run `build_exe.bat`. It creates `dist\Rewind\Rewind.exe`, which runs without Python. Keep everything in that folder together.

## Build `Installer.exe`
Run `build_installer.bat` on a Windows build machine with Python 3.10+ and Inno Setup 6 installed. It creates `dist\Installer.exe`. The installer installs Rewind per-user, adds Start Menu and optional desktop shortcuts, and downloads FFmpeg from Gyan if it is not already installed. Internet access is required when downloading FFmpeg.

## Troubleshooting
- **"FFmpeg wasn't found" (Windows):** run `install.bat` again, or put `ffmpeg.exe` in `Rewind\ffmpeg\bin\`.
- **Rewind can't record on a Mac:** turn Rewind on in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen it.
- **Recording fails or looks choppy:** on the Replay page, change **Encoder** from Automatic to another option. The data folder (General → Open data folder) has `ffmpeg.log` with details.
- **You can't see the status icon in a game:** games in exclusive fullscreen draw over everything. Switch the game to *Borderless* or *Windowed fullscreen*.
- **Audio is slightly out of sync:** nudge **Audio sync offset** on the Replay page. Positive values fix sound that comes too early.

## License
Rewind is licensed under the [Functional Source License, Version 1.1, ALv2 Future License](LICENSE.md). Copyright 2026 Tanner Jensen and Colin Jaxon.
