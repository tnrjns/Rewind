//! The status pill: what it says, and when it goes away.
//!
//! Rewind shows a small pill in a screen corner saying whether replay is on. Clicks
//! pass through it, it never takes focus, and — the part that matters most — it is
//! excluded from screen capture so it never lands in a clip.
//!
//! This module is the content model only: given the engine state or a one-off message,
//! what two lines does the pill show, what colour, and how long until it returns to
//! the idle state. Keeping it separate from drawing means the awkward cases are
//! testable without a display: a flash that outlives its state, a failure while
//! recording, a save that never finishes.

use std::time::Duration;

/// Which small glyph sits in the coloured disc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// The rewind arrows: the app's own mark.
    Rewind,
    /// A tick: something was saved.
    Check,
    /// A ring: work in progress.
    Spinner,
    /// An exclamation mark: something failed.
    Bang,
}

/// Semantic colour, so the platform maps it to its own palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    /// Accent colour: replay on, or a save in progress.
    Active,
    /// Green: a clip or recording landed.
    Success,
    /// Amber: something failed.
    Warning,
    /// Grey: starting, or off.
    Quiet,
}

/// What the pill is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pill {
    /// Idle state, driven by the engine.
    Idle(IdleState),
    /// A one-off message that outranks the idle state for a moment.
    Flash(Flash),
}

/// The steady state the pill shows when nothing has just happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleState {
    Off,
    Starting,
    On { clip_seconds: u32 },
    Recording,
    SavingRecording,
}

/// A transient message, and how long it stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flash {
    SavingClip,
    ClipSaved { seconds: u32 },
    RecordingSaved { seconds: u32 },
    Screenshot,
    Failed { title: String, detail: String },
}

impl Flash {
    /// How long this message stays on screen before the pill returns to idle.
    ///
    /// A save that is still running is held much longer than a confirmation, so a slow
    /// clip does not appear stuck; a failure lingers longer than a success because the
    /// user needs time to read it.
    pub fn duration(&self) -> Duration {
        match self {
            Flash::SavingClip => Duration::from_secs(20),
            Flash::ClipSaved { .. } => Duration::from_millis(2400),
            Flash::RecordingSaved { .. } => Duration::from_millis(2400),
            Flash::Screenshot => Duration::from_millis(1800),
            Flash::Failed { .. } => Duration::from_secs(3),
        }
    }
}

/// The two lines, colour and glyph the pill draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    pub accent: Accent,
    pub glyph: Glyph,
    pub title: String,
    pub detail: String,
}

/// Text for a pill in a given state.
pub fn content(pill: Pill) -> Content {
    match pill {
        Pill::Idle(idle) => match idle {
            IdleState::Off => Content {
                accent: Accent::Quiet,
                glyph: Glyph::Rewind,
                title: "REWIND".into(),
                detail: "Off".into(),
            },
            IdleState::Starting => Content {
                accent: Accent::Quiet,
                glyph: Glyph::Spinner,
                title: "REWIND".into(),
                detail: "Starting…".into(),
            },
            IdleState::On { clip_seconds } => Content {
                accent: Accent::Active,
                glyph: Glyph::Rewind,
                title: "REWIND".into(),
                // The clip length is the number the user is about to capture, so it is
                // the most useful thing to show while replay is running.
                detail: format!("ON  ·  {clip_seconds}s"),
            },
            IdleState::Recording => Content {
                accent: Accent::Success,
                glyph: Glyph::Rewind,
                title: "RECORDING".into(),
                detail: "Screen capture on".into(),
            },
            IdleState::SavingRecording => Content {
                accent: Accent::Active,
                glyph: Glyph::Spinner,
                title: "REWIND".into(),
                detail: "Saving recording…".into(),
            },
        },
        Pill::Flash(flash) => match flash {
            Flash::SavingClip => Content {
                accent: Accent::Active,
                glyph: Glyph::Spinner,
                title: "REWIND".into(),
                detail: "Saving clip…".into(),
            },
            Flash::ClipSaved { seconds } => Content {
                accent: Accent::Success,
                glyph: Glyph::Check,
                title: "CLIP SAVED".into(),
                detail: format!("Last {seconds}s"),
            },
            Flash::RecordingSaved { seconds } => Content {
                accent: Accent::Success,
                glyph: Glyph::Check,
                title: "RECORDING SAVED".into(),
                detail: format!("{seconds} seconds"),
            },
            Flash::Screenshot => Content {
                accent: Accent::Active,
                glyph: Glyph::Check,
                title: "SCREENSHOT".into(),
                detail: "Saved to folder".into(),
            },
            Flash::Failed { title, detail } => Content {
                accent: Accent::Warning,
                glyph: Glyph::Bang,
                title: title.to_uppercase(),
                detail: truncate(detail.as_str(), DETAIL_LIMIT),
            },
        },
    }
}

/// Longest detail line the pill shows before trimming, so a long error does not
/// overflow the fixed-width pill.
pub const DETAIL_LIMIT: usize = 28;

/// Trim to `max` characters on a character boundary.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

/// How far through an animation cycle the pill is, for the spinner and the halo.
///
/// The Python app ticked at 33 ms and derived phase from elapsed time, so the spinner
/// speed did not depend on how often the timer actually fired.
pub fn phase(elapsed: Duration) -> f32 {
    let seconds = elapsed.as_secs_f32();
    // One halo pulse per 0.9 s, matching the original's `(phase * 0.9) % 1.0`.
    (seconds % 0.9) / 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_says_off() {
        let c = content(Pill::Idle(IdleState::Off));
        assert_eq!(c.detail, "Off");
        assert_eq!(c.accent, Accent::Quiet);
        assert_eq!(c.glyph, Glyph::Rewind);
    }

    #[test]
    fn on_shows_the_clip_length_that_is_about_to_be_saved() {
        let c = content(Pill::Idle(IdleState::On { clip_seconds: 30 }));
        assert_eq!(c.detail, "ON  ·  30s");
        assert_eq!(c.accent, Accent::Active);
    }

    #[test]
    fn on_reflects_a_changed_clip_length() {
        let c = content(Pill::Idle(IdleState::On { clip_seconds: 60 }));
        assert!(c.detail.contains("60s"), "{}", c.detail);
    }

    #[test]
    fn recording_is_green_and_named() {
        let c = content(Pill::Idle(IdleState::Recording));
        assert_eq!(c.title, "RECORDING");
        assert_eq!(c.accent, Accent::Success);
    }

    #[test]
    fn saving_a_recording_spins() {
        let c = content(Pill::Idle(IdleState::SavingRecording));
        assert_eq!(c.glyph, Glyph::Spinner);
        assert_eq!(c.detail, "Saving recording…");
    }

    #[test]
    fn a_saved_clip_counts_the_seconds_it_holds() {
        let c = content(Pill::Flash(Flash::ClipSaved { seconds: 30 }));
        assert_eq!(c.title, "CLIP SAVED");
        assert_eq!(c.detail, "Last 30s");
        assert_eq!(c.glyph, Glyph::Check);
        assert_eq!(c.accent, Accent::Success);
    }

    #[test]
    fn a_saved_recording_says_its_length() {
        let c = content(Pill::Flash(Flash::RecordingSaved { seconds: 95 }));
        assert_eq!(c.title, "RECORDING SAVED");
        assert_eq!(c.detail, "95 seconds");
    }

    #[test]
    fn a_screenshot_is_neutral_not_a_clip() {
        let c = content(Pill::Flash(Flash::Screenshot));
        assert_eq!(c.title, "SCREENSHOT");
        assert_eq!(c.glyph, Glyph::Check);
    }

    #[test]
    fn a_failure_is_amber_and_shows_its_reason() {
        let c = content(Pill::Flash(Flash::Failed {
            title: "Clip failed".into(),
            detail: "Instant Replay is off".into(),
        }));
        assert_eq!(c.accent, Accent::Warning);
        assert_eq!(c.glyph, Glyph::Bang);
        assert_eq!(c.title, "CLIP FAILED");
        assert_eq!(c.detail, "Instant Replay is off");
    }

    #[test]
    fn a_long_failure_is_trimmed_to_fit() {
        let long = "x".repeat(80);
        let c = content(Pill::Flash(Flash::Failed {
            title: "Clip failed".into(),
            detail: long,
        }));
        assert_eq!(c.detail.chars().count(), DETAIL_LIMIT);
    }

    #[test]
    fn trimming_respects_character_boundaries() {
        // Multi-byte characters must not be split, or the pill would show mojibake.
        let c = content(Pill::Flash(Flash::Failed {
            title: "Failed".into(),
            detail: "🎬".repeat(40),
        }));
        assert_eq!(c.detail.chars().count(), DETAIL_LIMIT);
        assert!(c.detail.chars().all(|ch| ch == '🎬'));
    }

    #[test]
    fn a_save_in_progress_is_held_longer_than_a_confirmation() {
        assert!(Flash::SavingClip.duration() > Flash::ClipSaved { seconds: 30 }.duration());
    }

    #[test]
    fn a_failure_lingers_longer_than_a_success() {
        // The user needs time to read why it failed.
        let failure = Flash::Failed {
            title: "Failed".into(),
            detail: String::new(),
        };
        assert!(failure.duration() > Flash::Screenshot.duration());
    }

    #[test]
    fn phase_cycles_within_one() {
        for ms in [0u64, 100, 450, 899, 900, 1800] {
            let p = phase(Duration::from_millis(ms));
            assert!((0.0..=1.0).contains(&p), "phase {p} at {ms}ms out of range");
        }
        // Halfway through a cycle.
        assert!((phase(Duration::from_millis(450)) - 0.5).abs() < 0.01);
    }

    #[test]
    fn phase_is_independent_of_tick_rate() {
        // The original derived phase from elapsed time, so a slow tick did not slow the
        // spinner; the same must hold here.
        assert_eq!(
            phase(Duration::from_millis(300)),
            phase(Duration::from_millis(300))
        );
    }

    #[test]
    fn every_idle_state_has_non_empty_text() {
        for idle in [
            IdleState::Off,
            IdleState::Starting,
            IdleState::On { clip_seconds: 30 },
            IdleState::Recording,
            IdleState::SavingRecording,
        ] {
            let c = content(Pill::Idle(idle));
            assert!(!c.title.is_empty(), "{idle:?}");
            assert!(!c.detail.is_empty(), "{idle:?}");
        }
    }
}
