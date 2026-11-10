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
  <img src="https://img.shields.io/badge/platform-Windows%2011-555?style=flat-square" alt="Platform">
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/license-FSL--1.1--ALv2-555?style=flat-square" alt="License"></a>
</p>

Rewind always keeps the last 15–60 seconds of your screen in a buffer. Press a hotkey and those seconds are saved as a clip, with a sound to confirm. It runs on Windows 11. Mac and Linux are coming.

## Download
Get `Installer.exe` from the [latest release](https://github.com/tnrjns/Rewind/releases/latest) and run it. It sets up FFmpeg for you.

## Run from source
1. Unzip the `Rewind` folder somewhere permanent, like `Documents\Rewind`.
2. Double-click **install.bat**. It installs the Python packages, downloads FFmpeg, adds Desktop and Start-menu shortcuts, then opens Rewind.
   - If Python isn't installed, the script installs it and asks you to run `install.bat` again.
3. After that, open Rewind from the Desktop shortcut or the Start menu. While it's running in the tray, press **Alt + 0** from anywhere to bring the window up. Turn on *Launch when Windows starts* (General page) so Alt + 0 always works.
   - Shortcut missing? Double-click **Create Shortcut.bat**, or click *Create shortcut* on Rewind's General page.

## Default hotkeys
| Action | Hotkey |
|---|---|
| Turn Instant Replay on/off | **Alt + F9** |
| Save clip | **Alt + F10** |
| Start/stop screen recording | **Alt + F11** |
| Take screenshot | **Alt + F12** |
| Open Rewind | **Alt + 0** |

You can change all five on the **Hotkeys** page. Click a box, press the new combo, and it takes effect right away. If NVIDIA's overlay is still running, it probably owns Alt+F10 already. Either turn off NVIDIA Instant Replay or pick a different combo. Rewind tells you when a combo is taken.

## Features
- **Clip length:** 15–60 seconds, in 5-second steps (slider or quick chips)
- **Bitrate:** 5–150 Mbps (slider or presets), with an estimate of the file size
- **Encoder:** NVIDIA NVENC, AMD AMF or Intel Quick Sync, detected automatically. If none works, it uses the CPU (x264).
- **Codec:** H.264 or HEVC. Frame rate: 30, 60, 120 or 144. Resolution: native, 1440p, 1080p or 720p. Pick a monitor if you have more than one.
- **Audio:** desktop audio, plus your microphone if you want it, with a volume control for each
- **Full recordings:** open the **Recording** page for a live preview of the selected monitor and start/stop controls, or press **Alt + F11**. Rewind saves an MP4 with desktop audio and optional microphone audio, then resumes Instant Replay if it was on before. The preview is a desktop thumbnail and may not show exclusive-fullscreen content.
- **Clip sound:** chime, pop, blip or your own .wav, with adjustable volume
- **Screenshots:** press Alt+F12, or use the button on Home or in the tray menu. They're saved as PNG or JPG in the same folder as your clips, with a shutter sound and a neutral "Screenshot" badge. They work over games because they use the same capture method as the recorder.
- **Status icon:** a small pill tucked into the bottom-right corner by default. It says whether Rewind is ON or OFF. It changes to a green "Clip saved" when a clip lands. Clicks pass through it, it never takes focus, and it's hidden from screen capture, so it never shows up in your clips.
- **Clips page:** thumbnails of your clips and screenshots, with filters for All, Clips and Screenshots. Double-click one to play it. Right-click to copy it (you can paste the file straight into Discord), rename it or delete it.
- **Tray icon:** closing the window keeps Rewind running in the background. It can also start with Windows.

Clips and screenshots are saved to `Videos\Rewind`. You can change the folder on the Replay page.

## Make a standalone .exe (optional)
Run `build_exe.bat`. It creates `dist\Rewind\Rewind.exe`, which runs without Python. Keep everything in that folder together.


## Build `Installer.exe`
Run `build_installer.bat` on a Windows build machine with Python 3.10+ and Inno Setup 6 installed. It creates `dist\Installer.exe`. The installer installs Rewind per-user, adds Start Menu and optional desktop shortcuts, and downloads FFmpeg from Gyan if it is not already installed. Internet access is required when downloading FFmpeg.

## Troubleshooting
- **"FFmpeg wasn't found":** run `install.bat` again, or put `ffmpeg.exe` in `Rewind\ffmpeg\bin\`.
- **Recording fails or looks choppy:** on the Replay page, change **Encoder** from Automatic to another option. The data folder (General → Open data folder) has `ffmpeg.log` with details.
- **You can't see the status icon in a game:** games in exclusive fullscreen draw over everything. Switch the game to *Borderless* or *Windowed fullscreen*.
- **Audio is slightly out of sync:** nudge **Audio sync offset** on the Replay page. Positive values fix sound that comes too early.

## License
Rewind is licensed under the [Functional Source License, Version 1.1, ALv2 Future License](LICENSE.md). Copyright 2026 Tanner Jensen and Colin Jaxon.
