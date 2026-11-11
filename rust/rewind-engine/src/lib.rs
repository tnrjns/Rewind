//! Rewind's headless capture engine.
//!
//! Keeps the last 15–60 seconds of the screen in a rolling buffer of one-second
//! MPEG-TS segments written by FFmpeg, and turns them into MP4 clips on demand —
//! stream-copied, so saving is instant and lossless. It also does full recordings,
//! screenshots and trims, and all of it without a window.
//!
//! ```no_run
//! use rewind_engine::{Recorder, Settings};
//!
//! let recorder = Recorder::new(Settings::load());
//! recorder.start();
//! // ... later, on the hotkey:
//! recorder.save_clip(None);
//! for event in std::iter::from_fn(|| recorder.recv()) {
//!     println!("{event:?}");
//! }
//! ```
//!
//! The engine never blocks the caller: commands are queued for a worker thread and
//! results come back as [`Event`]s. A UI can drive it from its own event loop with
//! [`Recorder::try_recv`].

pub mod audio;
pub mod ffmpeg;
pub mod meta;
pub mod paths;
pub mod platform;
pub mod recorder;
pub mod screenshot;
pub mod settings;
pub mod sounds;
pub mod trim;
pub mod wav;

pub use audio::{list_microphones, AudioKind};
pub use ffmpeg::{detect_encoders, find_ffmpeg, EncoderKind};
pub use meta::{load_meta, make_thumbnail, meta_duration, update_meta};
pub use paths::{data_dir, default_clips_dir, sub_dir};
pub use platform::{list_monitors, screen_capture_allowed, Monitor};
pub use recorder::{Event, Mode, Recorder, State, PERMISSION_NOTE};
pub use screenshot::{screenshot_path, take_screenshot};
pub use settings::{
    default_hotkeys, Codec, Hotkey, Resolution, ScreenshotFormat, Settings, APP_VERSION,
};
pub use trim::{clock, trim_clip, TrimError, TrimResult};

/// Fresh settings from disk, or defaults when there is no settings file yet.
pub fn load_settings() -> Settings {
    Settings::load()
}
