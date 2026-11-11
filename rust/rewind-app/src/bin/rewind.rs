//! Rewind's tray binary.
//!
//! Everything runs on the main thread, because the platform requires it: menus must be
//! created there, and macOS dispatches hotkeys through the main run loop. Capture work
//! already happens on the engine's own threads, so the loop here only reads events and
//! updates the tray.

#[cfg(not(feature = "tray"))]
fn main() {
    eprintln!("build with --features tray to run the tray app");
}

#[cfg(feature = "tray")]
fn main() {
    if let Err(message) = run() {
        eprintln!("Rewind could not start: {message}");
        std::process::exit(1);
    }
}

#[cfg(feature = "tray")]
fn run() -> Result<(), String> {
    use std::time::Duration;

    use rewind_app::controller::Controller;
    use rewind_app::hotkeys::{HotkeyManager, Registration};
    use rewind_app::tray::TrayAction;
    use rewind_app::tray::TrayState;
    use rewind_app::tray_ui::{poll_menu_events, Tray};
    use rewind_engine::Settings;

    let settings = Settings::load();
    let mut controller = Controller::new(settings.clone());

    let logo = std::env::var_os("REWIND_LOGO").map(std::path::PathBuf::from);
    let tray_state = TrayState::from_engine(controller.state(), controller.mode());
    let tray = Tray::new(tray_state, logo.as_deref())?;

    let hotkeys = HotkeyManager::new().map_err(|e| e.to_string())?;
    let bound = hotkeys.apply_settings(&settings);
    for (name, reg) in &bound {
        if *reg != Registration::Ok {
            eprintln!("hotkey {name}: {reg:?} (already in use by another app?)");
        }
    }

    // Hotkey presses arrive on the platform's thread; queue them for the main loop.
    let (press_tx, press_rx) = std::sync::mpsc::channel::<String>();
    rewind_app::hotkeys::set_global_handler(move |action| {
        let _ = press_tx.send(action.to_string());
    });

    if settings.start_replay_on_launch {
        controller.recorder().start();
    }
    println!("Rewind running in the tray ({})", tray_state.label());

    let mut last_state = tray_state;
    loop {
        // Hotkey presses.
        while let Ok(action) = press_rx.try_recv() {
            match action.as_str() {
                // Quitting and window actions belong to the app, not the engine.
                "open" => println!("open window requested"),
                _ => {
                    for notice in controller.on_hotkey(&action) {
                        println!("{notice:?}");
                    }
                }
            }
        }

        // Tray clicks.
        for action in poll_menu_events() {
            match action {
                TrayAction::Quit => {
                    controller.shutdown();
                    hotkeys.stop().ok();
                    return Ok(());
                }
                TrayAction::OpenWindow => println!("open window requested"),
                TrayAction::ToggleReplay => {
                    controller.toggle_replay();
                }
                TrayAction::ToggleRecording => {
                    controller.toggle_recording();
                }
                TrayAction::SaveClip => {
                    controller.save_clip();
                }
                TrayAction::TakeScreenshot => {
                    controller.take_screenshot();
                }
                TrayAction::OpenClipsFolder => {
                    let _ = controller.open_clips_folder();
                }
            }
        }

        // Engine events: keep the menu in step with what capture is doing.
        for notice in controller.pump(Duration::from_millis(0)) {
            println!("{notice:?}");
        }
        let state = TrayState::from_engine(controller.state(), controller.mode());
        if state != last_state {
            tray.render(state);
            last_state = state;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
