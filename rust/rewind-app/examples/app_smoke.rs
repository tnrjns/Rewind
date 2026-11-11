//! Drive the real tray app binary end to end: hotkey in, clip out.
//!
//! Uses the injection path because synthetic keypresses never reach Carbon — the same
//! reason the hotkey tests inject rather than press. Proves the pieces are wired
//! together in the actual binary, not just in library tests.
#[cfg(feature = "tray")]
fn main() {
    use rewind_app::controller::Controller;
    use rewind_app::hotkeys::{HotkeyManager, Registration};
    use rewind_app::tray::TrayState;
    use rewind_engine::Settings;
    use std::time::{Duration, Instant};

    std::env::set_var("REWIND_TEST_SOURCE", "1");
    let dir = std::env::temp_dir().join("rewind-app-smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut settings = Settings::default();
    settings.output_dir = dir.clone();
    settings.clip_seconds = 15;
    settings.fps = 30;

    let mut controller = Controller::new(settings.clone());
    let tray_state = TrayState::from_engine(controller.state(), controller.mode());
    println!("tray starts at: {}", tray_state.label());

    let hotkeys = HotkeyManager::new().expect("hotkey backend");
    let bound = hotkeys.apply_settings(&settings);
    assert!(
        bound.iter().all(|(_, r)| *r == Registration::Ok),
        "{bound:?}"
    );
    println!("hotkeys bound: {}", bound.len());

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    rewind_app::hotkeys::set_global_handler(move |a| {
        let _ = tx.send(a.to_string());
    });

    controller.recorder().start();
    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline && controller.state() != rewind_engine::State::On {
        controller.pump(Duration::from_millis(10));
    }
    assert_eq!(controller.state(), rewind_engine::State::On);
    println!(
        "replay on, tray would show: {}",
        TrayState::from_engine(controller.state(), controller.mode()).label()
    );

    std::thread::sleep(Duration::from_secs(3));
    rewind_app::hotkeys::inject_press(1); // clip
    let action = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("hotkey dispatched");
    println!("hotkey -> {action}");
    controller.on_hotkey(&action);

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut saved = None;
    while Instant::now() < deadline && saved.is_none() {
        while let Some(event) = controller.recorder().try_recv() {
            if let rewind_engine::Event::ClipSaved { path, duration } = event {
                saved = Some((path, duration));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (path, duration) = saved.expect("clip saved");
    println!(
        "clip: {} ({duration:.2} s, {} bytes)",
        path.file_name().unwrap().to_string_lossy(),
        std::fs::metadata(&path).unwrap().len()
    );
    controller.shutdown();
    hotkeys.stop().ok();
    let _ = std::fs::remove_dir_all(&dir);
    println!("app smoke ok");
}
#[cfg(not(feature = "tray"))]
fn main() {
    println!("needs --features tray");
}
