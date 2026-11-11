//! The Overlay page: the status icon's visibility, corner, size and opacity.

use eframe::egui;

use crate::controller::Controller;
use crate::overlay::Flash;
use crate::ui::{widgets, Change, PageOutput};

pub struct OverlayPage;

impl OverlayPage {
    pub const SUBTITLE: &'static str =
        "The little status icon, and the one thing it must never do: get into a clip.";

    pub fn new() -> OverlayPage {
        OverlayPage
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, controller: &mut Controller) -> PageOutput {
        let mut output = PageOutput::default();
        let settings = controller.settings();

        widgets::card(ui, Some("Status icon"), |ui| {
            let mut enabled = settings.overlay_enabled;
            widgets::row(
                ui,
                "Show the status icon",
                Some("A small pill in the corner of the screen, so you can see Rewind working."),
                |ui| {
                    if widgets::switch(ui, &mut enabled) {
                        output.changes.push(Change::new("overlay_enabled", enabled));
                    }
                },
            );
            widgets::separator(ui);

            let modes: Vec<String> = ["Always visible", "Only when something changes"]
                .iter()
                .map(|label| (*label).to_string())
                .collect();
            let current = usize::from(settings.overlay_mode == "auto");
            widgets::row(
                ui,
                "Visibility",
                Some(if settings.overlay_mode == "auto" {
                    "Appears for a moment when Replay changes or a clip saves."
                } else {
                    "Stays in the corner while Rewind is running."
                }),
                |ui| {
                    if let Some(index) = widgets::combo(ui, "overlay_mode", &modes, current) {
                        let mode = if index == 1 { "auto" } else { "always" };
                        output.changes.push(Change::new("overlay_mode", mode));
                    }
                },
            );
            widgets::separator(ui);

            let corners: Vec<String> = ["Bottom right", "Bottom left", "Top right", "Top left"]
                .iter()
                .map(|label| (*label).to_string())
                .collect();
            let keys = ["bottom-right", "bottom-left", "top-right", "top-left"];
            let current = keys
                .iter()
                .position(|key| *key == settings.overlay_corner)
                .unwrap_or(0);
            widgets::row(ui, "Position", None, |ui| {
                if let Some(index) = widgets::combo(ui, "overlay_corner", &corners, current) {
                    output
                        .changes
                        .push(Change::new("overlay_corner", keys[index]));
                }
            });
            widgets::separator(ui);

            let mut scale = i64::from(settings.overlay_scale);
            widgets::row(ui, "Size", None, |ui| {
                if widgets::slider(ui, &mut scale, (75, 150), 5, "%").changed {
                    output.changes.push(Change::new("overlay_scale", scale));
                }
            });
            widgets::separator(ui);

            let mut opacity = i64::from(settings.overlay_opacity);
            widgets::row(
                ui,
                "Opacity",
                Some("How strongly the icon sits over what is behind it."),
                |ui| {
                    if widgets::slider(ui, &mut opacity, (30, 100), 5, "%").changed {
                        output.changes.push(Change::new("overlay_opacity", opacity));
                    }
                },
            );
            widgets::separator(ui);

            widgets::row(ui, "Preview", None, |ui| {
                let enabled = settings.overlay_enabled;
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.button("Show the icon").clicked() {
                        output.flash = Some(Flash::ClipSaved {
                            seconds: settings.clip_seconds,
                        });
                    }
                });
                if !enabled {
                    ui.label(
                        egui::RichText::new("Turn the status icon on first.")
                            .size(11.0)
                            .color(crate::ui::theme::MUTED),
                    );
                }
            });
        });

        ui.add_space(16.0);

        widgets::card(ui, Some("Captures"), |ui| {
            ui.label(
                egui::RichText::new(
                    "The status icon lives in its own window, outside your captures: it never \
                     appears in a clip or a screenshot, and clicks pass straight through it to \
                     whatever is underneath.",
                )
                .size(12.5)
                .color(crate::ui::theme::MUTED),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "If it does show up in a clip, the capture exclusion was refused by the \
                     system; restarting Rewind normally fixes it.",
                )
                .size(12.5)
                .color(crate::ui::theme::MUTED),
            );
        });

        output
    }
}
