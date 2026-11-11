//! Tray icon model: what the menu shows and which items are enabled.
//!
//! The tray is the app's main surface — Rewind lives in it, and closing the window
//! leaves it running. The *policy* of what the menu offers is pure data, so it is
//! testable without a tray, a display, or any OS call; the platform layer just renders
//! it and reports clicks back.
//!
//! The Python app derived this from the recorder on every state change, and the rules
//! are worth keeping exactly: Save clip is only offered while replay is actually
//! buffering, recording labels follow what is happening, and the toggle is disabled
//! while a recording owns the capture.

use rewind_engine::{Mode, State};

/// Current activity, as the tray presents it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Off,
    Starting,
    On,
    Recording,
    Saving,
}

impl TrayState {
    /// Derive the tray state from the engine, exactly as the Python app did: a
    /// recording *is* "on", but the tray names it differently.
    pub fn from_engine(state: State, mode: Mode) -> TrayState {
        match (state, mode) {
            (State::On, Mode::Recording) => TrayState::Recording,
            (State::Starting, Mode::Recording) => TrayState::Recording,
            (State::Saving, _) => TrayState::Saving,
            (State::Off, _) => TrayState::Off,
            (State::Starting, _) => TrayState::Starting,
            (State::On, _) => TrayState::On,
        }
    }

    /// Menu / tooltip text.
    pub fn label(self) -> &'static str {
        match self {
            TrayState::Off => "Instant Replay off",
            TrayState::Starting => "Starting Instant Replay…",
            TrayState::On => "Instant Replay on",
            TrayState::Recording => "Recording",
            TrayState::Saving => "Saving recording…",
        }
    }

    /// Whether the icon should be drawn dimmed (replay not running).
    pub fn dimmed(self) -> bool {
        !matches!(self, TrayState::On | TrayState::Recording)
    }
}

/// One row of the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    pub action: TrayAction,
    pub enabled: bool,
    /// Shown with a check mark, for the replay toggle.
    pub checked: bool,
    /// Draw a separator before this item.
    pub separator_before: bool,
}

/// What a menu entry does, mapped onto the controller's actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenWindow,
    ToggleReplay,
    ToggleRecording,
    SaveClip,
    TakeScreenshot,
    OpenClipsFolder,
    Quit,
}

/// The whole menu for a given state.
pub fn menu(state: TrayState) -> Vec<MenuItem> {
    let recording = state == TrayState::Recording;
    let saving = state == TrayState::Saving;
    // Save clip needs a live replay buffer; during a recording there is no buffer.
    let can_clip = state == TrayState::On;
    vec![
        MenuItem {
            label: "Open Rewind".into(),
            action: TrayAction::OpenWindow,
            enabled: true,
            checked: false,
            separator_before: false,
        },
        MenuItem {
            label: "Instant Replay".into(),
            action: TrayAction::ToggleReplay,
            enabled: !recording && !saving,
            checked: matches!(
                state,
                TrayState::On | TrayState::Starting | TrayState::Recording
            ),
            separator_before: true,
        },
        MenuItem {
            label: if recording {
                "Stop screen recording".into()
            } else {
                "Start screen recording".into()
            },
            action: TrayAction::ToggleRecording,
            enabled: !saving,
            checked: false,
            separator_before: false,
        },
        MenuItem {
            label: "Save clip".into(),
            action: TrayAction::SaveClip,
            enabled: can_clip,
            checked: false,
            separator_before: false,
        },
        MenuItem {
            label: "Take screenshot".into(),
            action: TrayAction::TakeScreenshot,
            enabled: true,
            checked: false,
            separator_before: false,
        },
        MenuItem {
            label: "Open clips folder".into(),
            action: TrayAction::OpenClipsFolder,
            enabled: true,
            checked: false,
            separator_before: false,
        },
        MenuItem {
            label: "Quit Rewind".into(),
            action: TrayAction::Quit,
            enabled: true,
            checked: false,
            separator_before: true,
        },
    ]
}

/// Tooltip text for the tray icon.
pub fn tooltip(state: TrayState) -> String {
    format!("Rewind — {}", state.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_on_reports_on() {
        assert_eq!(
            TrayState::from_engine(State::On, Mode::Replay),
            TrayState::On
        );
        assert_eq!(TrayState::On.label(), "Instant Replay on");
        assert!(!TrayState::On.dimmed());
    }

    #[test]
    fn a_recording_is_named_recording_not_on() {
        assert_eq!(
            TrayState::from_engine(State::On, Mode::Recording),
            TrayState::Recording
        );
        assert_eq!(TrayState::Recording.label(), "Recording");
        assert!(!TrayState::Recording.dimmed());
    }

    #[test]
    fn starting_a_recording_still_reads_as_recording() {
        // The Python app labelled a recording from the moment it was requested.
        assert_eq!(
            TrayState::from_engine(State::Starting, Mode::Recording),
            TrayState::Recording
        );
    }

    #[test]
    fn saving_outranks_the_mode() {
        assert_eq!(
            TrayState::from_engine(State::Saving, Mode::Recording),
            TrayState::Saving
        );
        assert_eq!(TrayState::Saving.label(), "Saving recording…");
        assert!(TrayState::Saving.dimmed());
    }

    #[test]
    fn off_is_dimmed() {
        assert_eq!(
            TrayState::from_engine(State::Off, Mode::Off),
            TrayState::Off
        );
        assert!(TrayState::Off.dimmed());
        assert_eq!(TrayState::Off.label(), "Instant Replay off");
    }

    #[test]
    fn menu_has_the_same_entries_in_the_same_order_every_state() {
        let expected = [
            TrayAction::OpenWindow,
            TrayAction::ToggleReplay,
            TrayAction::ToggleRecording,
            TrayAction::SaveClip,
            TrayAction::TakeScreenshot,
            TrayAction::OpenClipsFolder,
            TrayAction::Quit,
        ];
        for state in [
            TrayState::Off,
            TrayState::Starting,
            TrayState::On,
            TrayState::Recording,
            TrayState::Saving,
        ] {
            let actions: Vec<TrayAction> = menu(state).iter().map(|i| i.action).collect();
            assert_eq!(actions, expected, "menu changed for {state:?}");
        }
    }

    #[test]
    fn save_clip_needs_a_live_buffer() {
        assert!(
            menu(TrayState::On)
                .iter()
                .find(|i| i.action == TrayAction::SaveClip)
                .unwrap()
                .enabled
        );
        for state in [TrayState::Off, TrayState::Recording, TrayState::Saving] {
            assert!(
                !menu(state)
                    .iter()
                    .find(|i| i.action == TrayAction::SaveClip)
                    .unwrap()
                    .enabled,
                "Save clip should be disabled for {state:?}"
            );
        }
    }

    #[test]
    fn recording_item_swaps_its_label() {
        let start = menu(TrayState::On)
            .into_iter()
            .find(|i| i.action == TrayAction::ToggleRecording)
            .unwrap();
        assert_eq!(start.label, "Start screen recording");
        let stop = menu(TrayState::Recording)
            .into_iter()
            .find(|i| i.action == TrayAction::ToggleRecording)
            .unwrap();
        assert_eq!(stop.label, "Stop screen recording");
    }

    #[test]
    fn replay_toggle_is_disabled_while_a_recording_owns_the_capture() {
        for state in [TrayState::Recording, TrayState::Saving] {
            assert!(
                !menu(state)
                    .iter()
                    .find(|i| i.action == TrayAction::ToggleReplay)
                    .unwrap()
                    .enabled,
                "replay toggle should be disabled for {state:?}"
            );
        }
        assert!(
            menu(TrayState::On)
                .iter()
                .find(|i| i.action == TrayAction::ToggleReplay)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn replay_toggle_is_checked_while_replay_or_recording_runs() {
        for state in [TrayState::On, TrayState::Starting, TrayState::Recording] {
            assert!(
                menu(state)
                    .iter()
                    .find(|i| i.action == TrayAction::ToggleReplay)
                    .unwrap()
                    .checked,
                "{state:?} should show replay as active"
            );
        }
        assert!(
            !menu(TrayState::Off)
                .iter()
                .find(|i| i.action == TrayAction::ToggleReplay)
                .unwrap()
                .checked
        );
    }

    #[test]
    fn screenshots_and_folder_are_always_available() {
        for state in [TrayState::Off, TrayState::Saving] {
            for action in [TrayAction::TakeScreenshot, TrayAction::OpenClipsFolder] {
                assert!(
                    menu(state)
                        .iter()
                        .find(|i| i.action == action)
                        .unwrap()
                        .enabled,
                    "{action:?} should stay enabled for {state:?}"
                );
            }
        }
    }

    #[test]
    fn separators_group_the_menu() {
        let items = menu(TrayState::On);
        let before: Vec<TrayAction> = items
            .iter()
            .filter(|i| i.separator_before)
            .map(|i| i.action)
            .collect();
        assert_eq!(before, vec![TrayAction::ToggleReplay, TrayAction::Quit]);
    }

    #[test]
    fn tooltip_names_the_state() {
        assert_eq!(tooltip(TrayState::On), "Rewind — Instant Replay on");
        assert_eq!(tooltip(TrayState::Off), "Rewind — Instant Replay off");
        assert_eq!(tooltip(TrayState::Saving), "Rewind — Saving recording…");
    }
}
