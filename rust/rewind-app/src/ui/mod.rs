//! The app window: the sidebar, the page header, and the pages themselves.
//!
//! The window mirrors the Python app's layout — a 218 px rail with the brand, the page
//! list and the capture state, and a scrolling page to its right — because the port is
//! meant to look like the app it replaces, not like a new one.

mod clips;
mod dialogs;
pub mod format;
mod home;
mod hotkeys;
mod keys;
mod overlay_page;
mod pill;
mod recording;
mod replay;
mod sound;
mod theme;
mod thumbs;
mod widgets;

pub use clips::ClipsPage;
pub use format::Filter;

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Vec2};

use crate::controller::{Controller, Notice, SettingValue};
use crate::hotkeys::HotkeyManager;
use crate::overlay::Flash;
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
    /// Notices the page produced itself, such as a recording that failed to start.
    pub notices: Vec<Notice>,
    /// `Some(true)` unbinds the global hotkeys (a combo is being captured);
    /// `Some(false)` binds them again.
    pub hotkeys: Option<bool>,
    /// A flash the page wants the status pill to show, such as a preview.
    pub flash: Option<Flash>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageKind {
    Home,
    Replay,
    Recording,
    Hotkeys,
    Sound,
    Overlay,
    Clips,
}

impl PageKind {
    /// The order the rail lists them in, matching the Python app's nav.
    const ALL: [PageKind; 7] = [
        PageKind::Home,
        PageKind::Replay,
        PageKind::Recording,
        PageKind::Hotkeys,
        PageKind::Sound,
        PageKind::Overlay,
        PageKind::Clips,
    ];

    fn label(self) -> &'static str {
        match self {
            PageKind::Home => "Home",
            PageKind::Replay => "Replay",
            PageKind::Recording => "Recording",
            PageKind::Hotkeys => "Hotkeys",
            PageKind::Sound => "Sound",
            PageKind::Overlay => "Overlay",
            PageKind::Clips => "Clips",
        }
    }
}

enum Page {
    Home(home::HomePage),
    Replay(replay::ReplayPage),
    Recording(recording::RecordingPage),
    Hotkeys(hotkeys::HotkeysPage),
    Sound(sound::SoundPage),
    Overlay(overlay_page::OverlayPage),
    // Boxed: the Clips page carries textures, jobs and a selection, and the enum should
    // not be sized for its largest member on every frame.
    Clips(Box<ClipsPage>),
}

impl Page {
    fn title(&self) -> &'static str {
        match self {
            Page::Home(_) => "Home",
            Page::Replay(_) => "Replay",
            Page::Recording(_) => "Recording",
            Page::Hotkeys(_) => "Hotkeys",
            Page::Sound(_) => "Sound",
            Page::Overlay(_) => "Overlay",
            Page::Clips(_) => "Clips",
        }
    }

    fn subtitle(&self) -> Option<&'static str> {
        match self {
            Page::Home(_) => None,
            Page::Replay(_) => Some(replay::ReplayPage::SUBTITLE),
            Page::Recording(_) => Some(recording::RecordingPage::SUBTITLE),
            Page::Hotkeys(_) => Some(hotkeys::HotkeysPage::SUBTITLE),
            Page::Sound(_) => Some(sound::SoundPage::SUBTITLE),
            Page::Overlay(_) => Some(overlay_page::OverlayPage::SUBTITLE),
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
            Page::Home(page) => page.ui(ui, controller),
            Page::Replay(page) => PageOutput {
                changes: page.ui(ui, controller),
                ..PageOutput::default()
            },
            Page::Recording(page) => page.ui(ui, controller),
            Page::Hotkeys(page) => page.ui(ui, controller, hotkeys),
            Page::Sound(page) => page.ui(ui, controller),
            Page::Overlay(page) => page.ui(ui, controller),
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
    pages: [Page; 7],
    current: usize,
    pill: pill::PillWindow,
    presses: Receiver<String>,
    hotkeys: Option<HotkeyManager>,
    #[cfg(feature = "tray")]
    tray: Option<crate::tray_ui::Tray>,
    last_tray_state: TrayState,
    indicator: RailIndicator,
    restart_due: Option<Instant>,
    quitting: bool,
}

impl RewindApp {
    fn new(cc: &eframe::CreationContext<'_>, controller: Controller, shell: Shell) -> RewindApp {
        theme::apply(&cc.egui_ctx);
        let mut clips = ClipsPage::new(&cc.egui_ctx);
        let mut home = home::HomePage::new(&cc.egui_ctx);
        for message in &shell.startup {
            clips.notice(message.clone());
            home.notice(message.clone());
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
                Page::Home(home),
                Page::Replay(replay::ReplayPage::new()),
                Page::Recording(recording::RecordingPage::new()),
                Page::Hotkeys(hotkeys::HotkeysPage::new()),
                Page::Sound(sound::SoundPage::new()),
                Page::Overlay(overlay_page::OverlayPage::new()),
                Page::Clips(Box::new(clips)),
            ],
            current: 0,
            pill: pill::PillWindow::new(),
            presses,
            hotkeys: shell.hotkeys,
            #[cfg(feature = "tray")]
            tray: shell.tray,
            last_tray_state,
            indicator: RailIndicator::new(),
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
        // The pill mirrors the same notices that feed the pages: one save, one message.
        let now = Instant::now();
        for notice in &notices {
            match notice {
                Notice::ClipStarted => self.pill.flash(Flash::SavingClip, now),
                Notice::ClipSaved { duration, .. } => self.pill.flash(
                    Flash::ClipSaved {
                        seconds: duration.round() as u32,
                    },
                    now,
                ),
                Notice::RecordingSaved { duration, .. } => self.pill.flash(
                    Flash::RecordingSaved {
                        seconds: duration.round() as u32,
                    },
                    now,
                ),
                Notice::ScreenshotSaved(_) => self.pill.flash(Flash::Screenshot, now),
                Notice::Failed(message) => self.pill.flash(
                    Flash::Failed {
                        title: "Failed".to_string(),
                        detail: message.clone(),
                    },
                    now,
                ),
                Notice::StateChanged(_) => self.pill.note_activity(now),
                _ => {}
            }
        }
        for page in &mut self.pages {
            match page {
                Page::Clips(clips) => {
                    for notice in &notices {
                        clips.handle_notice(notice);
                    }
                }
                Page::Home(home) => {
                    for notice in &notices {
                        home.handle_notice(notice);
                    }
                }
                _ => {}
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
            ui.spacing_mut().item_spacing.x = 10.0;
            // The brand row is inset six points, as in the Python app, without moving
            // the nav items that follow it.
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
            theme::logo(ui.painter(), rect);
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

        let mut rects = [egui::Rect::ZERO; PageKind::ALL.len()];
        for (index, kind) in PageKind::ALL.iter().enumerate() {
            let (clicked, rect) = nav_button(ui, kind.label(), index == self.current);
            rects[index] = rect;
            if clicked {
                self.current = index;
            }
        }
        if let Some(rect) = rects.get(self.current) {
            if self.indicator.show(ui, *rect, Instant::now()) {
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        }

        // The capture state sits at the bottom of the rail, as it does in the Python app.
        ui.add_space((ui.available_height() - 26.0).max(0.0));
        let (text, active) = status_line(self.controller.state(), self.controller.mode());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(8.0);
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
    /// Transparent for every viewport: the main window's panels cover theirs, and the
    /// pill has to show the desktop around its rounded card.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

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
        self.absorb(output.notices);
        match output.hotkeys {
            Some(true) => {
                if let Some(manager) = &self.hotkeys {
                    let _ = manager.stop();
                }
            }
            Some(false) => self.rebind_hotkeys(),
            None => {}
        }
        if let Some(flash) = output.flash {
            self.pill.flash(flash, Instant::now());
        }

        let Self {
            controller, pill, ..
        } = self;
        pill.show(
            ui.ctx(),
            controller.settings(),
            controller.state(),
            controller.mode(),
            Instant::now(),
        );
    }
}

impl Drop for RewindApp {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

/// Height of a rail entry: the Python app's 13 px text plus 9 px padding above and below.
const NAV_HEIGHT: f32 = 34.0;
/// The selected-page bar: 3 x 18 points, as in the Python app.
const BAR_W: f32 = 3.0;
const BAR_H: f32 = 18.0;
/// How long the bar takes to slide between entries.
const BAR_MOVE: f32 = 0.26;

/// The gradient bar that marks the selected page, sliding between entries.
struct RailIndicator {
    /// The bar's top edge in panel coordinates; `None` until the first layout.
    y: Option<f32>,
    /// `(from, to, started)` while the bar is moving.
    moving: Option<(f32, f32, Instant)>,
}

impl RailIndicator {
    fn new() -> RailIndicator {
        RailIndicator {
            y: None,
            moving: None,
        }
    }

    /// Draw the bar beside `item`, moving towards it if the selection changed.
    /// Returns whether the move is still running.
    fn show(&mut self, ui: &egui::Ui, item: egui::Rect, now: Instant) -> bool {
        let target = item.top() + (item.height() - BAR_H) / 2.0;
        let current = match self.moving {
            Some((from, to, started)) => {
                let t = ((now - started).as_secs_f32() / BAR_MOVE).clamp(0.0, 1.0);
                if t >= 1.0 {
                    self.moving = None;
                    to
                } else {
                    let eased = 1.0 - (1.0 - t).powi(3);
                    from + (to - from) * eased
                }
            }
            None => self.y.unwrap_or(target),
        };
        // Retarget only when the destination changed, never mid-flight, or the bar
        // would restart the same move every frame and never arrive.
        let destination = self.moving.map_or(current, |(_, to, _)| to);
        if (destination - target).abs() > 0.25 {
            self.moving = Some((current, target, now));
        }
        self.y = Some(current);
        self.paint(ui, item.left() - 8.0, current);
        self.moving.is_some()
    }

    /// A capsule, three points wide, fading from the accent to white top to bottom.
    fn paint(&self, ui: &egui::Ui, x: f32, y: f32) {
        let painter = ui.painter();
        let centre = x + BAR_W / 2.0;
        let rows = BAR_H as usize;
        for row in 0..rows {
            let t = row as f32 / (rows - 1) as f32;
            let color = mix(theme::ACCENT, Color32::WHITE, t);
            let dy = (row as f32 + 0.5 - BAR_H / 2.0).abs();
            let cap = BAR_H / 2.0 - 1.5;
            let half = if dy > cap {
                (1.5 * 1.5 - (dy - cap) * (dy - cap)).max(0.0).sqrt()
            } else {
                1.5
            };
            if half <= 0.0 {
                continue;
            }
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(centre - half, y + row as f32),
                    egui::pos2(centre + half, y + row as f32 + 1.0),
                ),
                0,
                color,
            );
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

/// A full-width rail entry: left-aligned, with hover and selected states.
/// Returns whether it was clicked, and its rectangle for the selection bar.
fn nav_button(ui: &mut egui::Ui, label: &str, selected: bool) -> (bool, egui::Rect) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), NAV_HEIGHT), Sense::click());
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
    (
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked(),
        rect,
    )
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
