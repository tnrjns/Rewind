//! The Sound page: what plays when a clip lands.

use eframe::egui;

use crate::controller::Controller;
use crate::ui::{widgets, Change, PageOutput};
use rewind_engine::sounds;

pub struct SoundPage;

impl SoundPage {
    pub const SUBTITLE: &'static str =
        "A little confirmation so you know the clip landed, without alt-tabbing.";

    pub fn new() -> SoundPage {
        SoundPage
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, controller: &mut Controller) -> PageOutput {
        let mut output = PageOutput::default();
        let settings = controller.settings();
        // Played after the rows are drawn, once, with whatever was just chosen.
        let mut preview: Option<(String, i32, String)> = None;

        widgets::card(ui, Some("Clip sound"), |ui| {
            let mut enabled = settings.sound_enabled;
            widgets::row(ui, "Play a sound when a clip is saved", None, |ui| {
                if widgets::switch(ui, &mut enabled) {
                    output.changes.push(Change::new("sound_enabled", enabled));
                }
            });
            widgets::separator(ui);

            let labels: Vec<String> = sounds::STYLES
                .iter()
                .map(|(_, label)| (*label).to_string())
                .collect();
            let current = sounds::STYLES
                .iter()
                .position(|(key, _)| *key == settings.sound_style)
                .unwrap_or(0);
            let sub = if settings.sound_style == "custom" {
                std::path::Path::new(&settings.sound_custom)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let sub = (!sub.is_empty()).then_some(sub.as_str());

            widgets::row(ui, "Sound", sub, |ui| {
                if let Some(index) = widgets::combo(ui, "sound_style", &labels, current) {
                    let key = sounds::STYLES[index].0;
                    if key == "custom" {
                        // Cancelling with no earlier file leaves the setting alone.
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("WAV audio", &["wav"])
                            .pick_file()
                        {
                            let path = path.display().to_string();
                            output
                                .changes
                                .push(Change::new("sound_custom", path.clone()));
                            output.changes.push(Change::new("sound_style", "custom"));
                            preview = Some(("custom".to_string(), settings.sound_volume, path));
                        }
                    } else {
                        output.changes.push(Change::new("sound_style", key));
                        preview = Some((
                            key.to_string(),
                            settings.sound_volume,
                            settings.sound_custom.clone(),
                        ));
                    }
                }
            });
            widgets::separator(ui);

            let mut volume = i64::from(settings.sound_volume);
            let mut released = false;
            widgets::row(ui, "Volume", None, |ui| {
                let edit = widgets::slider(ui, &mut volume, (5, 100), 5, "%");
                if edit.changed {
                    output.changes.push(Change::new("sound_volume", volume));
                }
                released = edit.released;
            });
            if released {
                // Hearing the new volume on release is the point of the slider.
                preview = Some((
                    settings.sound_style.clone(),
                    volume as i32,
                    settings.sound_custom.clone(),
                ));
            }
            widgets::separator(ui);

            widgets::row(ui, "Preview", None, |ui| {
                if ui.button("Play test sound").clicked() {
                    preview = Some((
                        settings.sound_style.clone(),
                        settings.sound_volume,
                        settings.sound_custom.clone(),
                    ));
                }
            });
        });

        ui.add_space(16.0);

        widgets::card(ui, Some("Other sounds"), |ui| {
            let mut toggle = settings.toggle_sound;
            widgets::row(
                ui,
                "Play a sound when Instant Replay turns on or off",
                None,
                |ui| {
                    if widgets::switch(ui, &mut toggle) {
                        output.changes.push(Change::new("toggle_sound", toggle));
                    }
                },
            );
            widgets::separator(ui);

            let mut shutter = settings.screenshot_sound;
            widgets::row(
                ui,
                "Play a camera-shutter sound for screenshots",
                None,
                |ui| {
                    if widgets::switch(ui, &mut shutter) {
                        output
                            .changes
                            .push(Change::new("screenshot_sound", shutter));
                    }
                },
            );
        });

        if let Some((style, volume, custom)) = preview {
            sounds::play(&style, volume, &custom);
        }
        output
    }
}

impl Default for SoundPage {
    fn default() -> Self {
        Self::new()
    }
}
