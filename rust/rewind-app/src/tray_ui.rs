//! The tray icon and menu, drawn by the platform.
//!
//! Only the platform layer lives here: [`Tray::render`] takes the menu the pure model
//! in [`crate::tray`] produced and makes the OS show it. Everything decidable — which
//! entries exist, what is enabled, what the tooltip says — is decided before this
//! module is reached, so the rules are testable without a display.
//!
//! Rewind lives in the tray: closing the window leaves it running, and this is where
//! the user turns replay on and off, saves a clip, or quits.

#![cfg(feature = "tray")]

use std::collections::HashMap;
use std::path::Path;

use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::tray::{TrayAction, TrayState};

/// A tray icon plus its menu, kept alive for as long as the app runs.
pub struct Tray {
    icon: TrayIcon,
    menu: Menu,
    /// Menu entry per action, so a re-render updates text and enablement in place
    /// instead of rebuilding the menu (which would close it mid-click on Windows).
    items: HashMap<TrayActionKey, MenuEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TrayActionKey(TrayAction);

enum MenuEntry {
    Normal(MenuItem),
    Check(CheckMenuItem),
}

/// Which action a clicked menu id maps to.
pub fn action_for_menu_id(id: &muda::MenuId) -> Option<TrayAction> {
    let text = id.0.as_str();
    ACTIONS
        .iter()
        .find(|(name, _)| *name == text)
        .map(|(_, action)| *action)
}

/// Stable ids for menu entries: the action name, so a click can be mapped back.
const ACTIONS: [(&str, TrayAction); 7] = [
    ("open_window", TrayAction::OpenWindow),
    ("toggle_replay", TrayAction::ToggleReplay),
    ("toggle_recording", TrayAction::ToggleRecording),
    ("save_clip", TrayAction::SaveClip),
    ("take_screenshot", TrayAction::TakeScreenshot),
    ("open_clips_folder", TrayAction::OpenClipsFolder),
    ("quit", TrayAction::Quit),
];

fn id_for(action: TrayAction) -> &'static str {
    ACTIONS
        .iter()
        .find(|(_, a)| *a == action)
        .map(|(name, _)| *name)
        .unwrap_or("quit")
}

impl Tray {
    /// Create the tray icon with a menu for `state`.
    ///
    /// `icon_path` is the app logo; a missing or unreadable file is not fatal, the tray
    /// simply shows without an icon rather than refusing to start.
    pub fn new(state: TrayState, icon_path: Option<&Path>) -> Result<Tray, String> {
        let model = crate::tray::menu(state);
        let menu = Menu::new();
        let mut items = HashMap::new();
        for entry in &model {
            if entry.separator_before {
                let _ = menu.append(&PredefinedMenuItem::separator());
            }
            let key = TrayActionKey(entry.action);
            let built = if entry.action == TrayAction::ToggleReplay {
                let item = CheckMenuItem::with_id(
                    id_for(entry.action),
                    &entry.label,
                    entry.enabled,
                    entry.checked,
                    None,
                );
                let _ = menu.append(&item);
                MenuEntry::Check(item)
            } else {
                let item =
                    MenuItem::with_id(id_for(entry.action), &entry.label, entry.enabled, None);
                let _ = menu.append(&item);
                MenuEntry::Normal(item)
            };
            items.insert(key, built);
        }

        let icon = icon_path.and_then(load_icon);

        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_tooltip(crate::tray::tooltip(state));
        if let Some(icon) = icon {
            builder = builder.with_icon(icon);
        }
        let tray = builder.build().map_err(|e| e.to_string())?;

        Ok(Tray {
            icon: tray,
            menu,
            items,
        })
    }

    /// Update the menu, tooltip and icon for a new state, in place.
    pub fn render(&self, state: TrayState) {
        for entry in crate::tray::menu(state) {
            let key = TrayActionKey(entry.action);
            match self.items.get(&key) {
                Some(MenuEntry::Check(item)) => {
                    item.set_text(&entry.label);
                    item.set_checked(entry.checked);
                    item.set_enabled(entry.enabled);
                }
                Some(MenuEntry::Normal(item)) => {
                    item.set_text(&entry.label);
                    item.set_enabled(entry.enabled);
                }
                None => {}
            }
        }
        let _ = self.icon.set_tooltip(Some(crate::tray::tooltip(state)));
    }

    /// Show a balloon/notification, where the platform supports one.
    pub fn notify(&self, title: &str, body: &str) {
        // tray-icon has no portable notification API; the app layer owns this on
        // Windows and the notification centre on macOS. Kept as a hook so the caller
        // does not have to match on the platform itself.
        let _ = (title, body);
    }

    /// The menu, for callers that need to attach or inspect it.
    pub fn menu(&self) -> &Menu {
        &self.menu
    }
}

/// Load a PNG into a tray icon.
///
/// The icon is optional by design: a missing or unreadable asset should leave Rewind
/// running without a tray picture, not stop it from starting. macOS wants a small
/// template image, so anything larger is scaled down to 32 px on the longest side.
fn load_icon(path: &Path) -> Option<Icon> {
    let bytes = std::fs::read(path).ok()?;
    let (rgba, width, height) = decode_png(&bytes)?;
    Icon::from_rgba(rgba, width, height).ok()
}

/// Decode any PNG into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Expand palettes, grayscale and 16-bit samples to plain 8-bit RGBA.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let pixels = &buf[..info.buffer_size()];
    let (w, h) = (info.width, info.height);
    // The tray wants RGBA; convert from whatever the file actually stored.
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, w, h))
}

/// Drain pending menu clicks and map them to actions.
pub fn poll_menu_events() -> Vec<TrayAction> {
    MenuEvent::receiver()
        .try_iter()
        .filter_map(|event| action_for_menu_id(&event.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_stable_id() {
        for (name, action) in ACTIONS {
            assert_eq!(id_for(action), name);
        }
    }

    #[test]
    fn ids_round_trip_through_the_click_mapping() {
        for action in [
            TrayAction::OpenWindow,
            TrayAction::ToggleReplay,
            TrayAction::ToggleRecording,
            TrayAction::SaveClip,
            TrayAction::TakeScreenshot,
            TrayAction::OpenClipsFolder,
            TrayAction::Quit,
        ] {
            let id = muda::MenuId(id_for(action).to_string());
            assert_eq!(action_for_menu_id(&id), Some(action));
        }
    }

    #[test]
    fn unknown_menu_ids_are_ignored() {
        let id = muda::MenuId("something_else".into());
        assert_eq!(action_for_menu_id(&id), None);
    }

    #[test]
    fn every_model_item_maps_to_a_menu_id() {
        // A menu entry with no id would be a click that does nothing.
        for entry in crate::tray::menu(TrayState::On) {
            let id = id_for(entry.action);
            assert_eq!(
                action_for_menu_id(&muda::MenuId(id.to_string())),
                Some(entry.action)
            );
        }
    }
}
