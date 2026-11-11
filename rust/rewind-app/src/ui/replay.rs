//! The Replay page: how much is kept, and how good it looks.
//!
//! Every control writes one setting through [`Change`]; the shell applies it and owns
//! the restart debounce, because a restart drops the buffered seconds.

use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, RichText};

use crate::controller::Controller;
use crate::ui::Change;
use crate::ui::{theme, widgets};

use rewind_engine::{EncoderKind, Mode, Monitor};

const FPS: [(&str, u32); 4] = [
    ("30 fps", 30),
    ("60 fps", 60),
    ("120 fps", 120),
    ("144 fps", 144),
];

const RESOLUTIONS: [(&str, &str); 4] = [
    ("Native", "native"),
    ("1440p", "1440"),
    ("1080p", "1080"),
    ("720p", "720"),
];

pub struct ReplayPage {
    monitors: Vec<Monitor>,
    microphones: Vec<String>,
    encoders: Option<Vec<EncoderKind>>,
    probing: Option<Receiver<Vec<EncoderKind>>>,
    loaded: bool,
}

impl ReplayPage {
    pub const SUBTITLE: &'static str = "How much gets kept and how good it looks. Changes \
         apply instantly (the buffer restarts if the capture settings change).";

    pub fn new() -> ReplayPage {
        ReplayPage {
            monitors: Vec::new(),
            microphones: Vec::new(),
            encoders: None,
            probing: None,
            loaded: false,
        }
    }

    /// Fetch what the page needs once: displays, inputs, and the encoder probes.
    ///
    /// Probing runs FFmpeg three times, so it goes on a thread; the page shows
    /// "Detecting…" until it answers.
    fn ensure_loaded(&mut self, ctx: &egui::Context, controller: &Controller) {
        if !self.loaded {
            self.loaded = true;
            self.monitors = controller.recorder().monitors();
            self.microphones = rewind_engine::list_microphones();
        }
        if self.encoders.is_none() {
            if let Some(found) = controller.recorder().encoders() {
                self.encoders = Some(found);
                return;
            }
            if let Some(results) = &self.probing {
                if let Ok(found) = results.try_recv() {
                    self.encoders = Some(found);
                    self.probing = None;
                }
                return;
            }
            let recorder = controller.recorder();
            let ctx = ctx.clone();
            let (tx, rx) = channel();
            std::thread::Builder::new()
                .name("rewind-encoders".into())
                .spawn(move || {
                    let found = recorder.detect_encoders();
                    let _ = tx.send(found);
                    ctx.request_repaint();
                })
                .ok();
            self.probing = Some(rx);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, controller: &mut Controller) -> Vec<Change> {
        self.ensure_loaded(ui.ctx(), controller);
        let mut changes = Vec::new();
        let settings = controller.settings();

        if controller.mode() == Mode::Recording {
            ui.label(
                RichText::new(
                    "Full recording is active. Encoder, quality, monitor and audio-source \
                     changes apply after it stops; the save folder applies when the file is saved.",
                )
                .size(11.5)
                .color(theme::WARN),
            );
            ui.add_space(12.0);
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.clip_card(ui, settings, &mut changes);
                ui.add_space(16.0);
                self.quality_card(ui, settings, &mut changes);
                ui.add_space(16.0);
                self.capture_card(ui, settings, &mut changes);
                ui.add_space(16.0);
                self.audio_card(ui, settings, &mut changes);
                ui.add_space(16.0);
                self.saving_card(ui, controller, settings, &mut changes);
            });

        changes
    }

    fn clip_card(
        &self,
        ui: &mut egui::Ui,
        settings: &rewind_engine::Settings,
        changes: &mut Vec<Change>,
    ) {
        widgets::card(ui, Some("Clip"), |ui| {
            let mut seconds = settings.clip_seconds as i64;
            widgets::row(
                ui,
                "Replay length",
                Some("Save anywhere from the last 15 seconds up to a full minute"),
                |ui| {
                    if widgets::slider(ui, &mut seconds, (15, 60), 5, " seconds") {
                        changes.push(Change::new("clip_seconds", seconds));
                    }
                },
            );
            widgets::separator(ui);
            let mut copy = settings.copy_after_save;
            widgets::row(
                ui,
                "Copy each clip when it saves",
                Some("Paste it straight into Discord or a chat without opening the folder"),
                |ui| {
                    if widgets::switch(ui, &mut copy) {
                        changes.push(Change::new("copy_after_save", copy));
                    }
                },
            );
        });
    }

    fn quality_card(
        &self,
        ui: &mut egui::Ui,
        settings: &rewind_engine::Settings,
        changes: &mut Vec<Change>,
    ) {
        widgets::card(ui, Some("Quality"), |ui| {
            let mut rate = settings.bitrate_mbps as i64;
            let mb = estimate_mb(settings.bitrate_mbps, settings.clip_seconds);
            let bitrate_sub = format!(
                "≈ {mb:.0} MB per {}s clip. 25–50 Mbps looks great at 1080p60; use 80+ for 1440p/4K",
                settings.clip_seconds
            );
            widgets::row(ui, "Bitrate", Some(&bitrate_sub), |ui| {
                if widgets::slider(ui, &mut rate, (5, 150), 5, " Mbps") {
                    changes.push(Change::new("bitrate_mbps", rate));
                }
            });
            widgets::separator(ui);

            let fps_labels: Vec<String> =
                FPS.iter().map(|(label, _)| (*label).to_string()).collect();
            let fps_now = FPS
                .iter()
                .position(|(_, value)| *value == settings.fps)
                .unwrap_or(1);
            widgets::row(
                ui,
                "Frame rate",
                Some("Higher frame rates need more bitrate to stay sharp"),
                |ui| {
                    if let Some(index) = widgets::combo(ui, "fps", &fps_labels, fps_now) {
                        changes.push(Change::new("fps", FPS[index].1 as i64));
                    }
                },
            );
            widgets::separator(ui);

            let resolution_labels: Vec<String> = RESOLUTIONS
                .iter()
                .map(|(label, _)| (*label).to_string())
                .collect();
            let resolution_key = settings.resolution.as_str();
            let resolution_now = resolution_index(&resolution_key);
            widgets::row(
                ui,
                "Resolution",
                Some("Downscaling uses a little extra CPU"),
                |ui| {
                    if let Some(index) =
                        widgets::combo(ui, "resolution", &resolution_labels, resolution_now)
                    {
                        changes.push(Change::new("resolution", RESOLUTIONS[index].1));
                    }
                },
            );
            widgets::separator(ui);

            let encoder_labels = encoder_labels(self.encoders.as_deref());
            let encoder_now = encoder_index(self.encoders.as_deref(), &settings.encoder);
            let encoder_sub = match &self.encoders {
                Some(found) => {
                    let hardware: Vec<&str> = found
                        .iter()
                        .filter(|kind| **kind != EncoderKind::X264)
                        .map(|kind| kind.label())
                        .collect();
                    if hardware.is_empty() {
                        "Hardware encoders found: none (using CPU)".to_string()
                    } else {
                        format!("Hardware encoders found: {}", hardware.join(", "))
                    }
                }
                None => "GPU encoders have almost no impact on game performance".to_string(),
            };
            widgets::row(ui, "Encoder", Some(&encoder_sub), |ui| {
                if let Some(index) = widgets::combo(ui, "encoder", &encoder_labels, encoder_now) {
                    let key = encoder_key(self.encoders.as_deref(), index);
                    changes.push(Change::new("encoder", key));
                }
            });
            widgets::separator(ui);

            let codec_labels = [
                "H.264 (plays everywhere)".to_string(),
                "HEVC / H.265 (smaller files)".to_string(),
            ];
            let codec_now = usize::from(settings.codec == rewind_engine::Codec::Hevc);
            widgets::row(ui, "Codec", None, |ui| {
                if let Some(index) = widgets::combo(ui, "codec", &codec_labels, codec_now) {
                    changes.push(Change::new(
                        "codec",
                        if index == 1 { "hevc" } else { "h264" },
                    ));
                }
            });
        });
    }

    fn capture_card(
        &self,
        ui: &mut egui::Ui,
        settings: &rewind_engine::Settings,
        changes: &mut Vec<Change>,
    ) {
        widgets::card(ui, Some("Capture"), |ui| {
            let monitor_labels: Vec<String> = if self.monitors.is_empty() {
                vec!["Display 1  ·  1920×1080".to_string()]
            } else {
                self.monitors
                    .iter()
                    .map(|m| {
                        format!(
                            "{}  ·  {}×{}{}",
                            m.name,
                            m.w,
                            m.h,
                            if m.primary { "  (main)" } else { "" }
                        )
                    })
                    .collect()
            };
            let monitor_now = settings.monitor.min(monitor_labels.len().saturating_sub(1));
            widgets::row(ui, "Monitor", None, |ui| {
                if let Some(index) = widgets::combo(ui, "monitor", &monitor_labels, monitor_now) {
                    changes.push(Change::new("monitor", index as i64));
                }
            });
            widgets::separator(ui);

            let mut cursor = settings.capture_cursor;
            widgets::row(ui, "Show mouse cursor", None, |ui| {
                if widgets::switch(ui, &mut cursor) {
                    changes.push(Change::new("capture_cursor", cursor));
                }
            });
        });
    }

    fn audio_card(
        &self,
        ui: &mut egui::Ui,
        settings: &rewind_engine::Settings,
        changes: &mut Vec<Change>,
    ) {
        widgets::card(ui, Some("Audio"), |ui| {
            let mut system = settings.system_audio;
            widgets::row(
                ui,
                "Desktop audio",
                Some("Game and everything else you hear"),
                |ui| {
                    if widgets::switch(ui, &mut system) {
                        changes.push(Change::new("system_audio", system));
                    }
                },
            );
            widgets::separator(ui);

            let mut system_volume = settings.system_volume as i64;
            widgets::row(ui, "Desktop volume", None, |ui| {
                ui.add_enabled_ui(settings.system_audio, |ui| {
                    if widgets::slider(ui, &mut system_volume, (0, 200), 5, "%") {
                        changes.push(Change::new("system_volume", system_volume));
                    }
                });
            });
            widgets::separator(ui);

            let mut mic = settings.mic_enabled;
            widgets::row(ui, "Microphone", Some("Mix your voice into clips"), |ui| {
                if widgets::switch(ui, &mut mic) {
                    changes.push(Change::new("mic_enabled", mic));
                }
            });
            widgets::separator(ui);

            let mut mic_labels = vec!["Default microphone".to_string()];
            mic_labels.extend(self.microphones.iter().cloned());
            let mic_now = self
                .microphones
                .iter()
                .position(|name| *name == settings.mic_device)
                .map(|index| index + 1)
                .unwrap_or(0);
            widgets::row(ui, "Mic device", None, |ui| {
                ui.add_enabled_ui(settings.mic_enabled, |ui| {
                    if let Some(index) = widgets::combo(ui, "mic_device", &mic_labels, mic_now) {
                        let name = if index == 0 {
                            String::new()
                        } else {
                            mic_labels[index].clone()
                        };
                        changes.push(Change::new("mic_device", name));
                    }
                });
            });
            widgets::separator(ui);

            let mut mic_volume = settings.mic_volume as i64;
            widgets::row(ui, "Mic volume", None, |ui| {
                ui.add_enabled_ui(settings.mic_enabled, |ui| {
                    if widgets::slider(ui, &mut mic_volume, (0, 200), 5, "%") {
                        changes.push(Change::new("mic_volume", mic_volume));
                    }
                });
            });
            widgets::separator(ui);

            let mut offset = settings.audio_offset_ms as i64;
            widgets::row(
                ui,
                "Audio sync offset",
                Some("Only touch this if sound is early (+) or late (−) in your clips"),
                |ui| {
                    if widgets::number(ui, &mut offset, (-1000, 1000), 10.0, " ms") {
                        changes.push(Change::new("audio_offset_ms", offset));
                    }
                },
            );
        });
    }

    fn saving_card(
        &self,
        ui: &mut egui::Ui,
        controller: &Controller,
        settings: &rewind_engine::Settings,
        changes: &mut Vec<Change>,
    ) {
        widgets::card(ui, Some("Saving"), |ui| {
            let folder = settings.output_dir.display().to_string();
            widgets::row(ui, "Clips folder", Some(&folder), |ui| {
                if ui.button("Change…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new()
                        .set_directory(&settings.output_dir)
                        .pick_folder()
                    {
                        changes.push(Change::new("output_dir", dir));
                    }
                }
                if ui.button("Open").clicked() {
                    let _ = controller.open_clips_folder();
                }
            });
            widgets::separator(ui);

            let formats = [
                "PNG (lossless)".to_string(),
                "JPG (smaller files)".to_string(),
            ];
            let format_now = usize::from(settings.screenshot_format.extension() == "jpg");
            widgets::row(
                ui,
                "Screenshot format",
                Some("Screenshots are saved in the same folder as your clips"),
                |ui| {
                    if let Some(index) =
                        widgets::combo(ui, "screenshot_format", &formats, format_now)
                    {
                        changes.push(Change::new(
                            "screenshot_format",
                            if index == 1 { "jpg" } else { "png" },
                        ));
                    }
                },
            );
        });
    }
}

impl Default for ReplayPage {
    fn default() -> Self {
        Self::new()
    }
}

/// Rough size of one clip at a constant bitrate, in decimal megabytes.
///
/// `mbps × seconds / 8` is what the Python page showed; it is an estimate because the
/// container and the audio track add a little.
fn estimate_mb(mbps: u32, seconds: u32) -> f64 {
    f64::from(mbps) * f64::from(seconds) / 8.0
}

fn resolution_index(key: &str) -> usize {
    RESOLUTIONS
        .iter()
        .position(|(_, value)| *value == key)
        .unwrap_or(0)
}

fn encoder_labels(encoders: Option<&[EncoderKind]>) -> Vec<String> {
    let mut labels = vec!["Automatic (best available)".to_string()];
    for kind in encoders.unwrap_or(&[]) {
        labels.push(kind.label().to_string());
    }
    labels
}

/// Index into [`encoder_labels`] for a stored encoder key.
fn encoder_index(encoders: Option<&[EncoderKind]>, key: &str) -> usize {
    let Some(encoders) = encoders else {
        return 0;
    };
    encoders
        .iter()
        .position(|kind| kind.key() == key)
        .map(|index| index + 1)
        .unwrap_or(0)
}

/// The stored encoder key for a picked index.
fn encoder_key(encoders: Option<&[EncoderKind]>, index: usize) -> String {
    match (encoders, index.checked_sub(1)) {
        (Some(encoders), Some(offset)) => encoders
            .get(offset)
            .map(|kind| kind.key().to_string())
            .unwrap_or_else(|| "auto".to_string()),
        _ => "auto".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_estimate_matches_the_python_page() {
        // 50 Mbps for 30 s is 187.5 MB, shown as "188".
        assert_eq!(format!("{:.0}", estimate_mb(50, 30)), "188");
        assert_eq!(format!("{:.0}", estimate_mb(5, 15)), "9");
        assert_eq!(format!("{:.0}", estimate_mb(0, 30)), "0");
    }

    #[test]
    fn resolutions_round_trip_through_their_keys() {
        for (index, (_, key)) in RESOLUTIONS.iter().enumerate() {
            assert_eq!(resolution_index(key), index);
        }
        // An unrecognised key (an older build's) falls back to Native.
        assert_eq!(resolution_index("2160"), 0);
    }

    #[test]
    fn encoders_round_trip_through_the_dropdown() {
        let found = [EncoderKind::Nvenc, EncoderKind::X264];
        let labels = encoder_labels(Some(&found));
        assert_eq!(
            labels,
            vec!["Automatic (best available)", "NVIDIA NVENC", "CPU (x264)"]
        );
        assert_eq!(encoder_index(Some(&found), "nvenc"), 1);
        assert_eq!(encoder_index(Some(&found), "x264"), 2);
        assert_eq!(encoder_index(Some(&found), "auto"), 0);
        // A stored encoder that is not available here falls back to Automatic.
        assert_eq!(encoder_index(Some(&found), "amf"), 0);
        assert_eq!(encoder_key(Some(&found), 1), "nvenc");
        assert_eq!(encoder_key(Some(&found), 0), "auto");
    }

    #[test]
    fn encoders_are_empty_before_the_probe_answers() {
        assert_eq!(encoder_labels(None).len(), 1);
        assert_eq!(encoder_index(None, "nvenc"), 0);
    }
}
