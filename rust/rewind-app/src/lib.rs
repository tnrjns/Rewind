//! Rewind's desktop shell.
//!
//! The capture engine lives in `rewind-engine` and never blocks its caller; this crate
//! is what turns it into an app: global hotkeys first, with the tray, event loop and
//! controller glue following.
//!
//! Hotkeys are the first slice because they are what makes Rewind feel like Rewind —
//! one key saves the last thirty seconds, from inside a game.

pub mod hotkeys;

pub use hotkeys::{HotkeyError, HotkeyManager, Registration, HOTKEY_ACTIONS};
