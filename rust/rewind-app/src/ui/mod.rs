//! The app window: the sidebar, the page header, and the pages themselves.
//!
//! The window mirrors the Python app's layout — a 218 px rail with the brand, the page
//! list and the capture state, and a scrolling page to its right — because the port is
//! meant to look like the app it replaces, not like a new one.

mod clips;
mod dialogs;
pub mod format;
mod hotkeys;
mod keys;
mod replay;
mod theme;
mod widgets;

pub use clips::ClipsPage;
pub use format::Filter;

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Vec2};

use crate::controller::{Controller, Notice, SettingValue};
use crate::hotkeys::HotkeyManager;
use crate::tray::{TrayAction, TrayState};
use rewind_engine::{Mode, State};

/// How long a capture setting has to sit still before the buffer restarts. The Python
/// app used the same 900 ms, so dragging a slider across its range restarts once.
const RESTART_DELAY: Duration = Duration::from_millis(900);

/// A settings write a page is asking for.
///
/// Pages never call `Controller::set` themselves: the shell applies every change so all
/// of them share one restart debounce.
pub struct Change {
    pub key: &'static str,
    pub value: SettingValue,
}

impl Change {
    pub fn new(key: &'static str, value: impl Into<SettingValue>) -> Change {
        Change {
            key,
            value: value.into(),
        }
    }
}

/// What a page asks the shell to do after drawing.
#[derive(Default)]
pub struct PageOutput {
    pub changes: Vec<Change>,
    /// `Some(true)` unbinds the global hotkeys (a combo is being captured);
    /// `Some(false)` binds them again.
    pub hotkeys: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageKind {
    Replay,
    Hotkeys,
    Clips,
}

impl PageKind {
    /// The order the rail lists them in, matching the Python app's nav.
    const ALL: [PageKind; 3] = [PageKind::Replay, PageKind::Hotkeys, PageKind::Clips];

    fn label(self) -> &'static str {
        match self {
            PageKind::Replay => "Replay",
            PageKind::Hotkeys => "Hotkeys",
            PageKind::Clips => "Clips",
        }
    }
}

enum Page {
    Replay(replay::ReplayPage),
    Hotkeys(hotkeys::HotkeysPage),
    // Boxed: the Clips page carries textures, jobs and a selection, and the enum should
    // not be sized for its largest member on every frame.
    Clips(Box<ClipsPage>),
}

impl Page {
    fn title(&self) -> &'static str {
        match self {
            Page::Replay(_) => "Replay",
            Page::Hotkeys(_) => "Hotkeys",
            Page::Clips(_) => "Clips",
        }
    }

    fn subtitle(&self) -> Option<&'static str> {
        match self {
            Page::Replay(_) => Some(replay::ReplayPage::SUBTITLE),
            Page::Hotkeys(_) => Some(hotkeys::HotkeysPage::SUBTITLE),
            Page::Clips(_) => None,
        }
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        controller: &mut Controller,
        hotkeys: &Option<HotkeyManager>,
    ) -> PageOutput {
        match self {
            Page::Replay(page) => PageOutput {
                changes: page.ui(ui, controller),
                hotkeys: None,
            },
            Page::Hotkeys(page) => page.ui(ui, controller, hotkeys),
            Page::Clips(page) => {
                page.ui(ui, controller);
                PageOutput::default()
            }
        }
    }
}

/// The platform pieces the window needs, built by the binary before it opens: the tray
/// (menus must be made on the main thread), the hotkey manager and the startup notices.
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
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([900.0, 600.0]),
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
    pages: [Page; 3],
    current: usize,
    presses: Receiver<String>,
    hotkeys: Option<HotkeyManager>,
    #[cfg(feature = "tray")]
    tray: Option<crate::tray_ui::Tray>,
    last_tray_state: TrayState,
    restart_due: Option<Instant>,
    quitting: bool,
}

impl RewindApp {
    fn new(cc: &eframe::CreationContext<'_>, controller: Controller, shell: Shell) -> RewindApp {
        theme::apply(&cc.egui_ctx);
        let mut clips = ClipsPage::new(&cc.egui_ctx);
        for message in shell.startup {
            clips.notice(message);
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
            pages: [
                Page::Replay(replay::ReplayPage::new()),
                Page::Hotkeys(hotkeys::HotkeysPage::new()),
                Page::Clips(Box::new(clips)),
            ],
            current: 0,
            presses,
            hotkeys: shell.hotkeys,
            #[cfg(feature = "tray")]
            tray: shell.tray,
            last_tray_state,
            restart_due: None,
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
        for page in &mut self.pages {
            if let Page::Clips(clips) = page {
                for notice in &notices {
                    clips.handle_notice(notice);
                }
            }
        }
    }

    /// Bind the current settings' hotkeys, after a change or a capture.
    fn rebind_hotkeys(&mut self) {
        if let Some(manager) = &self.hotkeys {
            let settings = self.controller.settings();
            manager.apply_settings(settings);
        }
    }

    /// Apply what the page asked for, and schedule the one restart it may need.
    fn apply_changes(&mut self, changes: Vec<Change>) {
        let mut rebind = false;
        for change in changes {
            if change.key.starts_with("hotkey_") {
                rebind = true;
            }
            let (notices, restart) = self.controller.set(change.key, change.value);
            self.absorb(notices);
            if restart {
                self.restart_due = Some(Instant::now() + RESTART_DELAY);
            }
        }
        if rebind {
            self.rebind_hotkeys();
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

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            theme::rewind_glyph(ui, 30.0);
            ui.add_space(7.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Rewind").size(18.0).strong());
                ui.label(
                    RichText::new("Instant Replay")
                        .size(11.0)
                        .color(theme::MUTED),
                );
            });
        });
        ui.add_space(22.0);

        for (index, kind) in PageKind::ALL.iter().enumerate() {
            if nav_button(ui, kind.label(), index == self.current) {
                self.current = index;
            }
        }

        // The capture state sits at the bottom of the rail, as it does in the Python app.
        ui.add_space((ui.available_height() - 26.0).max(0.0));
        let (text, active) = status_line(self.controller.state(), self.controller.mode());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
            ui.painter().circle_filled(
                rect.center(),
                4.0,
                if active { theme::GREEN } else { theme::SUBTLE },
            );
            ui.label(RichText::new(text).size(12.0).color(theme::MUTED));
        });
    }
}

impl eframe::App for RewindApp {
    /// Shell work, once per frame — and also while the window is hidden, which is what
    /// keeps the hotkeys alive after the user closes it.
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

        if let Some(due) = self.restart_due {
            if Instant::now() >= due {
                self.restart_due = None;
                self.controller.recorder().restart();
            } else {
                ctx.request_repaint_after(RESTART_DELAY);
            }
        }

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
        egui::Panel::left("rewind-nav")
            .exact_size(218.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 14,
                        top: 20,
                        bottom: 16,
                    }),
            )
            .show(ui, |ui| self.sidebar(ui));

        let mut output = PageOutput::default();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: 36,
                        right: 36,
                        top: 30,
                        bottom: 30,
                    }),
            )
            .show(ui, |ui| {
                let Self {
                    controller,
                    pages,
                    current,
                    hotkeys,
                    ..
                } = self;
                let page = &mut pages[*current];
                ui.label(RichText::new(page.title()).size(26.0).strong());
                if let Some(subtitle) = page.subtitle() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(subtitle).size(13.0).color(theme::MUTED));
                }
                ui.add_space(16.0);
                output = page.ui(ui, controller, hotkeys);
            });
        self.apply_changes(output.changes);
        match output.hotkeys {
            Some(true) => {
                if let Some(manager) = &self.hotkeys {
                    let _ = manager.stop();
                }
            }
            Some(false) => self.rebind_hotkeys(),
            None => {}
        }
    }
}

impl Drop for RewindApp {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

/// A full-width rail entry: left-aligned, with hover and selected states.
fn nav_button(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    let hovered = response.hovered();
    let fill = if selected {
        Color32::from_rgb(0x30, 0x30, 0x30)
    } else if hovered {
        theme::CARD_2
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), fill);
    }
    let color = if selected || hovered {
        theme::TEXT
    } else {
        theme::MUTED
    };
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        color,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The one-line state shown at the bottom of the rail, and whether it is "live".
pub fn status_line(state: State, mode: Mode) -> (String, bool) {
    if mode == Mode::Recording {
        let text = if state == State::Starting {
            "Starting recording…"
        } else {
            "Recording"
        };
        return (text.to_string(), true);
    }
    match state {
        State::On => ("Replay on".to_string(), true),
        State::Starting => ("Starting…".to_string(), true),
        State::Saving => ("Saving…".to_string(), true),
        State::Off => ("Replay off".to_string(), false),
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
    fn the_rail_names_what_is_running() {
        assert_eq!(status_line(State::On, Mode::Replay).0, "Replay on");
        assert_eq!(status_line(State::Off, Mode::Off).0, "Replay off");
        assert_eq!(status_line(State::On, Mode::Recording).0, "Recording");
        assert_eq!(
            status_line(State::Starting, Mode::Recording).0,
            "Starting recording…"
        );
    }

    #[test]
    fn only_a_running_capture_lights_the_dot() {
        assert!(status_line(State::On, Mode::Replay).1);
        assert!(status_line(State::On, Mode::Recording).1);
        assert!(!status_line(State::Off, Mode::Off).1);
    }
}
