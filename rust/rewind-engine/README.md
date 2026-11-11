# Rewind engine (Rust)

The headless half of Rewind: instant replay, recordings, screenshots and trim, with
no window, no tray and no toolkit. It is a straight port of `app/recorder.py`,
`app/audio.py`, `app/trim.py` and the OS helpers those touched, and it keeps FFmpeg
as the capture and encode backend.

```
rewind-engine/
├── src/
│   ├── recorder.rs   capture lifecycle, segment ring, clip + recording muxing
│   ├── audio.rs      PCM ring buffer, loopback/mic capture, recording spool
│   ├── ffmpeg.rs     discovery, download, process plumbing, encoder ladder
│   ├── screenshot.rs one-frame grabs
│   ├── trim.rs       stream-copy cuts
│   ├── platform.rs   monitors + screen-capture permission
│   ├── settings.rs   settings.json, compatible with the Python app
│   ├── meta.rs       clips.json + thumbnails
│   ├── paths.rs      data folders, file naming
│   └── wav.rs        44-byte PCM WAV writer
├── tests/capture.rs  end-to-end capture tests (synthetic source)
└── examples/end_to_end.rs
```

## How it captures

FFmpeg records the screen straight into the GPU encoder and writes a ring of
one-second MPEG-TS segments (`buffer/seg%03d.ts`, wrapped at 75 files). Saving a clip
concatenates the newest segments with `-c copy` — no re-encode — and muxes in the
last N seconds of audio held in RAM. Full recordings write the same stream to a
temporary Matroska file and spool audio to disk as raw s16le, muxing to MP4 on stop.

`SEGMENT_WRAP` (75) must stay above the 60 s maximum clip plus headroom; below that a
long clip silently loses its oldest second.

A capture that dies is restarted up to three times a minute; a fourth death stops
replay and reports it rather than spinning.

## Threading

One mutex-guarded `Core`, one worker thread, one watchdog, one thread per clip save.

* Public methods only enqueue commands and return immediately.
* Only the worker mutates state; it never holds the lock across an FFmpeg call, so
  readers never block on a twelve-second capture start.
* Results come back as `Event`s — `try_recv()` for UI event loops, `recv()` for
  blocking consumers.

## Audio

`cpal` covers both platforms: WASAPI loopback on Windows and a CoreAudio process tap
of the default output device on macOS. That replaces the Python app's
`PyAudioWPatch` + bundled `rewind-audio` Swift helper with one code path.

Loopback devices deliver nothing at all while the desktop is silent, so every chunk
is timestamped on a fixed 48 kHz audio clock and the gaps are filled with silence.
Without that, audio drifts out of sync after the first quiet moment.

Capture starts as soon as FFmpeg opens the source, which is before the engine reports
`On`, so a recording begins a second or two before the keypress — enough to catch the
moment without the user anticipating it. Audio is opened concurrently with the video
launch and shifted by its measured offset at mux time (`-itsoffset`), and the mux never
passes `-shortest`, which would otherwise trim the video's tail to match the shorter
audio.

A loopback device produces no frames at all while nothing is playing. That is not an
error: the track is simply silent, its spool file stays empty, and the mux drops it.
When testing audio by hand, play something first.

## Settings compatibility

`settings.json`, `clips.json` and the data folders are byte-compatible with the
Python build, so an existing install keeps its settings, metadata and clips. Keys
this crate does not model (overlay colours, hotkey text, tutorial flags) are preserved
on save rather than stripped.

## Verify

```
cargo test
cargo run --example end_to_end      # needs ffmpeg on PATH; writes to a temp dir
cargo check --target x86_64-pc-windows-msvc
```

Tests use the synthetic source (`Recorder::with_options(settings, true)`), so they
need no screen permission and no real display. The real AVFoundation and WASAPI
paths are exercised by running the app.

`Recorder::with_data_dir(settings, test_source, root)` gives a recorder its own
buffers, spools, log and pid file. Tests use it because they run in parallel, and two
recorders sharing a buffer folder clear each other's segments; the app itself wants
the default single-profile folder.

## Worth verifying on Windows

`list_monitors` sorts displays by device name (`\\.\DISPLAY1`, `DISPLAY2`, …), the same
order the Python app used, while macOS puts the primary display first. The capture
command indexes that list (`ddagrab=output_idx=N`), so on a multi-monitor Windows
machine the two orderings have to agree for `monitor: 0` to mean the screen the user
picked. They agree in the usual single-primary layout; they have not been checked
against a machine with displays arranged unusually. Pick the monitor explicitly in
settings when in doubt.

`init_process()` declares per-monitor DPI awareness on Windows. Without it
`GetMonitorInfoW` virtualises rectangles for a DPI-unaware process — a 1920×1080 panel
scaled to 150% reports 1280×720 — and the `gdigrab` fallback would take its
`-video_size` and offsets from those smaller numbers, capturing a cropped corner.
Windows freezes the policy when a thread creates its first window, so the app binary
calls `init_process` before the tray icon exists; `list_monitors` calls it too, as a
best effort for callers that forget. The Python app never had to say this out loud
because Qt declared awareness at startup.

## Not ported

The window and Clips page, sounds, updates and single-instance plumbing stay in `app/`
for now. This crate hands them the same primitives the Python controller used:
`Settings`, `Recorder` + `Event`, `take_screenshot`, `trim_clip`, `load_meta`,
`list_monitors`, `list_microphones`.

## Before pushing

CI enforces these, and a formatting slip fails the build before any test runs:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run `cargo fmt --all` before committing. The workspace has two crates (`rewind-engine`
and `rewind-app`), so `--all` and `--workspace` matter — a bare `cargo fmt` only checks
the crate you happen to be in.
