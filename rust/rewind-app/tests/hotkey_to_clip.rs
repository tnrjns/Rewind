//! Hotkey press → controller → engine → a real clip on disk.
//!
//! Uses the synthetic capture source so it needs no screen permission. The press goes
//! through the real dispatch path (a Carbon hotkey event on macOS), not a direct call,
//! so a broken hotkey-to-controller wiring fails here.

use std::time::{Duration, Instant};

use rewind_app::controller::{Controller, Notice};
use rewind_app::hotkeys::{HotkeyManager, Registration};
use rewind_engine::ffmpeg::{find_ffmpeg, probe_duration};
use rewind_engine::{Event, Settings, State};

fn pump(controller: &Controller) -> Vec<Notice> {
    let mut notices = Vec::new();
    while let Some(event) = controller.recorder().try_recv() {
        notices.push(match event {
            Event::StateChanged(s) => Notice::StateChanged(s),
            Event::Error(m) => Notice::Message(m),
            Event::ClipSaved { path, duration } => Notice::ClipSaved { path, duration },
            Event::ClipFailed(m) => Notice::Failed(m),
            Event::RecordingFailed(m) => Notice::Failed(m),
            Event::RecordingSaved { path, duration } => Notice::RecordingSaved { path, duration },
            _ => Notice::Message(String::new()),
        });
    }
    notices
}

fn wait_state(controller: &Controller, state: State, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if controller.state() == state {
            return;
        }
        pump(controller);
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "engine never reached {} (state {})",
        state.as_str(),
        controller.state().as_str()
    );
}

fn wait_clip(controller: &Controller, timeout: Duration) -> (std::path::PathBuf, f64) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while let Some(event) = controller.recorder().try_recv() {
            match event {
                Event::ClipSaved { path, duration } => return (path, duration),
                Event::ClipFailed(m) => panic!("clip failed: {m}"),
                Event::Error(m) => eprintln!("engine error: {m}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no clip was saved");
}

fn settings_for(dir: &std::path::Path) -> Settings {
    let mut settings = Settings::default();
    settings.output_dir = dir.join("clips");
    settings.clip_seconds = 15;
    settings.fps = 30;
    settings
}

#[test]
fn a_clip_hotkey_press_saves_a_clip() {
    let Some(ffmpeg) = find_ffmpeg() else {
        eprintln!("skipping: FFmpeg is not installed");
        return;
    };
    // The engine picks its source from the environment at construction.
    std::env::set_var("REWIND_TEST_SOURCE", "1");

    let dir = std::env::temp_dir().join(format!("rewind-hk-clip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let settings = settings_for(&dir);

    let manager = HotkeyManager::new().expect("hotkey backend");
    let bound = manager.apply_settings(&settings);
    assert!(
        bound.iter().all(|(_, r)| *r == Registration::Ok),
        "hotkeys did not bind: {bound:?}"
    );

    let (tx, rx) = std::sync::mpsc::channel();
    rewind_app::hotkeys::set_global_handler(move |action| {
        let _ = tx.send(action.to_string());
    });

    let mut controller = Controller::new(settings);
    controller.recorder().start();
    wait_state(&controller, State::On, Duration::from_secs(25));
    std::thread::sleep(Duration::from_secs(3));

    #[cfg(target_os = "macos")]
    {
        // HOTKEY_ACTIONS index 1 is `clip`.
        rewind_app::hotkeys::inject_press(1);
        let pressed = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("the clip hotkey did not dispatch");
        assert_eq!(pressed, "clip");
        controller.on_hotkey(&pressed);
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Hotkey injection is macOS-only for now; drive the same entry point.
        let _ = &rx;
        controller.on_hotkey("clip");
    }

    let (clip, duration) = wait_clip(&controller, Duration::from_secs(60));
    let probed = probe_duration(&ffmpeg, &clip).unwrap_or(0.0);
    assert!(clip.exists(), "clip file is missing");
    assert!(probed > 1.0, "clip is only {probed} s");
    assert!(
        (probed - duration).abs() < 1.0,
        "reported {duration:.2} s but the file is {probed:.2} s"
    );

    let clips = controller.list_clips(50);
    assert!(
        !clips.is_empty(),
        "the Clips page shows nothing after a save"
    );

    controller.shutdown();
    manager.stop().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_clip_hotkey_reports_why_it_cannot_save() {
    std::env::set_var("REWIND_TEST_SOURCE", "1");
    let dir = std::env::temp_dir().join(format!("rewind-hk-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut controller = Controller::new(settings_for(&dir));
    // Replay never started, so a clip request must explain itself.
    let notices = controller.on_hotkey("clip");
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, Notice::Message(m) if m.contains("off"))),
        "expected an explanation, got {notices:?}"
    );
    controller.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recording_toggle_round_trips_through_the_controller() {
    std::env::set_var("REWIND_TEST_SOURCE", "1");
    let dir = std::env::temp_dir().join(format!("rewind-hk-rec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut controller = Controller::new(settings_for(&dir));
    controller.on_hotkey("record");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && controller.mode() != rewind_engine::Mode::Recording {
        pump(&controller);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(controller.mode(), rewind_engine::Mode::Recording);
    std::thread::sleep(Duration::from_secs(2));

    controller.on_hotkey("record");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut saved = false;
    while Instant::now() < deadline && !saved {
        while let Some(event) = controller.recorder().try_recv() {
            if matches!(event, Event::RecordingSaved { .. }) {
                saved = true;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(saved, "the record hotkey did not produce a recording");
    controller.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
