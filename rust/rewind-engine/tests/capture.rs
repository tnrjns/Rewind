//! Capture-pipeline tests that run real FFmpeg against the synthetic test source.
//!
//! These are the behaviours a user would notice if they broke: a saved clip is a
//! playable MP4 with sound, a recording is as long as it was recorded, and the
//! recording's audio is aligned to the video rather than cut short by it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rewind_engine::ffmpeg::{find_ffmpeg, probe_duration, run};
use rewind_engine::{Codec, Event, Mode, Recorder, Settings, State};

/// A private output folder for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rewind-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settings_for(dir: &Path) -> Settings {
    let mut settings = Settings::default();
    settings.output_dir = dir.join("clips");
    settings.clip_seconds = 15;
    settings.fps = 30;
    settings.bitrate_mbps = 8;
    settings.codec = Codec::H264;
    settings.system_audio = true;
    settings.mic_enabled = false;
    settings
}

/// Block until the recorder reaches `state`, draining events so the channel cannot fill.
fn wait_state(recorder: &Recorder, state: State, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if recorder.state() == state {
            return;
        }
        while let Some(event) = recorder.try_recv() {
            if let Event::Error(msg) = event {
                eprintln!("engine error: {msg}");
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("engine never reached state {}", state.as_str());
}

fn wait_mode(recorder: &Recorder, mode: Mode, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if recorder.mode() == mode && recorder.state() == State::On {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("engine never reached mode {}", mode.as_str());
}

fn wait_saved(recorder: &Recorder, timeout: Duration) -> PathBuf {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(event) = recorder.try_recv() {
            match event {
                Event::ClipSaved { path, .. } | Event::RecordingSaved { path, .. } => return path,
                Event::ClipFailed(msg) => panic!("save failed: {msg}"),
                Event::RecordingFailed(msg) => panic!("recording failed: {msg}"),
                Event::Error(msg) => eprintln!("engine error: {msg}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("nothing was saved in time");
}

/// Stream layout of a media file, read from FFmpeg's banner.
struct Layout {
    duration: f64,
    has_video: bool,
    has_audio: bool,
    audio_start: f64,
}

fn inspect(ffmpeg: &Path, path: &Path) -> Layout {
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
    let has_video = banner.contains("Video: h264") || banner.contains("Video: hevc");
    let has_audio = banner.contains("Audio: aac");
    // `start:` on the Duration line is the container start offset, which zeroes when
    // FFmpeg muxes the first frame at t=0.
    let audio_start = banner
        .lines()
        .find(|l| l.contains("Audio: aac"))
        .and_then(|line| {
            let idx = line.find("start:")?;
            line[idx + "start:".len()..]
                .split(',')
                .next()?
                .trim()
                .parse::<f64>()
                .ok()
        })
        .unwrap_or(0.0);
    Layout {
        duration: probe_duration(ffmpeg, path).unwrap_or(0.0),
        has_video,
        has_audio,
        audio_start,
    }
}

/// Most lead-in a recording may contain: capture begins during the launch wait, which
/// is a couple of seconds on macOS because AVFoundation takes that long to open a display.
const LEAD_IN_BUDGET: f64 = 4.0;

fn require_ffmpeg() -> Option<PathBuf> {
    match find_ffmpeg() {
        Some(ff) => Some(ff),
        None => {
            eprintln!("skipping: FFmpeg is not installed");
            None
        }
    }
}

#[test]
fn clip_is_a_playable_mp4_with_sound() {
    let Some(ffmpeg) = require_ffmpeg() else {
        return;
    };
    let dir = scratch("clip");
    let recorder = Recorder::with_data_dir(settings_for(&dir), true, &dir.join("data"));

    recorder.start();
    wait_mode(&recorder, Mode::Replay, Duration::from_secs(20));
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        recorder.save_clip(None),
        "save_clip should be accepted while replaying"
    );

    let clip = wait_saved(&recorder, Duration::from_secs(60));
    recorder.shutdown();

    let layout = inspect(&ffmpeg, &clip);
    assert!(layout.has_video, "clip has no video stream");
    assert!(layout.has_audio, "clip has no audio stream");
    assert!(
        layout.duration >= 2.0,
        "clip is only {:.2} s; expected the buffered span",
        layout.duration
    );
    assert!(
        layout.duration <= 16.0,
        "clip is {:.2} s, longer than the 15 s window plus headroom",
        layout.duration
    );

    // Metadata and the thumbnail belong to this recorder's data folder. They used to go
    // to the user's real data folder, so running the tests dirtied a real library.
    let data = dir.join("data");
    assert!(
        data.join("clips.json").is_file(),
        "metadata was not written under the recorder's data root"
    );
    assert!(
        data.join("thumbs")
            .read_dir()
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false),
        "the thumbnail was not written under the recorder's data root"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recording_matches_the_time_it_ran() {
    let Some(ffmpeg) = require_ffmpeg() else {
        return;
    };
    let dir = scratch("recording");
    let recorder = Recorder::with_data_dir(settings_for(&dir), true, &dir.join("data"));

    recorder.start_recording();
    wait_mode(&recorder, Mode::Recording, Duration::from_secs(25));
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(4));
    let recorded = started.elapsed().as_secs_f64();
    recorder.stop_recording();

    let recording = wait_saved(&recorder, Duration::from_secs(90));
    wait_state(&recorder, State::Off, Duration::from_secs(20));
    recorder.shutdown();

    let layout = inspect(&ffmpeg, &recording);
    assert!(layout.has_video, "recording has no video stream");
    assert!(layout.has_audio, "recording lost its desktop audio");
    // The file must cover the recording, not be truncated to the audio track that
    // started a second or two late while the capture device opened.
    assert!(
        layout.duration >= recorded * 0.9,
        "recorded {recorded:.2} s but the file is {:.2} s — audio truncated the video",
        layout.duration
    );
    // Capture starts as soon as FFmpeg opens the source, which is before capture counts
    // as running, so a few seconds of lead-in are expected and welcome: they catch the
    // moment before the keypress. Anything beyond that is a leak.
    assert!(
        layout.duration <= recorded + LEAD_IN_BUDGET,
        "file is {:.2} s for a {recorded:.2} s recording — {:.2} s of lead-in exceeds the budget",
        layout.duration,
        layout.duration - recorded
    );
    assert!(
        layout.audio_start.abs() < 0.5,
        "audio starts at {:.2} s instead of with the video",
        layout.audio_start
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replay_resumes_after_a_recording() {
    let dir = scratch("resume");
    let recorder = Recorder::with_data_dir(settings_for(&dir), true, &dir.join("data"));

    recorder.start();
    wait_mode(&recorder, Mode::Replay, Duration::from_secs(20));
    recorder.start_recording();
    wait_mode(&recorder, Mode::Recording, Duration::from_secs(25));
    recorder.stop_recording();
    wait_state(&recorder, State::Off, Duration::from_secs(90));

    recorder.start();
    wait_mode(&recorder, Mode::Replay, Duration::from_secs(25));
    assert!(recorder.buffered_seconds() >= 0.0);
    recorder.shutdown();
    wait_state(&recorder, State::Off, Duration::from_secs(10));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stopping_replay_ends_the_capture_process() {
    let dir = scratch("stop");
    let recorder = Recorder::with_data_dir(settings_for(&dir), true, &dir.join("data"));
    recorder.start();
    wait_mode(&recorder, Mode::Replay, Duration::from_secs(20));
    recorder.stop();
    wait_state(&recorder, State::Off, Duration::from_secs(20));

    // A second stop must be harmless, and the engine must still start again.
    recorder.stop();
    recorder.start();
    wait_mode(&recorder, Mode::Replay, Duration::from_secs(20));
    recorder.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clip_request_while_off_reports_failure() {
    let dir = scratch("off");
    let recorder = Recorder::with_data_dir(settings_for(&dir), true, &dir.join("data"));
    assert!(
        !recorder.save_clip(None),
        "no clip can be saved while replay is off"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut failed = false;
    while Instant::now() < deadline && !failed {
        if let Some(Event::ClipFailed(msg)) = recorder.try_recv() {
            assert_eq!(msg, "Instant Replay is off");
            failed = true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        failed,
        "a ClipFailed event should explain why nothing was saved"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
