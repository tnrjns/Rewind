//! The app window.
//!
//! One native window, opened from the tray or the open hotkey, showing the Clips page.
//! Closing it hides it; Rewind keeps capturing in the tray, which is the whole point of
//! the app.

mod clips;
mod dialogs;
pub mod format;
mod theme;

pub use clips::ClipsPage;
pub use format::Filter;

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use eframe::egui;

use crate::controller::{Controller, Notice};
use crate::hotkeys::HotkeyManager;
use crate::tray::{TrayAction, TrayState};
use rewind_engine::{Mode, State};

/// The platform pieces the window needs, built by the binary before the window opens:
/// the tray (menus must be made on the main thread), the hotkey manager and its queue.
pub struct Shell {
    pub hotkeys: Option<HotkeyManager>,
    #[cfg(feature = "tray")]
    pub tray: Option<crate::tray_ui::Tray>,
    /// Problems found at startup, such as a hotkey another app already owns.
    pub startup: Vec<String>,
}

/// Open the window and run until the app quits.
pub fn run(controller: Controller, shell: Shell) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rewind")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([780.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rewind",
        options,
        Box::new(move |cc| Ok(Box::new(RewindApp::new(cc, controller, shell)))),
    )
}

struct RewindApp {
    controller: Controller,
    page: ClipsPage,
    presses: Receiver<String>,
    hotkeys: Option<HotkeyManager>,
    #[cfg(feature = "tray")]
    tray: Option<crate::tray_ui::Tray>,
    last_tray_state: TrayState,
    quitting: bool,
}

impl RewindApp {
    fn new(cc: &eframe::CreationContext<'_>, controller: Controller, shell: Shell) -> RewindApp {
        theme::apply(&cc.egui_ctx);
        let mut page = ClipsPage::new(&cc.egui_ctx);
        for message in shell.startup {
            page.notice(message);
        }
        if controller.settings().start_replay_on_launch {
            controller.recorder().start();
        }

        // Hotkeys arrive on the platform's thread. Waking egui from there is what keeps
        // a press immediate when the window is hidden, which is the normal case: the
        // poll below is only a floor.
        let (presses_tx, presses) = channel::<String>();
        let ctx = cc.egui_ctx.clone();
        crate::hotkeys::set_global_handler(move |action| {
            let _ = presses_tx.send(action.to_string());
            ctx.request_repaint();
        });

        let last_tray_state = TrayState::from_engine(controller.state(), controller.mode());
        RewindApp {
            controller,
            page,
            presses,
            hotkeys: shell.hotkeys,
            #[cfg(feature = "tray")]
            tray: shell.tray,
            last_tray_state,
            quitting: false,
        }
    }

    /// Whether closing the window leaves a way back in.
    fn has_tray(&self) -> bool {
        #[cfg(feature = "tray")]
        {
            self.tray.is_some()
        }
        #[cfg(not(feature = "tray"))]
        {
            false
        }
    }

    fn show_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.controller.shutdown();
        if let Some(hotkeys) = &self.hotkeys {
            let _ = hotkeys.stop();
        }
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn absorb(&mut self, notices: Vec<Notice>) {
        for notice in notices {
            self.page.handle_notice(&notice);
        }
    }

    fn on_tray(&mut self, action: TrayAction, ctx: &egui::Context) {
        match action {
            TrayAction::OpenWindow => self.show_window(ctx),
            TrayAction::Quit => self.quit(ctx),
            TrayAction::OpenClipsFolder => {
                let _ = self.controller.open_clips_folder();
            }
            TrayAction::ToggleReplay => {
                let notices = self.controller.toggle_replay();
                self.absorb(notices);
            }
            TrayAction::ToggleRecording => {
                let notices = self.controller.toggle_recording();
                self.absorb(notices);
            }
            TrayAction::SaveClip => {
                let notices = self.controller.save_clip();
                self.absorb(notices);
            }
            TrayAction::TakeScreenshot => {
                let notices = self.controller.take_screenshot();
                self.absorb(notices);
            }
        }
    }
}

impl eframe::App for RewindApp {
    /// Shell work, once per frame — and also while the window is hidden, which is what
    /// keeps the hotkeys alive after the user closes the window.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            if self.has_tray() {
                // Closing the window is not quitting: Rewind keeps capturing.
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                self.quit(ctx);
            }
        }

        while let Ok(action) = self.presses.try_recv() {
            if action == "open" {
                self.show_window(ctx);
            } else {
                let notices = self.controller.on_hotkey(&action);
                self.absorb(notices);
            }
        }

        for action in tray_actions() {
            self.on_tray(action, ctx);
        }

        let notices = self.controller.pump(Duration::ZERO);
        self.absorb(notices);

        let state = TrayState::from_engine(self.controller.state(), self.controller.mode());
        if state != self.last_tray_state {
            #[cfg(feature = "tray")]
            if let Some(tray) = &self.tray {
                tray.render(state);
            }
            self.last_tray_state = state;
        }

        // Poll at a walking pace: menu clicks arrive through their own channel, and
        // this only has to keep the tray honest.
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let frame = egui::Frame::new()
            .fill(theme::BG)
            .inner_margin(egui::Margin::same(20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.header(ui);
            ui.add_space(8.0);
            self.page.ui(ui, &mut self.controller);
        });
    }
}

impl Drop for RewindApp {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

impl RewindApp {
    fn header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Clips");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color) = status_line(
                    self.controller.state(),
                    self.controller.mode(),
                    self.controller.settings().clip_seconds,
                );
                ui.label(egui::RichText::new(text).size(12.5).color(color));
            });
        });
    }
}

/// The one-line state shown opposite the page title.
pub fn status_line(state: State, mode: Mode, clip_seconds: u32) -> (String, egui::Color32) {
    if mode == Mode::Recording {
        return ("Recording".to_string(), theme::WARN);
    }
    match state {
        State::Saving => ("Saving recording…".to_string(), theme::WARN),
        State::Starting => ("Starting…".to_string(), theme::MUTED),
        State::On => (
            format!("Instant Replay on · {clip_seconds} s"),
            theme::GREEN,
        ),
        State::Off => ("Instant Replay off".to_string(), theme::SUBTLE),
    }
}

fn tray_actions() -> Vec<TrayAction> {
    #[cfg(feature = "tray")]
    {
        crate::tray_ui::poll_menu_events()
    }
    #[cfg(not(feature = "tray"))]
    {
        Vec::new()
    }
}

/// Decode an image file into an egui texture, for thumbnails and trim previews.
pub(crate) fn load_texture(
    ctx: &egui::Context,
    name: &str,
    path: &Path,
) -> Option<egui::TextureHandle> {
    let bytes = std::fs::read(path).ok()?;
    let decoded = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    if size[0] == 0 || size[1] == 0 {
        return None;
    }
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    Some(ctx.load_texture(name.to_string(), pixels, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_names_what_is_running() {
        assert_eq!(
            status_line(State::On, Mode::Replay, 30).0,
            "Instant Replay on · 30 s"
        );
        assert_eq!(
            status_line(State::Off, Mode::Off, 30).0,
            "Instant Replay off"
        );
        assert_eq!(status_line(State::On, Mode::Recording, 30).0, "Recording");
    }
}
