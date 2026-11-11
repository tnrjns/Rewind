//! Rewind's binary: the window, plus the tray and hotkeys when built with `tray`.
//!
//! Everything platform-facing happens here, on the main thread, before the window opens:
//! menus and the status item must be created there on both Windows and macOS, and the
//! hotkey backend owns its own message loop.

use rewind_app::controller::Controller;
use rewind_app::hotkeys::{HotkeyManager, Registration};
use rewind_app::tray::TrayState;
use rewind_app::ui::{self, Shell};
use rewind_engine::Settings;

fn main() {
    // Before any window exists: Windows freezes DPI awareness at that moment, and a
    // process that misses the window reports scaled monitor geometry for its whole life.
    rewind_engine::platform::init_process();

    if let Err(message) = run() {
        eprintln!("Rewind could not start: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let settings = Settings::load();
    let controller = Controller::new(settings.clone());

    let mut startup = Vec::new();
    let hotkeys = start_hotkeys(&settings, &mut startup);

    let state = TrayState::from_engine(controller.state(), controller.mode());
    let logo = std::env::var_os("REWIND_LOGO").map(std::path::PathBuf::from);

    #[cfg(feature = "tray")]
    let shell = Shell {
        hotkeys,
        tray: build_tray(state, logo.as_deref(), &mut startup),
        startup,
    };
    #[cfg(not(feature = "tray"))]
    let shell = Shell { hotkeys, startup };
    let _ = (state, logo);

    ui::run(controller, shell).map_err(|e| e.to_string())
}

/// Bind the configured hotkeys, reporting the ones another app already owns.
fn start_hotkeys(settings: &Settings, startup: &mut Vec<String>) -> Option<HotkeyManager> {
    match HotkeyManager::new() {
        Ok(manager) => {
            for (name, registration) in manager.apply_settings(settings) {
                if !matches!(registration, Registration::Ok | Registration::Unset) {
                    startup.push(format!("Hotkey {name} is already in use by another app"));
                }
            }
            Some(manager)
        }
        Err(error) => {
            startup.push(format!("Hotkeys unavailable: {error}"));
            None
        }
    }
}

/// Create the tray icon, or explain why it is missing.
#[cfg(feature = "tray")]
fn build_tray(
    state: TrayState,
    logo: Option<&std::path::Path>,
    startup: &mut Vec<String>,
) -> Option<rewind_app::tray_ui::Tray> {
    match rewind_app::tray_ui::Tray::new(state, logo) {
        Ok(tray) => Some(tray),
        Err(error) => {
            startup.push(format!("Tray icon unavailable: {error}"));
            None
        }
    }
}
