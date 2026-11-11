//! The Hotkeys page: click a box, press a combo.
//!
//! Global hotkeys have to be *unregistered* while a combo is being captured, or pressing
//! the old binding would fire an action instead of being recorded. The page asks the
//! shell to pause them (`PageOutput::hotkeys`) and the shell resumes on close.

use eframe::egui::{self, RichText};

use crate::controller::Controller;
use crate::hotkeys::{configured_hotkeys, HotkeyManager, Registration};
use crate::ui::{keys, theme, widgets, Change, PageOutput};
use rewind_engine::default_hotkeys;

/// (action, row title, the settings key it writes)
const ROWS: [(&str, &str, &str); 5] = [
    ("toggle", "Turn Instant Replay on / off", "hotkey_toggle"),
    ("clip", "Save clip", "hotkey_clip"),
    ("record", "Start / stop screen recording", "hotkey_record"),
    ("screenshot", "Take screenshot", "hotkey_screenshot"),
    ("open", "Open Rewind", "hotkey_open"),
];

pub struct HotkeysPage {
    /// The action whose box is waiting for a press.
    capturing: Option<&'static str>,
}

impl HotkeysPage {
    pub const SUBTITLE: &'static str = "These work everywhere, even while you're in a \
         game. Click a box, then press the combo you want.";

    pub fn new() -> HotkeysPage {
        HotkeysPage { capturing: None }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        controller: &Controller,
        hotkeys: &Option<HotkeyManager>,
    ) -> PageOutput {
        let mut output = PageOutput::default();
        let settings = controller.settings();
        let status = hotkeys
            .as_ref()
            .map(|manager| manager.status())
            .unwrap_or_default();
        let (_, duplicates) = configured_hotkeys(settings);

        if let Some(action) = self.capturing {
            ui.ctx().request_repaint();
            let pressed = ui.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });

            if let Some((key, modifiers)) = pressed {
                if key == egui::Key::Escape {
                    self.capturing = None;
                    output.hotkeys = Some(false);
                } else if !is_modifier(key) {
                    if let Some(hotkey) = keys::capture(key, &modifiers) {
                        let row = ROWS.iter().find(|(name, _, _)| *name == action);
                        if let Some((_, _, setting)) = row {
                            output.changes.push(Change::new(setting, hotkey));
                        }
                        self.capturing = None;
                        output.hotkeys = Some(false);
                    }
                }
            }
        }

        widgets::card(ui, Some("Shortcuts"), |ui| {
            for (index, (action, title, _)) in ROWS.iter().enumerate() {
                if index > 0 {
                    widgets::separator(ui);
                }
                let (text, ok) = status_text(action, &status, &duplicates);
                let hotkey = settings.hotkey(action);
                let label = if self.capturing == Some(*action) {
                    "Press keys…  (Esc cancels)".to_string()
                } else if hotkey.text.is_empty() {
                    "Not set".to_string()
                } else {
                    hotkey.text.clone()
                };

                let color = if ok { theme::MUTED } else { theme::WARN };
                widgets::row_with(ui, title, Some(&text), color, |ui| {
                    let response = ui.add_sized(
                        [200.0, 30.0],
                        egui::Button::new(RichText::new(label).size(12.5))
                            .corner_radius(egui::CornerRadius::same(8)),
                    );
                    if response.clicked() && self.capturing.is_none() {
                        self.capturing = Some(action);
                    }
                });
            }
            widgets::separator(ui);
            widgets::row(
                ui,
                "Default keys",
                Some("Puts all five shortcuts back the way Rewind came"),
                |ui| {
                    if ui.add(egui::Button::new("Reset to defaults")).clicked() {
                        let (toggle, clip, record, screenshot, open) = default_hotkeys();
                        output.changes.push(Change::new("hotkey_toggle", toggle));
                        output.changes.push(Change::new("hotkey_clip", clip));
                        output.changes.push(Change::new("hotkey_record", record));
                        output
                            .changes
                            .push(Change::new("hotkey_screenshot", screenshot));
                        output.changes.push(Change::new("hotkey_open", open));
                    }
                },
            );
        });

        ui.add_space(12.0);
        let tip = if cfg!(target_os = "macos") {
            "Tip: if a combo shows “in use”, another app or macOS already owns it. Combos \
             need ⌃ or ⌘; macOS blocks ones that only use ⌥ and ⇧."
        } else {
            "Tip: if a combo shows “in use”, another app (often the NVIDIA overlay, \
             Discord or OBS) already owns it. Pick a different one, or turn it off in \
             that app."
        };
        ui.label(RichText::new(tip).size(12.0).color(theme::MUTED));

        // Pause after the frame: the next press is the one being captured, so the old
        // binding must be gone by then.
        if self.capturing.is_some() {
            output.hotkeys = Some(true);
        }
        output
    }
}

impl Default for HotkeysPage {
    fn default() -> Self {
        Self::new()
    }
}

/// The line under each shortcut: what the OS said about it.
fn status_text(
    action: &str,
    status: &[(String, Registration)],
    duplicates: &[String],
) -> (String, bool) {
    match status
        .iter()
        .find(|(name, _)| name == action)
        .map(|(_, registration)| *registration)
    {
        Some(Registration::Ok) => ("Active".to_string(), true),
        Some(Registration::Taken) if duplicates.iter().any(|name| name == action) => (
            "Same combo as another Rewind hotkey — pick another".to_string(),
            false,
        ),
        Some(Registration::Taken) => (
            "In use by another app — pick another combo".to_string(),
            false,
        ),
        Some(Registration::Unset) | None => ("Not set".to_string(), false),
    }
}

/// Modifier keys are part of the combo, not a combo on their own.
fn is_modifier(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ShiftLeft
            | egui::Key::ShiftRight
            | egui::Key::ControlLeft
            | egui::Key::ControlRight
            | egui::Key::AltLeft
            | egui::Key::AltRight
            | egui::Key::SuperLeft
            | egui::Key::SuperRight
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(entries: &[(&str, Registration)]) -> Vec<(String, Registration)> {
        entries
            .iter()
            .map(|(name, registration)| ((*name).to_string(), *registration))
            .collect()
    }

    #[test]
    fn an_active_hotkey_says_so() {
        let entries = status(&[("clip", Registration::Ok)]);
        assert_eq!(status_text("clip", &entries, &[]), ("Active".into(), true));
    }

    #[test]
    fn a_combo_taken_by_rewind_itself_is_named_differently() {
        let entries = status(&[("clip", Registration::Taken)]);
        let (text, ok) = status_text("clip", &entries, &["clip".to_string()]);
        assert!(!ok);
        assert!(text.contains("another Rewind hotkey"), "{text}");
    }

    #[test]
    fn a_combo_owned_by_another_app_points_at_the_other_app() {
        let entries = status(&[("clip", Registration::Taken)]);
        let (text, ok) = status_text("clip", &entries, &[]);
        assert!(!ok);
        assert!(text.contains("another app"), "{text}");
    }

    #[test]
    fn an_unbound_action_is_reported_as_not_set() {
        let entries = status(&[("clip", Registration::Unset)]);
        assert_eq!(status_text("clip", &entries, &[]).0, "Not set");
        assert_eq!(status_text("missing", &entries, &[]).0, "Not set");
    }

    #[test]
    fn modifier_keys_do_not_end_capture() {
        assert!(is_modifier(egui::Key::ControlLeft));
        assert!(is_modifier(egui::Key::SuperRight));
        assert!(!is_modifier(egui::Key::S));
    }
}
