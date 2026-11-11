//! The Clips page: a grid of everything in the clips folder, with the actions the
//! Python app offered on each one.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, Vec2};

use crate::clipboard;
use crate::controller::{ClipEntry, ClipKind, Controller, Notice};
use crate::ui::dialogs::{ConfirmDelete, Outcome, RenameDialog, TrimDialog};
use crate::ui::format::{self, Filter};
use crate::ui::{load_texture, theme};

const CELL_W: f32 = 240.0;
const THUMB_H: f32 = 126.0;
const CELL_H: f32 = 190.0;
const GAP: f32 = 14.0;
/// How often the folder is re-listed. Cheap (one `read_dir`), and it is what makes a
/// clip saved by a hotkey appear while the window is open.
const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// Thumbnail JPEGs decoded per frame, so opening the page on a large library does not
/// freeze the window for a second.
const DECODE_BUDGET: usize = 4;
const TOAST_FOR: Duration = Duration::from_secs(5);

/// The one modal that can be open at a time.
enum Dialog {
    Rename(RenameDialog),
    Trim(TrimDialog),
    Delete(ConfirmDelete),
}

struct Thumb {
    texture: egui::TextureHandle,
    stamp: Option<SystemTime>,
}

/// A context-menu choice, collected during painting and applied afterwards so the grid
/// is not mutated while it is being drawn.
enum MenuRequest {
    Open(PathBuf),
    Reveal(PathBuf),
    Copy(Vec<PathBuf>),
    Trim(PathBuf),
    Rename(PathBuf),
    Delete(Vec<PathBuf>),
}

pub struct ClipsPage {
    filter: Filter,
    visible: Vec<ClipEntry>,
    selected: Vec<PathBuf>,
    anchor: Option<usize>,
    textures: HashMap<PathBuf, Thumb>,
    thumb_jobs: ThumbJobs,
    failed_thumbs: HashSet<PathBuf>,
    dialog: Option<Dialog>,
    notices: VecDeque<(String, Instant)>,
    last_refresh: Instant,
    stale: bool,
}

impl ClipsPage {
    pub fn new(ctx: &egui::Context) -> ClipsPage {
        ClipsPage {
            filter: Filter::All,
            visible: Vec::new(),
            selected: Vec::new(),
            anchor: None,
            textures: HashMap::new(),
            thumb_jobs: ThumbJobs::start(ctx.clone()),
            failed_thumbs: HashSet::new(),
            dialog: None,
            notices: VecDeque::new(),
            last_refresh: Instant::now() - REFRESH_EVERY,
            stale: true,
        }
    }

    /// Note something the user should see, such as a failed copy.
    pub fn notice(&mut self, message: impl Into<String>) {
        self.notices.push_front((message.into(), Instant::now()));
        self.notices.truncate(3);
    }

    /// React to an engine or controller notice.
    pub fn handle_notice(&mut self, notice: &Notice) {
        match notice {
            Notice::ClipsChanged
            | Notice::ClipSaved { .. }
            | Notice::RecordingSaved { .. }
            | Notice::ScreenshotSaved(_) => self.stale = true,
            Notice::Failed(message) => self.notice(message.clone()),
            Notice::Message(text) if !text.is_empty() => self.notice(text.clone()),
            _ => {}
        }
    }

    /// Re-list the folder and keep the selection to files that still exist.
    pub fn refresh(&mut self, controller: &Controller) {
        let entries = controller.list_clips(400);
        let alive: HashSet<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
        self.selected.retain(|p| alive.contains(p));
        self.visible = entries
            .into_iter()
            .filter(|e| self.filter.allows(e.kind))
            .collect();
        self.last_refresh = Instant::now();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, controller: &mut Controller) {
        if self.stale || self.last_refresh.elapsed() >= REFRESH_EVERY {
            self.refresh(controller);
            self.stale = false;
        }
        self.drain_thumb_jobs();
        self.expire_notices();

        self.toolbar(ui, controller);
        ui.add_space(2.0);

        let footer_height = 58.0;
        let grid_height = (ui.available_height() - footer_height).max(120.0);
        self.grid(ui, controller, grid_height);

        ui.add_space(4.0);
        self.footer(ui, controller);
        self.show_dialog(ui.ctx(), controller);

        if !self.notices.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, controller: &mut Controller) {
        ui.horizontal(|ui| {
            for filter in Filter::ALL {
                let selected = self.filter == filter;
                let (fill, text) = if selected {
                    (theme::ACCENT, Color32::BLACK)
                } else {
                    (theme::CARD_2, theme::TEXT)
                };
                let button = egui::Button::new(egui::RichText::new(filter.label()).color(text))
                    .fill(fill)
                    .corner_radius(egui::CornerRadius::same(20));
                if ui.add(button).clicked() && !selected {
                    self.filter = filter;
                    self.stale = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Open folder").clicked() {
                    let _ = controller.open_clips_folder();
                }
                if ui.button("Refresh").clicked() {
                    self.refresh(controller);
                }
            });
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, controller: &mut Controller, height: f32) {
        if self.visible.is_empty() {
            self.empty_state(ui);
            return;
        }

        let mut clicked: Option<(usize, bool, bool)> = None;
        let mut double: Option<usize> = None;
        let mut requested: Option<MenuRequest> = None;
        let mut budget = DECODE_BUDGET;
        let mut queued = Vec::new();

        {
            let Self {
                visible,
                selected,
                textures,
                failed_thumbs,
                ..
            } = self;
            let columns = (((ui.available_width() + GAP) / (CELL_W + GAP)).floor() as usize).max(1);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(height)
                .show(ui, |ui| {
                    for (row_number, row) in visible.chunks(columns).enumerate() {
                        ui.horizontal(|ui| {
                            for (column, entry) in row.iter().enumerate() {
                                let index = row_number * columns + column;
                                let (rect, response) = ui
                                    .allocate_exact_size(Vec2::new(CELL_W, CELL_H), Sense::click());
                                if entry.kind == ClipKind::Video
                                    && (entry.thumbnail.is_none() || entry.duration.is_none())
                                    && !failed_thumbs.contains(&entry.path)
                                {
                                    queued.push(entry.path.clone());
                                }
                                paint_cell(
                                    ui,
                                    rect,
                                    entry,
                                    selected.contains(&entry.path),
                                    textures,
                                    &mut budget,
                                );

                                if response.clicked() {
                                    let modifiers = ui.input(|i| i.modifiers);
                                    clicked = Some((
                                        index,
                                        modifiers.command || modifiers.ctrl,
                                        modifiers.shift,
                                    ));
                                }
                                if response.secondary_clicked() {
                                    clicked = Some((index, false, false));
                                }
                                if response.double_clicked() {
                                    double = Some(index);
                                }

                                // Built inside the popup, not per frame: cloning a large
                                // selection for every visible cell would allocate on
                                // every repaint for a menu that is rarely open.
                                let is_selected = selected.contains(&entry.path);
                                let path = entry.path.clone();
                                let kind = entry.kind;
                                response.context_menu(|ui| {
                                    let targets = if is_selected {
                                        selected.clone()
                                    } else {
                                        vec![path.clone()]
                                    };
                                    let mut chosen: Option<MenuRequest> = None;
                                    if ui.button("Open").clicked() {
                                        chosen = Some(MenuRequest::Open(path.clone()));
                                    }
                                    let reveal = if cfg!(target_os = "macos") {
                                        "Show in Finder"
                                    } else {
                                        "Show in folder"
                                    };
                                    if ui.button(reveal).clicked() {
                                        chosen = Some(MenuRequest::Reveal(path.clone()));
                                    }
                                    if ui.button("Copy (paste into Discord etc.)").clicked() {
                                        chosen = Some(MenuRequest::Copy(targets.clone()));
                                    }
                                    if targets.len() == 1 {
                                        if kind == ClipKind::Video && ui.button("Trim…").clicked()
                                        {
                                            chosen = Some(MenuRequest::Trim(path.clone()));
                                        }
                                        if ui.button("Rename…").clicked() {
                                            chosen = Some(MenuRequest::Rename(path.clone()));
                                        }
                                    }
                                    ui.separator();
                                    let label = if targets.len() == 1 {
                                        "Delete".to_string()
                                    } else {
                                        format!("Delete {} items", targets.len())
                                    };
                                    if ui.button(label).clicked() {
                                        chosen = Some(MenuRequest::Delete(targets.clone()));
                                    }
                                    if let Some(chosen) = chosen {
                                        requested = Some(chosen);
                                        ui.close();
                                    }
                                });
                            }
                        });
                        ui.add_space(GAP);
                    }
                });
        }

        for path in queued {
            self.thumb_jobs.request(path);
        }
        if let Some((index, toggle, range)) = clicked {
            self.click(index, toggle, range);
        }
        if let Some(index) = double {
            if let Some(entry) = self.visible.get(index) {
                rewind_engine::paths::open_path(&entry.path);
            }
        }
        if let Some(request) = requested {
            self.apply(request, controller);
        }
    }

    fn empty_state(&self, ui: &mut egui::Ui) {
        let text = if self.filter == Filter::All {
            "No clips yet.\nPress your save-clip hotkey and they will appear here."
        } else {
            "Nothing here matches this filter."
        };
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(
                egui::RichText::new(text)
                    .size(15.0)
                    .color(theme::MUTED)
                    .line_height(Some(22.0)),
            );
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui, controller: &Controller) {
        if let Some((message, _)) = self.notices.front() {
            ui.label(egui::RichText::new(message).size(12.5).color(theme::WARN));
        }
        let count = self.visible.len();
        let size = format::fmt_size(format::total_size(&self.visible));
        let folder = controller.settings().output_dir.display().to_string();
        ui.label(
            egui::RichText::new(format!(
                "{count} item{}  ·  {size}  ·  {folder}",
                if count == 1 { "" } else { "s" }
            ))
            .size(12.5)
            .color(theme::MUTED),
        );
        ui.label(
            egui::RichText::new(
                "Double-click to open  ·  Right-click to trim, copy (paste straight into Discord), rename or delete",
            )
            .size(12.0)
            .color(theme::SUBTLE),
        );
    }

    fn click(&mut self, index: usize, toggle: bool, range: bool) {
        if index >= self.visible.len() {
            return;
        }
        let path = self.visible[index].path.clone();
        if range {
            if let Some(anchor) = self.anchor {
                let (from, to) = if anchor <= index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                self.selected = self.visible[from..=to]
                    .iter()
                    .map(|e| e.path.clone())
                    .collect();
                return;
            }
        }
        if toggle {
            if let Some(position) = self.selected.iter().position(|p| *p == path) {
                self.selected.remove(position);
            } else {
                self.selected.push(path);
            }
        } else if self.selected.len() != 1 || self.selected[0] != path {
            self.selected = vec![path];
        }
        self.anchor = Some(index);
    }

    fn apply(&mut self, request: MenuRequest, controller: &mut Controller) {
        match request {
            MenuRequest::Open(path) => rewind_engine::paths::open_path(&path),
            MenuRequest::Reveal(path) => rewind_engine::paths::reveal_path(&path),
            MenuRequest::Copy(paths) => {
                let existing = clipboard::existing(&paths);
                match clipboard::copy_files(&existing) {
                    Ok(()) => self.notice(format!("Copied {}", clipboard::names(&existing))),
                    Err(message) => self.notice(message),
                }
            }
            MenuRequest::Trim(path) => {
                let duration = self
                    .visible
                    .iter()
                    .find(|e| e.path == path)
                    .and_then(|e| e.duration)
                    .or_else(|| controller.clip_duration(&path))
                    .unwrap_or(0.0);
                if duration <= 0.0 {
                    self.notice("Couldn't read that clip's duration");
                    return;
                }
                let mut dialog = TrimDialog::new(path, duration);
                dialog.request_preview();
                self.dialog = Some(Dialog::Trim(dialog));
            }
            MenuRequest::Rename(path) => {
                if let Some(entry) = self.visible.iter().find(|e| e.path == path) {
                    self.dialog = Some(Dialog::Rename(RenameDialog::new(entry)));
                }
            }
            MenuRequest::Delete(paths) => {
                self.dialog = Some(Dialog::Delete(ConfirmDelete::new(paths)));
            }
        }
    }

    fn show_dialog(&mut self, ctx: &egui::Context, controller: &mut Controller) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let outcome = match &mut dialog {
            Dialog::Rename(rename) => rename.show(ctx),
            Dialog::Trim(trim) => trim.show(ctx),
            Dialog::Delete(delete) => delete.show(ctx),
        };
        match outcome {
            Outcome::Open => self.dialog = Some(dialog),
            Outcome::Cancelled => {}
            Outcome::Confirm => match dialog {
                Dialog::Rename(mut rename) => match rename.apply() {
                    Ok(path) => {
                        self.notice(format!("Renamed to {}", format::title(&path)));
                        self.stale = true;
                    }
                    Err(message) => {
                        rename.error = Some(message);
                        self.dialog = Some(Dialog::Rename(rename));
                    }
                },
                Dialog::Trim(trim) => {
                    let (start, end) = (trim.start.min(trim.end), trim.start.max(trim.end));
                    let notices = controller.trim(&trim.path, start, end);
                    for notice in &notices {
                        self.handle_notice(notice);
                    }
                    self.stale = true;
                }
                Dialog::Delete(mut delete) => match delete.apply() {
                    Ok(removed) => {
                        self.notice(format!(
                            "Moved {removed} file{} to the {}",
                            if removed == 1 { "" } else { "s" },
                            trash_name()
                        ));
                        self.selected.clear();
                        self.stale = true;
                    }
                    Err(message) => {
                        delete.error = Some(message);
                        self.dialog = Some(Dialog::Delete(delete));
                    }
                },
            },
        }
    }

    fn drain_thumb_jobs(&mut self) {
        for (video, thumbnail) in self.thumb_jobs.drain() {
            match thumbnail {
                // The thumbnail is on disk now, so re-list and decode it next pass.
                Some(_) => self.stale = true,
                None => {
                    self.failed_thumbs.insert(video);
                }
            }
        }
    }

    fn expire_notices(&mut self) {
        let now = Instant::now();
        self.notices
            .retain(|(_, at)| now.duration_since(*at) < TOAST_FOR);
    }
}

/// Where deleted files go, for the confirmation and the result message.
pub fn trash_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Trash"
    } else if cfg!(target_os = "windows") {
        "Recycle Bin"
    } else {
        "trash"
    }
}

// ---------------------------------------------------------------- painting

fn paint_cell(
    ui: &egui::Ui,
    rect: Rect,
    entry: &ClipEntry,
    selected: bool,
    textures: &mut HashMap<PathBuf, Thumb>,
    budget: &mut usize,
) {
    let painter = ui.painter();
    theme::card(
        painter,
        rect,
        if selected { theme::CARD_2 } else { theme::CARD },
        Some(if selected {
            theme::ACCENT
        } else {
            theme::BORDER
        }),
    );

    let thumb_rect = Rect::from_min_size(
        rect.min + Vec2::new(8.0, 8.0),
        Vec2::new(CELL_W - 16.0, THUMB_H),
    );

    match thumbnail_texture(ui.ctx(), entry, textures, budget) {
        Some(texture) => {
            let source = texture.size_vec2();
            let source_aspect = source.x / source.y.max(1.0);
            let target_aspect = thumb_rect.width() / thumb_rect.height();
            let uv = if source_aspect > target_aspect {
                let height = target_aspect / source_aspect;
                Rect::from_min_max(
                    egui::pos2(0.0, 0.5 - height / 2.0),
                    egui::pos2(1.0, 0.5 + height / 2.0),
                )
            } else {
                let width = source_aspect / target_aspect;
                Rect::from_min_max(
                    egui::pos2(0.5 - width / 2.0, 0.0),
                    egui::pos2(0.5 + width / 2.0, 1.0),
                )
            };
            painter.image(texture.id(), thumb_rect, uv, Color32::WHITE);
        }
        None => {
            painter.rect_filled(thumb_rect, egui::CornerRadius::same(8), theme::CARD_2);
            painter.text(
                thumb_rect.center(),
                Align2::CENTER_CENTER,
                "◀◀",
                FontId::proportional(22.0),
                Color32::from_rgb(0x66, 0x66, 0x66),
            );
        }
    }

    if entry.kind == ClipKind::Video {
        let centre = egui::pos2(thumb_rect.right() - 22.0, thumb_rect.bottom() - 22.0);
        painter.circle_filled(centre, 13.0, Color32::from_black_alpha(150));
        painter.add(egui::Shape::convex_polygon(
            vec![
                centre + Vec2::new(5.0, 0.0),
                centre + Vec2::new(-3.5, -6.0),
                centre + Vec2::new(-3.5, 6.0),
            ],
            Color32::WHITE,
            Stroke::NONE,
        ));
    }

    let text_width = CELL_W - 24.0;
    let name = elide(
        ui,
        &format::title(&entry.path),
        FontId::proportional(13.5),
        text_width,
    );
    painter.text(
        egui::pos2(rect.left() + 12.0, thumb_rect.bottom() + 9.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(13.5),
        theme::TEXT,
    );
    let meta = elide(
        ui,
        &format::meta_text(entry),
        FontId::proportional(11.5),
        text_width,
    );
    painter.text(
        egui::pos2(rect.left() + 12.0, thumb_rect.bottom() + 30.0),
        Align2::LEFT_TOP,
        meta,
        FontId::proportional(11.5),
        theme::MUTED,
    );
}

/// Shrink text with an ellipsis until it fits `max_width`.
fn elide(ui: &egui::Ui, text: &str, font: FontId, max_width: f32) -> String {
    fn fits(ui: &egui::Ui, text: &str, font: &FontId, max_width: f32) -> bool {
        ui.painter()
            .layout_no_wrap(text.to_string(), font.clone(), theme::TEXT)
            .size()
            .x
            <= max_width
    }
    if fits(ui, text, &font, max_width) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low + 1 < high {
        let mid = (low + high) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(ui, &candidate, &font, max_width) {
            low = mid;
        } else {
            high = mid;
        }
    }
    chars[..low].iter().collect::<String>() + "…"
}

/// The cached texture for a clip, decoding it when it is new or has changed.
fn thumbnail_texture(
    ctx: &egui::Context,
    entry: &ClipEntry,
    textures: &mut HashMap<PathBuf, Thumb>,
    budget: &mut usize,
) -> Option<egui::TextureHandle> {
    let path = entry.thumbnail.as_ref()?;
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some(cached) = textures.get(path) {
        if cached.stamp == stamp {
            return Some(cached.texture.clone());
        }
    }
    if *budget == 0 {
        return textures.get(path).map(|t| t.texture.clone());
    }
    *budget -= 1;
    let pointer = std::path::Path::new(path);
    let texture = load_texture(ctx, &format!("thumb:{}", path.display()), pointer)?;
    textures.insert(
        path.clone(),
        Thumb {
            texture: texture.clone(),
            stamp,
        },
    );
    Some(texture)
}

// ---------------------------------------------------------------- thumbnails

/// Generates missing thumbnails off the UI thread.
///
/// Older clips saved before the thumbnail existed (or screenshots whose thumbnail was
/// deleted) would otherwise stay blank forever. One worker thread keeps the FFmpeg
/// spawns serial, which is plenty for a library that is mostly already done.
struct ThumbJobs {
    jobs: Sender<PathBuf>,
    done: Receiver<(PathBuf, Option<PathBuf>)>,
    inflight: HashSet<PathBuf>,
}

impl ThumbJobs {
    fn start(ctx: egui::Context) -> ThumbJobs {
        let (jobs, job_rx) = channel::<PathBuf>();
        let (done, done_rx) = channel();
        std::thread::Builder::new()
            .name("rewind-thumbs".into())
            .spawn(move || {
                let ffmpeg = rewind_engine::find_ffmpeg();
                for video in job_rx {
                    let thumbnail = ffmpeg.as_ref().and_then(|ffmpeg| {
                        // Probe and record the duration here, as the Python worker did:
                        // it is the only place a clip's length is available before the
                        // user opens it, and the grid shows it.
                        let key = rewind_engine::paths::meta_key(&video);
                        let duration = rewind_engine::meta::meta_duration(&key).or_else(|| {
                            let probed = rewind_engine::ffmpeg::probe_duration(ffmpeg, &video)?;
                            let _ = rewind_engine::meta::update_meta(
                                &key,
                                &[(
                                    "duration",
                                    serde_json::json!((probed * 100.0).round() / 100.0),
                                )],
                            );
                            Some(probed)
                        });
                        rewind_engine::meta::make_thumbnail(ffmpeg, &video, duration)
                    });
                    let _ = done.send((video, thumbnail));
                    ctx.request_repaint();
                }
            })
            .ok();
        ThumbJobs {
            jobs,
            done: done_rx,
            inflight: HashSet::new(),
        }
    }

    fn request(&mut self, video: PathBuf) {
        if self.inflight.insert(video.clone()) {
            let _ = self.jobs.send(video);
        }
    }

    fn drain(&mut self) -> Vec<(PathBuf, Option<PathBuf>)> {
        let results: Vec<_> = self.done.try_iter().collect();
        for (video, _) in &results {
            self.inflight.remove(video);
        }
        results
    }
}
