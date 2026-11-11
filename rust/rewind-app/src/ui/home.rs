//! The Home page: what capture is doing right now, and the controls to change it.
//!
//! It mirrors the Python dashboard — hero state, buffer bar, four summary tiles, the
//! action buttons, quick setting chips and the three most recent clips.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Vec2};

use crate::controller::{ClipEntry, Controller, Notice};
use crate::ui::format;
use crate::ui::thumbs::Thumbs;
use crate::ui::{theme, widgets, Change, PageOutput};
use rewind_engine::{EncoderKind, Mode, State, PERMISSION_NOTE};

const RECENT: usize = 3;
const REFRESH_EVERY: Duration = Duration::from_secs(2);

pub struct HomePage {
    thumbs: Thumbs,
    recent: Vec<ClipEntry>,
    last_recent: Instant,
    banner: Option<String>,
}

impl HomePage {
    pub fn new(ctx: &egui::Context) -> HomePage {
        HomePage {
            thumbs: Thumbs::new(ctx),
            recent: Vec::new(),
            last_recent: Instant::now() - REFRESH_EVERY,
            banner: None,
        }
    }

    /// Show a message in the banner (startup problems arrive this way).
    pub fn notice(&mut self, message: impl Into<String>) {
        self.banner = Some(message.into());
    }

    /// Engine and controller notices: errors become the banner, saved clips refresh the row.
    pub fn handle_notice(&mut self, notice: &Notice) {
        match notice {
            Notice::Message(text) => {
                self.banner = (!text.is_empty()).then(|| text.clone());
            }
            Notice::Failed(message) => self.banner = Some(message.clone()),
            Notice::ClipsChanged
            | Notice::ClipSaved { .. }
            | Notice::RecordingSaved { .. }
            | Notice::ScreenshotSaved(_) => {
                self.last_recent = Instant::now() - REFRESH_EVERY;
            }
            _ => {}
        }
    }

    fn refresh_recent(&mut self, controller: &Controller) {
        self.recent = controller.list_clips(RECENT);
        self.last_recent = Instant::now();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, controller: &mut Controller) -> PageOutput {
        let mut output = PageOutput::default();
        // Start the shared thumbnail frame: reset its decode budget and take in results.
        if self.thumbs.begin() {
            self.last_recent = Instant::now() - REFRESH_EVERY;
        }
        // The buffer bar and elapsed clock move in tenths.
        ui.ctx().request_repaint_after(Duration::from_millis(100));
        if self.last_recent.elapsed() >= REFRESH_EVERY {
            self.refresh_recent(controller);
        }

        let state = controller.state();
        let mode = controller.mode();
        let recording = mode == Mode::Recording;
        let replay_on = mode == Mode::Replay && matches!(state, State::On | State::Starting);
        let clip_seconds = controller.settings().clip_seconds;
        let bitrate = controller.settings().bitrate_mbps;
        let clip_hotkey = controller.settings().hotkey("clip").text;
        let toggle_hotkey = controller.settings().hotkey("toggle").text;
        let shot_hotkey = controller.settings().hotkey("screenshot").text;
        let encoder = encoder_label(controller, state);
        let buffered = controller.recorder().buffered_seconds();
        let pipeline = controller.recorder().pipeline();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.hero(
                    ui,
                    state,
                    mode,
                    replay_on,
                    recording,
                    buffered,
                    clip_seconds,
                    bitrate,
                    &pipeline,
                    &toggle_hotkey,
                    &encoder,
                    &clip_hotkey,
                    controller,
                    &mut output,
                );

                if let Some(banner) = self.banner.clone() {
                    ui.add_space(16.0);
                    self.banner_box(ui, &banner);
                }

                ui.add_space(16.0);
                self.actions(
                    ui,
                    state,
                    mode,
                    recording,
                    clip_seconds,
                    &shot_hotkey,
                    controller,
                    &mut output,
                );

                ui.add_space(16.0);
                widgets::card(ui, Some("Quick settings"), |ui| {
                    widgets::row(
                        ui,
                        "Clip length",
                        Some("How far back each clip reaches"),
                        |ui| {
                            // Added right to left: the row's control area lays out from
                            // the right edge, and the chips should read left to right.
                            for (value, label) in
                                [(60, "60s"), (45, "45s"), (30, "30s"), (15, "15s")]
                            {
                                if widgets::chip(ui, label, clip_seconds == value) {
                                    output
                                        .changes
                                        .push(Change::new("clip_seconds", i64::from(value)));
                                }
                            }
                        },
                    );
                    widgets::separator(ui);
                    widgets::row(
                        ui,
                        "Bitrate",
                        Some("Fine-tune anywhere from 5–150 Mbps on the Replay page"),
                        |ui| {
                            for (value, label) in
                                [(100, "100 Mbps"), (50, "50"), (25, "25"), (10, "10")]
                            {
                                if widgets::chip(ui, label, bitrate == value) {
                                    output
                                        .changes
                                        .push(Change::new("bitrate_mbps", i64::from(value)));
                                }
                            }
                        },
                    );
                });

                ui.add_space(16.0);
                self.recent_card(ui, &clip_hotkey);
            });

        output
    }

    #[allow(clippy::too_many_arguments)]
    fn hero(
        &mut self,
        ui: &mut egui::Ui,
        state: State,
        mode: Mode,
        replay_on: bool,
        recording: bool,
        buffered: f64,
        clip_seconds: u32,
        bitrate: u32,
        pipeline: &str,
        toggle_hotkey: &str,
        encoder: &str,
        clip_hotkey: &str,
        controller: &mut Controller,
        output: &mut PageOutput,
    ) {
        egui::Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin {
                left: 28,
                right: 28,
                top: 24,
                bottom: 24,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("INSTANT REPLAY")
                                .size(11.0)
                                .strong()
                                .color(theme::SUBTLE),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                            ui.painter().circle_filled(
                                rect.center(),
                                6.0,
                                if replay_on || recording {
                                    theme::GREEN
                                } else {
                                    theme::SUBTLE
                                },
                            );
                            ui.label(RichText::new(hero_word(state, mode)).size(30.0).strong());
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(detail_text(state, mode, pipeline, toggle_hotkey))
                                .size(11.5)
                                .color(theme::MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut on = replay_on;
                        let enabled = !recording && state != State::Saving;
                        let changed = ui
                            .add_enabled_ui(enabled, |ui| widgets::switch_large(ui, &mut on))
                            .inner;
                        if changed {
                            output.notices.extend(controller.set_replay(on));
                        }
                    });
                });

                ui.add_space(18.0);
                hero_buffer(ui, recording, buffered, clip_seconds);
                ui.add_space(18.0);
                ui.columns(4, |columns| {
                    widgets::tile(
                        &mut columns[0],
                        "CLIP LENGTH",
                        &format!("{clip_seconds} seconds"),
                    );
                    widgets::tile(&mut columns[1], "BITRATE", &format!("{bitrate} Mbps"));
                    widgets::tile(&mut columns[2], "ENCODER", encoder);
                    let hotkey = if clip_hotkey.is_empty() {
                        "Not set"
                    } else {
                        clip_hotkey
                    };
                    widgets::tile(&mut columns[3], "CLIP HOTKEY", hotkey);
                });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn actions(
        &mut self,
        ui: &mut egui::Ui,
        state: State,
        mode: Mode,
        recording: bool,
        clip_seconds: u32,
        shot_hotkey: &str,
        controller: &mut Controller,
        output: &mut PageOutput,
    ) {
        let can_save = mode == Mode::Replay && state == State::On;
        ui.horizontal(|ui| {
            let save = ui.add_enabled(
                can_save,
                theme::primary_text(format!("Save last {clip_seconds}s")),
            );
            if save.clicked() {
                output.notices.extend(controller.save_clip());
            }
            if !can_save {
                save.on_disabled_hover_text("Turn on Instant Replay first");
            }

            let record_label = if state == State::Saving {
                "Saving recording…".to_string()
            } else if recording {
                "Stop recording".to_string()
            } else {
                "Start screen recording".to_string()
            };
            if ui
                .add_enabled(state != State::Saving, egui::Button::new(record_label))
                .clicked()
            {
                output.notices.extend(controller.toggle_recording());
            }

            let shot_label = if shot_hotkey.is_empty() {
                "Screenshot".to_string()
            } else {
                format!("Screenshot  ({shot_hotkey})")
            };
            if ui.button(shot_label).clicked() {
                output.notices.extend(controller.take_screenshot());
            }

            if ui.button("Open clips folder").clicked() {
                let _ = controller.open_clips_folder();
            }
        });
    }

    fn banner_box(&self, ui: &mut egui::Ui, text: &str) {
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 181, 71, 20))
            .stroke(Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(255, 181, 71, 90),
            ))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 10,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(text).size(12.5).color(theme::WARN));
                    if text.starts_with(PERMISSION_NOTE)
                        && ui.button("Open Screen Recording settings").clicked()
                    {
                        rewind_engine::platform::open_privacy_settings();
                    }
                });
            });
    }

    fn recent_card(&mut self, ui: &mut egui::Ui, clip_hotkey: &str) {
        let recent = self.recent.clone();
        widgets::card(ui, Some("Recent clips & screenshots"), |ui| {
            ui.add_space(10.0);
            if recent.is_empty() {
                let message = if clip_hotkey.is_empty() {
                    "No clips yet. Press your save-clip hotkey while Instant Replay is on and \
                     your clip will show up here."
                        .to_string()
                } else {
                    format!(
                        "No clips yet. Press {clip_hotkey} while Instant Replay is on and your \
                         clip will show up here."
                    )
                };
                ui.label(RichText::new(message).size(12.0).color(theme::MUTED));
            } else {
                ui.horizontal(|ui| {
                    for entry in &recent {
                        recent_card(ui, entry, &mut self.thumbs);
                    }
                });
            }
            ui.add_space(8.0);
        });
    }
}

/// `Off`, `Replay On`, `Recording`, `Starting…`, `Saving…`.
fn hero_word(state: State, mode: Mode) -> &'static str {
    if mode == Mode::Recording {
        return match state {
            State::Starting => "Starting…",
            State::Saving => "Saving…",
            _ => "Recording",
        };
    }
    match state {
        State::On => "Replay On",
        State::Starting => "Starting…",
        State::Saving => "Saving…",
        State::Off => "Off",
    }
}

/// The line under the state, explaining what is happening or how to start.
fn detail_text(state: State, mode: Mode, pipeline: &str, toggle_hotkey: &str) -> String {
    match state {
        State::On if mode == Mode::Recording => format!("Screen recording  ·  {pipeline}"),
        State::On => pipeline.to_string(),
        State::Saving => "Finishing the recording and adding its audio…".to_string(),
        State::Starting if mode == Mode::Recording => "Starting screen recording…".to_string(),
        State::Starting => "Warming up the encoder…".to_string(),
        State::Off => {
            let hotkey = if toggle_hotkey.is_empty() {
                "the switch"
            } else {
                toggle_hotkey
            };
            format!("Flip the switch or press {hotkey} to start Instant Replay")
        }
    }
}

/// What the encoder tile says: what is running, then what is configured, then what was found.
fn encoder_label(controller: &Controller, state: State) -> String {
    let active = controller.recorder().active_encoder();
    if state == State::On && !active.is_empty() {
        if let Some(kind) = EncoderKind::from_key(&active) {
            return kind.label().to_string();
        }
    }
    let configured = controller.settings().encoder.clone();
    if configured != "auto" {
        if let Some(kind) = EncoderKind::from_key(&configured) {
            return kind.label().to_string();
        }
    }
    match controller.recorder().encoders() {
        Some(found) => found
            .iter()
            .find(|kind| **kind != EncoderKind::X264)
            .or_else(|| found.first())
            .map(|kind| kind.label().to_string())
            .unwrap_or_else(|| "CPU (x264)".to_string()),
        None => "Detecting…".to_string(),
    }
}

fn hero_buffer(ui: &mut egui::Ui, recording: bool, buffered: f64, clip_seconds: u32) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if recording { "Elapsed" } else { "Buffer" })
                .size(12.0)
                .color(theme::MUTED),
        );
        let value = if recording {
            format::fmt_dur(Some(buffered))
        } else {
            let have = buffered.min(f64::from(clip_seconds));
            format!(
                "{} / {}",
                format::fmt_dur(Some(have)),
                format::fmt_dur(Some(f64::from(clip_seconds)))
            )
        };
        let width = (ui.available_width() - 110.0).max(40.0);
        if recording {
            ui.add_space(width);
        } else {
            let have = buffered.min(f64::from(clip_seconds));
            let fraction = (have / f64::from(clip_seconds.max(1))) as f32;
            widgets::bar(ui, fraction, width);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).size(13.0).strong());
        });
    });
}

/// One mini card: thumbnail, name and one line of detail. Clicking opens the file.
fn recent_card(ui: &mut egui::Ui, entry: &ClipEntry, thumbs: &mut Thumbs) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(160.0, 128.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );

    if entry.kind == crate::controller::ClipKind::Video
        && (entry.thumbnail.is_none() || entry.duration.is_none())
    {
        thumbs.request(entry.path.clone());
    }
    let (thumb_rect, _) = child.allocate_exact_size(Vec2::new(160.0, 90.0), Sense::hover());
    match thumbs.texture(ui.ctx(), entry) {
        Some(texture) => {
            let source = texture.size_vec2();
            let source_aspect = source.x / source.y.max(1.0);
            let target_aspect = thumb_rect.width() / thumb_rect.height();
            let uv = if source_aspect > target_aspect {
                let height = target_aspect / source_aspect;
                egui::Rect::from_min_max(
                    egui::pos2(0.0, 0.5 - height / 2.0),
                    egui::pos2(1.0, 0.5 + height / 2.0),
                )
            } else {
                let width = source_aspect / target_aspect;
                egui::Rect::from_min_max(
                    egui::pos2(0.5 - width / 2.0, 0.0),
                    egui::pos2(0.5 + width / 2.0, 1.0),
                )
            };
            child
                .painter()
                .image(texture.id(), thumb_rect, uv, Color32::WHITE);
        }
        None => {
            child
                .painter()
                .rect_filled(thumb_rect, egui::CornerRadius::same(8), theme::CARD_2);
        }
    }

    child.add_space(6.0);
    child.add(
        egui::Label::new(
            RichText::new(
                format::title(&entry.path)
                    .trim_start_matches("Rewind_")
                    .trim_start_matches("Screenshot_")
                    .to_string(),
            )
            .size(12.0)
            .strong(),
        )
        .truncate(),
    );
    let meta = match entry.kind {
        crate::controller::ClipKind::Image => {
            format!("Screenshot  ·  {}", format::fmt_size(entry.size))
        }
        crate::controller::ClipKind::Video => format!(
            "{}  ·  {}",
            format::fmt_dur(entry.duration),
            format::fmt_size(entry.size)
        ),
    };
    child.add(egui::Label::new(RichText::new(meta).size(11.0).color(theme::MUTED)).truncate());

    if response.clicked() {
        rewind_engine::paths::open_path(&entry.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hero_names_the_capture_state() {
        assert_eq!(hero_word(State::Off, Mode::Off), "Off");
        assert_eq!(hero_word(State::On, Mode::Replay), "Replay On");
        assert_eq!(hero_word(State::On, Mode::Recording), "Recording");
        assert_eq!(hero_word(State::Starting, Mode::Recording), "Starting…");
        assert_eq!(hero_word(State::Saving, Mode::Replay), "Saving…");
    }

    #[test]
    fn the_detail_line_explains_what_to_do() {
        let idle = detail_text(State::Off, Mode::Off, "", "⌃⌥I");
        assert!(idle.contains("⌃⌥I"), "{idle}");
        // With no hotkey configured the switch is the instruction.
        let idle = detail_text(State::Off, Mode::Off, "", "");
        assert!(idle.contains("the switch"), "{idle}");

        let recording = detail_text(State::On, Mode::Recording, "VT · H264", "");
        assert!(recording.starts_with("Screen recording"), "{recording}");
        assert!(recording.contains("VT · H264"));
    }
}
