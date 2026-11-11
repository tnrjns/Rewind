//! The Clips page's modals: rename, trim and delete confirmation.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText, Vec2};

use crate::controller::ClipEntry;
use crate::ui::format::{self, fmt_dur, title};
use crate::ui::{load_texture, theme};

/// What the user chose in a modal this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancelled,
    Confirm,
}

const PREVIEW_SIDE: Vec2 = Vec2::new(246.0, 138.0);
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(180);

/// Ask for a new clip name.
pub struct RenameDialog {
    pub entry: ClipEntry,
    pub input: String,
    pub error: Option<String>,
    focus: bool,
}

impl RenameDialog {
    pub fn new(entry: &ClipEntry) -> RenameDialog {
        RenameDialog {
            entry: entry.clone(),
            input: title(&entry.path),
            error: None,
            focus: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let modal = egui::Modal::new(egui::Id::unique("rewind-rename")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Rename clip");
            ui.label(
                RichText::new(format::meta_text(&self.entry))
                    .size(12.0)
                    .color(theme::MUTED),
            );
            ui.add_space(8.0);
            let field =
                ui.add(egui::TextEdit::singleline(&mut self.input).desired_width(f32::INFINITY));
            if self.focus {
                field.request_focus();
                self.focus = false;
            }
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).size(12.0).color(theme::WARN));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancelled;
                }
                let confirm = ui.add(theme::primary("Rename"));
                if confirm.clicked() {
                    outcome = Outcome::Confirm;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    outcome = Outcome::Confirm;
                }
            });
        });
        if modal.should_close() {
            outcome = Outcome::Cancelled;
        }
        outcome
    }

    /// Rename the file and carry its metadata and thumbnail along with it.
    pub fn apply(&self) -> Result<PathBuf, String> {
        let target = format::rename_target(&self.entry.path, &self.input)?;
        let old_key = rewind_engine::paths::meta_key(&self.entry.path);
        let new_key = rewind_engine::paths::meta_key(&target);
        std::fs::rename(&self.entry.path, &target).map_err(|e| e.to_string())?;
        // The file is renamed; the metadata move is best effort, exactly like the
        // Python app's, because a lost duration is better than a half-failed rename.
        let _ = rewind_engine::meta::rename_entry(&old_key, &new_key);
        let old_thumb = rewind_engine::meta::thumbnail_for(&self.entry.path);
        if old_thumb.exists() {
            let _ = std::fs::rename(&old_thumb, rewind_engine::meta::thumbnail_for(&target));
        }
        Ok(target)
    }
}

/// Cut a clip down, keeping the original, and save the cut beside it.
pub struct TrimDialog {
    pub path: PathBuf,
    pub duration: f64,
    pub start: f64,
    pub end: f64,
    pub error: Option<String>,
    preview: Preview,
    last_edit: Instant,
}

impl TrimDialog {
    pub fn new(path: PathBuf, duration: f64) -> TrimDialog {
        let duration = duration.max(0.1);
        TrimDialog {
            path,
            duration,
            start: 0.0,
            end: duration,
            error: None,
            preview: Preview::new(),
            last_edit: Instant::now() - PREVIEW_DEBOUNCE,
        }
    }

    /// Fetch the start and end frames for the current selection.
    pub fn request_preview(&mut self) {
        self.preview.request(&self.path, self.start, self.end);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        self.preview.poll(ctx);
        let mut outcome = Outcome::Open;
        let mut moved = false;

        let modal = egui::Modal::new(egui::Id::unique("rewind-trim")).show(ctx, |ui| {
            ui.set_width(540.0);
            ui.heading("Trim clip");
            ui.label(
                RichText::new(title(&self.path))
                    .size(12.0)
                    .color(theme::MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                self.preview.frame(ui, "Start", true);
                self.preview.frame(ui, "End", false);
            });
            ui.add_space(10.0);
            ui.spacing_mut().slider_width = 340.0;
            let range = 0.0..=self.duration;
            if ui
                .add(
                    egui::Slider::new(&mut self.start, range.clone())
                        .step_by(0.1)
                        .text("Start"),
                )
                .changed()
            {
                moved = true;
            }
            if ui
                .add(
                    egui::Slider::new(&mut self.end, range)
                        .step_by(0.1)
                        .text("End"),
                )
                .changed()
            {
                moved = true;
            }

            let (from, to) = (self.start.min(self.end), self.start.max(self.end));
            ui.label(
                RichText::new(format!(
                    "{} to {}  ·  {:.1} s of {:.1} s. Cuts land on the nearest second.",
                    fmt_dur(Some(from)),
                    fmt_dur(Some(to)),
                    to - from,
                    self.duration
                ))
                .size(11.5)
                .color(theme::MUTED),
            );
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).size(12.0).color(theme::WARN));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancelled;
                }
                let long_enough = to - from >= 0.5;
                if ui
                    .add_enabled(long_enough, theme::primary("Save as new clip"))
                    .clicked()
                {
                    outcome = Outcome::Confirm;
                }
            });
        });
        if modal.should_close() {
            outcome = Outcome::Cancelled;
        }

        if moved {
            self.last_edit = Instant::now();
            ctx.request_repaint_after(PREVIEW_DEBOUNCE);
        } else if self.last_edit.elapsed() >= PREVIEW_DEBOUNCE
            && self.preview.wants(self.start, self.end)
        {
            self.request_preview();
        }
        outcome
    }
}

/// The start and end frame previews, extracted by FFmpeg off the UI thread.
struct Preview {
    start: Option<egui::TextureHandle>,
    end: Option<egui::TextureHandle>,
    pending: Option<Receiver<(f64, f64)>>,
    requested: Option<(f64, f64)>,
    dir: PathBuf,
}

impl Preview {
    fn new() -> Preview {
        Preview {
            start: None,
            end: None,
            pending: None,
            requested: None,
            dir: std::env::temp_dir().join(format!("rewind-trim-{}", std::process::id())),
        }
    }

    fn wants(&self, start: f64, end: f64) -> bool {
        self.requested != Some((start, end))
    }

    fn request(&mut self, path: &Path, start: f64, end: f64) {
        if self.pending.is_some() || !self.wants(start, end) {
            return;
        }
        self.requested = Some((start, end));
        let dir = self.dir.clone();
        let path = path.to_path_buf();
        let (tx, rx) = channel();
        self.pending = Some(rx);
        std::thread::Builder::new()
            .name("rewind-trim-frames".into())
            .spawn(move || {
                if let Some(ffmpeg) = rewind_engine::find_ffmpeg() {
                    rewind_engine::meta::frame_at(&ffmpeg, &path, start, &dir.join("start.png"));
                    rewind_engine::meta::frame_at(
                        &ffmpeg,
                        &path,
                        (end - 0.05).max(0.0),
                        &dir.join("end.png"),
                    );
                }
                let _ = tx.send((start, end));
            })
            .ok();
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else {
            return;
        };
        match pending.try_recv() {
            Ok((start, end)) => {
                self.pending = None;
                self.start = load_texture(ctx, "trim-start", &self.dir.join("start.png"));
                self.end = load_texture(ctx, "trim-end", &self.dir.join("end.png"));
                self.requested = Some((start, end));
                ctx.request_repaint();
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(120));
            }
        }
    }

    fn frame(&self, ui: &mut egui::Ui, label: &str, start: bool) {
        ui.vertical(|ui| {
            let frame = egui::Frame::new()
                .fill(theme::CARD_2)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(4.0);
            frame.show(ui, |ui| {
                let texture = if start { &self.start } else { &self.end };
                match texture {
                    Some(texture) => {
                        ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(
                                texture.id(),
                                PREVIEW_SIDE,
                            ))
                            .fit_to_exact_size(PREVIEW_SIDE),
                        );
                    }
                    None => {
                        ui.allocate_ui(PREVIEW_SIDE, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.label(RichText::new("…").size(18.0).color(theme::SUBTLE));
                            });
                        });
                    }
                }
            });
            ui.label(RichText::new(label).size(11.5).color(theme::MUTED));
        });
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Move files to the Trash / Recycle Bin, after asking.
pub struct ConfirmDelete {
    pub paths: Vec<PathBuf>,
    pub error: Option<String>,
}

impl ConfirmDelete {
    pub fn new(paths: Vec<PathBuf>) -> ConfirmDelete {
        ConfirmDelete { paths, error: None }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let what = if self.paths.len() == 1 {
            "this file".to_string()
        } else {
            format!("these {} files", self.paths.len())
        };
        let modal = egui::Modal::new(egui::Id::unique("rewind-delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Delete");
            ui.label(format!(
                "Move {what} to the {}?",
                super::clips::trash_name()
            ));
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).size(12.0).color(theme::WARN));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Cancelled;
                }
                if ui.add(theme::primary("Delete")).clicked() {
                    outcome = Outcome::Confirm;
                }
            });
        });
        if modal.should_close() {
            outcome = Outcome::Cancelled;
        }
        outcome
    }

    /// Returns how many files were moved.
    pub fn apply(&self) -> Result<usize, String> {
        trash::delete_all(&self.paths).map_err(|e| e.to_string())?;
        Ok(self.paths.len())
    }
}
