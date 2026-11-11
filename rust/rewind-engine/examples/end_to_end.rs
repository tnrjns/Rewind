//! End-to-end check of the real capture path, using the synthetic test source so it
//! runs headless in CI: start replay, buffer, save a clip, then record and stop.
//!
//! Run with `cargo run --example end_to_end`. Exits non-zero if either artifact is
//! missing, empty, or unreadable by FFmpeg.

use std::path::Path;
use std::time::{Duration, Instant};

use rewind_engine::ffmpeg::{find_ffmpeg, probe_duration, run};
use rewind_engine::{Event, Mode, Recorder, Settings, State};

fn wait_for(recorder: &Recorder, what: &str, timeout: Duration) -> Event {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(event) = recorder.try_recv() {
            match &event {
                Event::Error(msg) => eprintln!("  error event: {msg}"),
                Event::StateChanged(state) => eprintln!("  state -> {}", state.as_str()),
                other => eprintln!("  event: {other:?}"),
            }
            match &event {
                Event::ClipSaved { .. } | Event::RecordingSaved { .. } | Event::ClipFailed(_) => {
                    return event
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

fn wait_for_mode(recorder: &Recorder, mode: Mode, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if recorder.mode() == mode && recorder.state() == State::On {
            return;
        }
        while let Some(event) = recorder.try_recv() {
            if let Event::Error(msg) = event {
                eprintln!("  error event: {msg}");
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "timed out waiting for mode {} (state {})",
        mode.as_str(),
        recorder.state().as_str()
    );
}

fn wait_for_state(recorder: &Recorder, state: State, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if recorder.state() == state {
            return;
        }
        while let Some(event) = recorder.try_recv() {
            if let Event::Error(msg) = event {
                eprintln!("  error event: {msg}");
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for state {}", state.as_str());
}

/// Confirm FFmpeg can read the file and report its stream layout.
fn describe(ffmpeg: &Path, path: &Path, expect_audio: bool) {
    let out = run(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-i".into(),
            path.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(20),
        None,
    );
    let banner = String::from_utf8_lossy(&out.stderr);
    let duration = probe_duration(ffmpeg, path);
    let has_video = banner.contains("Video: h264") || banner.contains("Video: hevc");
    let has_audio = banner.contains("Audio: aac");
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!(
        "  {} — {:.2} s, {} bytes, video={}, audio={}",
        path.file_name().unwrap().to_string_lossy(),
        duration.unwrap_or(-1.0),
        size,
        has_video,
        has_audio
    );
    assert!(size > 10_000, "{} is suspiciously small", path.display());
    assert!(has_video, "no video stream in {}", path.display());
    assert_eq!(
        has_audio,
        expect_audio,
        "audio stream mismatch in {}",
        path.display()
    );
    let duration = duration.unwrap_or(0.0);
    assert!(duration > 0.5, "{} reports {duration} s", path.display());
}

fn main() {
    let ffmpeg = find_ffmpeg().expect("ffmpeg must be installed for this check");

    let dir = std::env::temp_dir().join(format!("rewind-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut settings = Settings::default();
    settings.output_dir = dir.join("clips");
    settings.clip_seconds = 15; // shortest allowed; keeps the run quick
    settings.system_audio = true;
    settings.mic_enabled = false;
    settings.fps = 30;
    settings.bitrate_mbps = 10;
    settings.codec = rewind_engine::Codec::H264;

    println!(
        "test-source replay with clip_seconds={}",
        settings.clip_seconds
    );
    let recorder = Recorder::with_options(settings, true);

    // ---- instant replay + clip
    recorder.start();
    wait_for_mode(&recorder, Mode::Replay, Duration::from_secs(25));
    println!("  buffered: {:.1} s", recorder.buffered_seconds());
    assert!(recorder.buffered_seconds() >= 0.0);

    println!("buffering 6 s of test video");
    std::thread::sleep(Duration::from_secs(6));
    assert!(
        recorder.save_clip(None),
        "save_clip should accept the request"
    );

    let clip = match wait_for(&recorder, "clip", Duration::from_secs(60)) {
        Event::ClipSaved { path, duration } => {
            println!("  clip saved: {} ({duration:.2} s)", path.display());
            path
        }
        other => panic!("clip did not save: {other:?}"),
    };
    describe(&ffmpeg, &clip, true);

    // ---- full recording
    println!("full recording via the record hotkey path");
    recorder.start_recording();
    wait_for_mode(&recorder, Mode::Recording, Duration::from_secs(25));
    std::thread::sleep(Duration::from_secs(4));
    recorder.stop_recording();

    let recording = match wait_for(&recorder, "recording", Duration::from_secs(90)) {
        Event::RecordingSaved { path, duration } => {
            println!("  recording saved: {} ({duration:.2} s)", path.display());
            path
        }
        other => panic!("recording did not save: {other:?}"),
    };
    describe(&ffmpeg, &recording, true);

    // ---- replay resumes after a recording, and shutdown is clean
    recorder.start();
    wait_for_mode(&recorder, Mode::Replay, Duration::from_secs(25));
    recorder.shutdown();
    wait_for_state(&recorder, State::Off, Duration::from_secs(10));

    let state = recorder.state();
    assert_eq!(state, State::Off, "engine should be off after shutdown");

    let _ = std::fs::remove_dir_all(&dir);
    println!("end-to-end ok");
}
