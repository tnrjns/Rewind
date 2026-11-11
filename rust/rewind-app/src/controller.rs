//! The controller: turns hotkey presses and settings changes into engine commands.
//!
//! This is the headless equivalent of `app/controller.py`. It owns the decisions that
//! are policy rather than capture — when a recording should resume replay afterwards,
//! what a press does while something is already saving — so the engine stays a
//! mechanical capture layer and the rules live in one readable place.
//!
//! Everything here is synchronous and toolkit-free, so it can be driven by any event
//! loop (or a test) without a window.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rewind_engine::ffmpeg::{find_ffmpeg, probe_duration};
use rewind_engine::meta::{load_meta, update_meta};
use rewind_engine::paths::meta_key;
use rewind_engine::{screenshot, trim, Event, Mode, Recorder, Settings, State};

/// Something the controller wants the app to do — show a message, play a sound,
/// refresh the clip list. The tray and window are the app's business, not this layer's.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// Text for the status banner; an empty string clears it.
    Message(String),
    /// A clip landed; play the clip sound and refresh the list.
    ClipSaved {
        path: PathBuf,
        duration: f64,
    },
    RecordingSaved {
        path: PathBuf,
        duration: f64,
    },
    ScreenshotSaved(PathBuf),
    Failed(String),
    /// The replay state the tray icon and overlay should show.
    StateChanged(State),
    SettingsChanged(String),
    ClipsChanged,
}

/// What a press should do, given what the engine is currently doing.
///
/// Kept as a pure function so the awkward combinations can be tested without a real
/// capture: pressing clip mid-recording, toggling replay while saving, and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    StartReplay,
    StopReplay,
    StartRecording,
    StopRecording,
    SaveClip,
    TakeScreenshot,
    /// Nothing to do; the reason is worth telling the user.
    Ignore(&'static str),
}

/// Decide what toggling replay should do.
pub fn replay_action(state: State, mode: Mode) -> Action {
    if mode == Mode::Recording {
        return Action::Ignore("Instant Replay is paused while a full recording is in progress.");
    }
    if state == State::Off {
        Action::StartReplay
    } else {
        Action::StopReplay
    }
}

/// Decide what toggling recording should do.
pub fn recording_action(state: State, mode: Mode) -> Action {
    if mode == Mode::Recording {
        return Action::StopRecording;
    }
    if state == State::Saving {
        return Action::Ignore("Rewind is still saving the last recording.");
    }
    Action::StartRecording
}

/// Decide whether a clip request makes sense right now.
pub fn clip_action(state: State, mode: Mode, saving: bool) -> Action {
    if state != State::On || mode != Mode::Replay {
        return Action::Ignore("Instant Replay is off");
    }
    if saving {
        return Action::Ignore("A clip is already being saved");
    }
    Action::SaveClip
}

/// Drives a [`Recorder`] from hotkeys and settings, and reports what happened.
pub struct Controller {
    recorder: Arc<Recorder>,
    settings: Settings,
    /// Set when a recording was started while replay was running, so it can resume.
    resume_replay_after_recording: bool,
    /// Guards against two screenshot grabs at once.
    shooting: AtomicBool,
    ffmpeg: Option<PathBuf>,
}

impl Controller {
    pub fn new(settings: Settings) -> Controller {
        let recorder = Recorder::new(settings.clone());
        let ffmpeg = find_ffmpeg();
        Controller {
            recorder: Arc::new(recorder),
            settings,
            resume_replay_after_recording: false,
            shooting: AtomicBool::new(false),
            ffmpeg,
        }
    }

    /// Share the recorder, e.g. to hand it to an event loop that reads its events.
    pub fn recorder(&self) -> Arc<Recorder> {
        Arc::clone(&self.recorder)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn state(&self) -> State {
        self.recorder.state()
    }

    pub fn mode(&self) -> Mode {
        self.recorder.mode()
    }

    /// Run the action bound to a hotkey name.
    pub fn on_hotkey(&mut self, action: &str) -> Vec<Notice> {
        match action {
            "toggle" => self.toggle_replay(),
            "clip" => self.save_clip(),
            "record" => self.toggle_recording(),
            "screenshot" => self.take_screenshot(),
            // Opening the window is the app's job; it observes the press directly.
            "open" => Vec::new(),
            other => vec![Notice::Message(format!("Unknown hotkey action: {other}"))],
        }
    }

    pub fn toggle_replay(&mut self) -> Vec<Notice> {
        let action = replay_action(self.state(), self.mode());
        self.apply(action)
    }

    pub fn set_replay(&mut self, on: bool) -> Vec<Notice> {
        let action = if on {
            // Turning replay on during a recording is refused, same as toggling.
            match replay_action(self.state(), self.mode()) {
                Action::StartReplay | Action::StopReplay => Action::StartReplay,
                other => other,
            }
        } else {
            Action::StopReplay
        };
        self.apply(action)
    }

    pub fn toggle_recording(&mut self) -> Vec<Notice> {
        let action = recording_action(self.state(), self.mode());
        if action == Action::StartRecording {
            // Only resume replay afterwards if it was running before the recording.
            self.resume_replay_after_recording =
                self.state() != State::Off && self.mode() == Mode::Replay;
        }
        self.apply(action)
    }

    pub fn save_clip(&mut self) -> Vec<Notice> {
        let action = clip_action(self.state(), self.mode(), false);
        if action != Action::SaveClip {
            return self.apply(action);
        }
        // The engine itself refuses a second concurrent save, so its answer is the
        // authority; only report a refusal it did not already explain.
        if self.recorder.save_clip(Some(self.settings.clip_seconds)) {
            Vec::new()
        } else {
            vec![Notice::Message("A clip is already being saved".into())]
        }
    }

    fn apply(&mut self, action: Action) -> Vec<Notice> {
        match action {
            Action::StartReplay => {
                self.recorder.start();
                Vec::new()
            }
            Action::StopReplay => {
                self.recorder.stop();
                Vec::new()
            }
            Action::StartRecording => {
                self.recorder.start_recording();
                Vec::new()
            }
            Action::StopRecording => {
                self.recorder.stop_recording();
                Vec::new()
            }
            Action::SaveClip => Vec::new(),
            Action::TakeScreenshot => Vec::new(),
            Action::Ignore(reason) => vec![Notice::Message(reason.to_string())],
        }
    }

    /// Grab one frame of the selected monitor and save it beside the clips.
    pub fn take_screenshot(&mut self) -> Vec<Notice> {
        if self.shooting.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        let result = self.screenshot_now();
        self.shooting.store(false, Ordering::SeqCst);
        match result {
            Ok(path) => vec![Notice::ScreenshotSaved(path)],
            Err(message) => vec![Notice::Failed(message)],
        }
    }

    fn screenshot_now(&mut self) -> Result<PathBuf, String> {
        let Some(ffmpeg) = self.ffmpeg.clone().or_else(find_ffmpeg) else {
            return Err("FFmpeg wasn't found".into());
        };
        let dir = self.settings.output_dir.clone();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = screenshot::screenshot_path(&dir, self.settings.screenshot_format);
        let monitor = self
            .recorder
            .monitors()
            .get(self.settings.monitor)
            .map(|_| self.settings.monitor)
            .unwrap_or(0);
        let ok = screenshot::take_screenshot(
            &ffmpeg,
            monitor,
            &path,
            self.settings.capture_cursor,
            false,
        );
        if ok {
            Ok(path)
        } else {
            Err("Couldn't capture screen".into())
        }
    }

    /// Change one setting, applying anything the engine needs to know about.
    ///
    /// Returns whether the change should restart the capture (the app debounces that,
    /// because a restart drops the buffered seconds).
    pub fn set<T: Into<SettingValue>>(&mut self, key: &str, value: T) -> (Vec<Notice>, bool) {
        let value = value.into();
        let changed = value.apply(&mut self.settings, key);
        if !changed {
            return (Vec::new(), false);
        }
        self.recorder.update_settings(self.settings.clone());
        let restart = needs_restart(key) && matches!(self.state(), State::On | State::Starting);
        let mut notices = vec![Notice::SettingsChanged(key.to_string())];
        if key == "output_dir" {
            notices.push(Notice::ClipsChanged);
        }
        (notices, restart)
    }

    /// Trim a clip to `start..end`, saving the cut beside the original.
    pub fn trim(&mut self, video: &Path, start: f64, end: f64) -> Vec<Notice> {
        let Some(ffmpeg) = self.ffmpeg.clone().or_else(find_ffmpeg) else {
            return vec![Notice::Failed("FFmpeg wasn't found".into())];
        };
        match trim::trim_clip(&ffmpeg, video, start, end) {
            Ok(result) => {
                let duration = result.end - result.start;
                let _ = update_meta(
                    &meta_key(&result.path),
                    &[(
                        "duration",
                        serde_json::json!((duration * 100.0).round() / 100.0),
                    )],
                );
                vec![
                    Notice::ClipSaved {
                        path: result.path,
                        duration,
                    },
                    Notice::ClipsChanged,
                ]
            }
            Err(err) => vec![Notice::Failed(err.to_string())],
        }
    }

    /// Duration of a clip, from `clips.json` or by probing the file.
    pub fn clip_duration(&self, video: &Path) -> Option<f64> {
        if let Some(known) = rewind_engine::meta::meta_duration(&meta_key(video)) {
            return Some(known);
        }
        let ffmpeg = self.ffmpeg.clone().or_else(find_ffmpeg)?;
        let duration = probe_duration(&ffmpeg, video)?;
        let _ = update_meta(
            &meta_key(video),
            &[(
                "duration",
                serde_json::json!((duration * 100.0).round() / 100.0),
            )],
        );
        Some(duration)
    }

    /// Files in the clips folder, newest first.
    pub fn list_clips(&self, limit: usize) -> Vec<ClipEntry> {
        let meta = load_meta();
        let dir = &self.settings.output_dir;
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut items: Vec<(PathBuf, std::time::SystemTime, u64)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
                let is_clip = matches!(ext.as_str(), "mp4" | "mkv" | "mov");
                let is_shot = matches!(ext.as_str(), "png" | "jpg" | "jpeg");
                if !(is_clip || is_shot) {
                    return None;
                }
                let meta = entry.metadata().ok()?;
                Some((path, meta.modified().ok()?, meta.len()))
            })
            .collect();
        items.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
        items
            .into_iter()
            .take(limit)
            .map(|(path, modified, size)| {
                let is_shot = path
                    .extension()
                    .map(|e| {
                        let e = e.to_string_lossy().to_ascii_lowercase();
                        matches!(e.as_str(), "png" | "jpg" | "jpeg")
                    })
                    .unwrap_or(false);
                let thumb = if is_shot {
                    Some(path.clone())
                } else {
                    let t = rewind_engine::paths::thumb_path(&path);
                    t.exists().then_some(t)
                };
                ClipEntry {
                    duration: if is_shot {
                        None
                    } else {
                        rewind_engine::meta::duration_of(&meta, &path)
                    },
                    path,
                    modified,
                    size,
                    kind: if is_shot {
                        ClipKind::Image
                    } else {
                        ClipKind::Video
                    },
                    thumbnail: thumb,
                }
            })
            .collect()
    }

    pub fn open_clips_folder(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.settings.output_dir).map_err(|e| e.to_string())?;
        rewind_engine::paths::open_path(&self.settings.output_dir);
        Ok(())
    }

    /// Translate one engine event into what the app should show or play.
    ///
    /// The clip sound is chosen here rather than in the engine, because whether a sound
    /// plays at all is a settings question.
    pub fn handle_event(&mut self, event: Event) -> Vec<Notice> {
        match event {
            Event::StateChanged(state) => {
                // Replay resumes once a recording has finished, if it was running before.
                if state == State::Off && self.resume_replay_after_recording {
                    self.resume_replay_after_recording = false;
                    let action = replay_action(self.state(), self.mode());
                    if action == Action::StartReplay {
                        self.recorder.start();
                    }
                }
                vec![Notice::StateChanged(state)]
            }
            Event::Error(message) => vec![Notice::Message(message)],
            Event::ClipStarted => vec![Notice::Message(String::new())],
            Event::ClipSaved { path, duration } => {
                let _ = update_meta(
                    &meta_key(&path),
                    &[(
                        "duration",
                        serde_json::json!((duration * 100.0).round() / 100.0),
                    )],
                );
                vec![Notice::ClipSaved { path, duration }, Notice::ClipsChanged]
            }
            Event::ClipFailed(message) => vec![Notice::Failed(message)],
            Event::RecordingSaved { path, duration } => vec![
                Notice::RecordingSaved { path, duration },
                Notice::ClipsChanged,
            ],
            Event::RecordingFailed(message) => vec![Notice::Failed(message)],
            Event::ThumbnailReady(_) => vec![Notice::ClipsChanged],
            Event::EncodersDetected(_) => Vec::new(),
        }
    }

    /// Stop capture and finish any recording before the process exits.
    pub fn shutdown(&self) {
        self.recorder.shutdown();
    }

    /// Wait for the next engine event, with a timeout, for callers that poll.
    pub fn pump(&mut self, timeout: Duration) -> Vec<Notice> {
        let deadline = Instant::now() + timeout;
        let mut notices = Vec::new();
        loop {
            while let Some(event) = self.recorder.try_recv() {
                notices.extend(self.handle_event(event));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !notices.is_empty() || remaining.is_zero() {
                return notices;
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        }
    }
}

/// Settings keys that require restarting capture to take effect.
pub fn needs_restart(key: &str) -> bool {
    matches!(
        key,
        "bitrate_mbps"
            | "fps"
            | "resolution"
            | "monitor"
            | "encoder"
            | "codec"
            | "capture_cursor"
            | "system_audio"
            | "mic_enabled"
            | "mic_device"
    )
}

/// A value to store in settings, keeping the controller's `set` ergonomic.
#[derive(Debug, Clone)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl From<bool> for SettingValue {
    fn from(v: bool) -> Self {
        SettingValue::Bool(v)
    }
}
impl From<i64> for SettingValue {
    fn from(v: i64) -> Self {
        SettingValue::Int(v)
    }
}
impl From<u32> for SettingValue {
    fn from(v: u32) -> Self {
        SettingValue::Int(v as i64)
    }
}
impl From<usize> for SettingValue {
    fn from(v: usize) -> Self {
        SettingValue::Int(v as i64)
    }
}
impl From<&str> for SettingValue {
    fn from(v: &str) -> Self {
        SettingValue::Text(v.to_string())
    }
}
impl From<String> for SettingValue {
    fn from(v: String) -> Self {
        SettingValue::Text(v)
    }
}
impl From<PathBuf> for SettingValue {
    fn from(v: PathBuf) -> Self {
        SettingValue::Text(v.to_string_lossy().into_owned())
    }
}

impl SettingValue {
    /// Apply to `settings`, returning whether anything actually changed.
    fn apply(&self, settings: &mut Settings, key: &str) -> bool {
        match (key, self) {
            ("clip_seconds", SettingValue::Int(v)) => {
                let v = (*v).clamp(15, 60) as u32;
                let changed = settings.clip_seconds != v;
                settings.clip_seconds = v;
                changed
            }
            ("bitrate_mbps", SettingValue::Int(v)) => {
                let v = (*v).clamp(2, 200) as u32;
                let changed = settings.bitrate_mbps != v;
                settings.bitrate_mbps = v;
                changed
            }
            ("fps", SettingValue::Int(v)) => {
                let v = (*v).max(1) as u32;
                let changed = settings.fps != v;
                settings.fps = v;
                changed
            }
            ("monitor", SettingValue::Int(v)) => {
                let v = (*v).max(0) as usize;
                let changed = settings.monitor != v;
                settings.monitor = v;
                changed
            }
            ("system_volume", SettingValue::Int(v)) => {
                let v = (*v).clamp(0, 100) as i32;
                let changed = settings.system_volume != v;
                settings.system_volume = v;
                changed
            }
            ("mic_volume", SettingValue::Int(v)) => {
                let v = (*v).clamp(0, 100) as i32;
                let changed = settings.mic_volume != v;
                settings.mic_volume = v;
                changed
            }
            ("audio_offset_ms", SettingValue::Int(v)) => {
                let changed = settings.audio_offset_ms != *v as i32;
                settings.audio_offset_ms = *v as i32;
                changed
            }
            ("system_audio", SettingValue::Bool(v)) => {
                let changed = settings.system_audio != *v;
                settings.system_audio = *v;
                changed
            }
            ("mic_enabled", SettingValue::Bool(v)) => {
                let changed = settings.mic_enabled != *v;
                settings.mic_enabled = *v;
                changed
            }
            ("capture_cursor", SettingValue::Bool(v)) => {
                let changed = settings.capture_cursor != *v;
                settings.capture_cursor = *v;
                changed
            }
            ("start_replay_on_launch", SettingValue::Bool(v)) => {
                let changed = settings.start_replay_on_launch != *v;
                settings.start_replay_on_launch = *v;
                changed
            }
            ("copy_after_save", SettingValue::Bool(v)) => {
                let changed = settings.copy_after_save != *v;
                settings.copy_after_save = *v;
                changed
            }
            ("encoder", SettingValue::Text(v)) => {
                let changed = settings.encoder != *v;
                settings.encoder = v.clone();
                changed
            }
            ("mic_device", SettingValue::Text(v)) => {
                let changed = settings.mic_device != *v;
                settings.mic_device = v.clone();
                changed
            }
            ("output_dir", SettingValue::Text(v)) => {
                let path = PathBuf::from(v);
                let changed = settings.output_dir != path;
                settings.output_dir = path;
                changed
            }
            ("codec", SettingValue::Text(v)) => {
                let codec = if v == "hevc" {
                    rewind_engine::Codec::Hevc
                } else {
                    rewind_engine::Codec::H264
                };
                let changed = settings.codec != codec;
                settings.codec = codec;
                changed
            }
            ("resolution", SettingValue::Text(v)) => {
                let resolution = match v.as_str() {
                    "1440" => rewind_engine::Resolution::Height(1440),
                    "1080" => rewind_engine::Resolution::Height(1080),
                    "720" => rewind_engine::Resolution::Height(720),
                    _ => rewind_engine::Resolution::Native,
                };
                let changed = settings.resolution != resolution;
                settings.resolution = resolution;
                changed
            }
            _ => false,
        }
    }
}

/// One row of the Clips page.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipEntry {
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
    pub size: u64,
    pub duration: Option<f64>,
    pub kind: ClipKind,
    pub thumbnail: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    Video,
    Image,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_toggle_starts_when_off_and_stops_when_on() {
        assert_eq!(replay_action(State::Off, Mode::Off), Action::StartReplay);
        assert_eq!(replay_action(State::On, Mode::Replay), Action::StopReplay);
        assert_eq!(
            replay_action(State::Starting, Mode::Replay),
            Action::StopReplay
        );
    }

    #[test]
    fn replay_toggle_is_refused_during_a_recording() {
        let action = replay_action(State::On, Mode::Recording);
        assert!(matches!(action, Action::Ignore(msg) if msg.contains("paused")));
    }

    #[test]
    fn recording_toggle_starts_when_idle_and_stops_when_recording() {
        assert_eq!(
            recording_action(State::Off, Mode::Off),
            Action::StartRecording
        );
        assert_eq!(
            recording_action(State::On, Mode::Replay),
            Action::StartRecording
        );
        assert_eq!(
            recording_action(State::On, Mode::Recording),
            Action::StopRecording
        );
    }

    #[test]
    fn recording_toggle_waits_while_saving() {
        let action = recording_action(State::Saving, Mode::Off);
        assert!(matches!(action, Action::Ignore(msg) if msg.contains("saving")));
    }

    #[test]
    fn clip_is_refused_when_replay_is_off() {
        let action = clip_action(State::Off, Mode::Off, false);
        assert!(matches!(action, Action::Ignore(msg) if msg.contains("off")));
    }

    #[test]
    fn clip_is_refused_while_another_save_runs() {
        let action = clip_action(State::On, Mode::Replay, true);
        assert!(matches!(action, Action::Ignore(msg) if msg.contains("already")));
    }

    #[test]
    fn clip_is_allowed_while_replaying() {
        assert_eq!(
            clip_action(State::On, Mode::Replay, false),
            Action::SaveClip
        );
    }

    #[test]
    fn clip_is_refused_during_a_recording() {
        let action = clip_action(State::On, Mode::Recording, false);
        assert!(matches!(action, Action::Ignore(_)));
    }

    #[test]
    fn restart_keys_cover_every_capture_setting() {
        for key in [
            "bitrate_mbps",
            "fps",
            "resolution",
            "monitor",
            "encoder",
            "codec",
            "capture_cursor",
            "system_audio",
            "mic_enabled",
            "mic_device",
        ] {
            assert!(needs_restart(key), "{key} should restart capture");
        }
        // Changing the clip length must not restart: it would drop the buffer.
        assert!(!needs_restart("clip_seconds"));
        assert!(!needs_restart("output_dir"));
        assert!(!needs_restart("sound_volume"));
    }

    #[test]
    fn settings_are_clamped_on_write() {
        let mut settings = Settings::default();
        SettingValue::Int(999).apply(&mut settings, "clip_seconds");
        assert_eq!(settings.clip_seconds, 60);
        SettingValue::Int(1).apply(&mut settings, "clip_seconds");
        assert_eq!(settings.clip_seconds, 15);
        SettingValue::Int(-5).apply(&mut settings, "system_volume");
        assert_eq!(settings.system_volume, 0);
    }

    #[test]
    fn unchanged_setting_reports_no_change() {
        let mut settings = Settings::default();
        let before = settings.fps;
        assert!(!SettingValue::Int(before as i64).apply(&mut settings, "fps"));
        assert!(SettingValue::Int(30).apply(&mut settings, "fps"));
    }

    #[test]
    fn text_settings_parse_in_place() {
        let mut settings = Settings::default();
        assert!(SettingValue::Text("hevc".into()).apply(&mut settings, "codec"));
        assert_eq!(settings.codec, rewind_engine::Codec::Hevc);
        assert!(SettingValue::Text("1080".into()).apply(&mut settings, "resolution"));
        assert_eq!(settings.resolution, rewind_engine::Resolution::Height(1080));
    }

    #[test]
    fn unknown_keys_are_left_alone() {
        let mut settings = Settings::default();
        let before = settings.clone();
        assert!(!SettingValue::Bool(true).apply(&mut settings, "not_a_setting"));
        assert_eq!(settings, before);
    }
}
