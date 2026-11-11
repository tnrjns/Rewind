//! The Recording page: a live monitor preview and start/stop.
//!
//! The Python page grabbed the screen in-process four times a second. Here each frame is
//! an FFmpeg grab, so it refreshes a little slower — spawning a process four times a
//! second to draw a thumbnail would cost more than the recording it previews.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Vec2};

use crate::controller::Controller;
use crate::ui::{load_texture, theme, PageOutput};
use rewind_engine::{Mode, State};

const PREVIEW_EVERY: Duration = Duration::from_millis(800);
const PREVIEW_HEIGHT: f32 = 400.0;

pub struct RecordingPage {
    preview: Preview,
}

impl RecordingPage {
    pub const SUBTITLE: &'static str = "Screen capture with a live monitor preview.";

    pub fn new() -> RecordingPage {
        RecordingPage {
            preview: Preview {
                texture: None,
                pending: None,
                last: Instant::now() - PREVIEW_EVERY,
                path: std::env::temp_dir()
                    .join(format!("rewind-preview-{}.png", std::process::id())),
            },
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, controller: &mut Controller) -> PageOutput {
        let mut output = PageOutput::default();
        let state = controller.state();
        let recording = controller.mode() == Mode::Recording;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(status_word(recording, state))
                    .size(11.0)
                    .strong()
                    .color(theme::SUBTLE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let seconds = if recording {
                    controller.recorder().buffered_seconds()
                } else {
                    0.0
                };
                ui.label(
                    RichText::new(clock(seconds))
                        .size(13.0)
                        .font(FontId::monospace(13.0)),
                );
            });
        });
        ui.add_space(10.0);

        self.preview.keep_fresh(ui.ctx(), controller);
        self.preview.draw(ui);

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let label = if recording {
                "Stop recording"
            } else {
                "Start recording"
            };
            let enabled = state != State::Saving;
            if ui.add_enabled(enabled, theme::primary(label)).clicked() {
                output.notices = controller.toggle_recording();
            }
            let detail = if recording {
                controller.recorder().pipeline()
            } else {
                "Preview follows the selected monitor in Replay settings.".to_string()
            };
            ui.label(RichText::new(detail).size(12.0).color(theme::MUTED));
        });

        output
    }
}

impl Default for RecordingPage {
    fn default() -> Self {
        Self::new()
    }
}

/// `READY`, `RECORDING`, `STARTING` or `SAVING`, as the Python page labelled it.
fn status_word(recording: bool, state: State) -> &'static str {
    match (recording, state) {
        (true, State::Starting) => "STARTING",
        (true, _) => "RECORDING",
        (false, State::Saving) => "SAVING",
        _ => "READY",
    }
}

/// `00:42`.
fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

struct Preview {
    texture: Option<egui::TextureHandle>,
    pending: Option<Receiver<Option<PathBuf>>>,
    last: Instant,
    path: PathBuf,
}

impl Preview {
    /// Ask for a fresh frame when the last one has answered and is old enough.
    fn keep_fresh(&mut self, ctx: &egui::Context, controller: &Controller) {
        if let Some(pending) = &self.pending {
            if let Ok(result) = pending.try_recv() {
                self.pending = None;
                if let Some(path) = result {
                    self.texture = load_texture(ctx, "recording-preview", &path);
                }
            }
        }
        ctx.request_repaint_after(PREVIEW_EVERY);
        if self.pending.is_some() || self.last.elapsed() < PREVIEW_EVERY {
            return;
        }
        let Some(ffmpeg) = controller
            .recorder()
            .ffmpeg_path()
            .or_else(rewind_engine::find_ffmpeg)
        else {
            return;
        };
        let monitors = controller.recorder().monitors();
        let monitor =
            rewind_engine::platform::clamp_monitor(controller.settings().monitor, &monitors);
        let cursor = controller.settings().capture_cursor;
        let out = self.path.clone();
        self.last = Instant::now();
        let ctx = ctx.clone();
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("rewind-preview".into())
            .spawn(move || {
                let ok = rewind_engine::screenshot::take_screenshot(
                    &ffmpeg, monitor, &out, cursor, false,
                );
                let _ = tx.send(ok.then_some(out));
                ctx.request_repaint();
            })
            .ok();
        self.pending = Some(rx);
    }

    fn draw(&self, ui: &mut egui::Ui) {
        let height = PREVIEW_HEIGHT.min(ui.available_height() - 70.0).max(160.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        theme::card(ui.painter(), rect, theme::CARD, Some(theme::BORDER));

        let Some(texture) = &self.texture else {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Waiting for screen preview…",
                FontId::proportional(13.0),
                theme::MUTED,
            );
            return;
        };
        let source = texture.size_vec2();
        let scale = (rect.width() / source.x).min(rect.height() / source.y);
        let size = source * scale;
        let fitted = egui::Rect::from_center_size(rect.center(), size);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(texture.id(), fitted, uv, Color32::WHITE);
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_word_names_what_capture_is_doing() {
        assert_eq!(status_word(false, State::Off), "READY");
        assert_eq!(status_word(true, State::Starting), "STARTING");
        assert_eq!(status_word(true, State::On), "RECORDING");
        assert_eq!(status_word(false, State::Saving), "SAVING");
    }

    #[test]
    fn the_clock_counts_minutes_and_seconds() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(42.9), "00:42");
        assert_eq!(clock(61.0), "01:01");
        assert_eq!(clock(-5.0), "00:00");
    }
}
