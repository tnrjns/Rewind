//! Persistent settings, JSON-compatible with the Python app's `settings.json`.
//!
//! Unknown keys are preserved on save, so a settings file shared with the Python
//! build keeps its UI-only fields (overlay colours, tutorial flags…) instead of
//! being stripped by the engine.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::paths::{data_dir, default_clips_dir, write_atomic};

pub const APP_VERSION: &str = "1.3.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

impl Codec {
    pub fn as_str(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
        }
    }
    /// The `-tag:v` value MP4 needs so QuickTime and Safari play HEVC.
    pub fn mp4_tag(self) -> Option<&'static str> {
        match self {
            Codec::Hevc => Some("hvc1"),
            Codec::H264 => None,
        }
    }
    fn parse(s: &str) -> Codec {
        match s {
            "hevc" => Codec::Hevc,
            _ => Codec::H264,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Capture at the monitor's own size.
    Native,
    Height(u32),
}

impl Resolution {
    pub fn as_str(self) -> String {
        match self {
            Resolution::Native => "native".into(),
            Resolution::Height(h) => h.to_string(),
        }
    }
    fn parse(s: &str) -> Resolution {
        match s {
            "1440" => Resolution::Height(1440),
            "1080" => Resolution::Height(1080),
            "720" => Resolution::Height(720),
            _ => Resolution::Native,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotFormat {
    Png,
    Jpg,
}

impl ScreenshotFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ScreenshotFormat::Png => "png",
            ScreenshotFormat::Jpg => "jpg",
        }
    }
}

/// A global hotkey: modifier flags from the old app's `hotkeys.py` plus a platform key code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    /// Win32 `MOD_*` flags: Alt 1, Control 2, Shift 4, Win/Cmd 8.
    pub mods: u32,
    /// Win32 virtual-key code on Windows, macOS virtual key code on Mac.
    pub vk: u32,
    pub text: String,
}

impl Hotkey {
    /// Stable identity for duplicate detection: the same key with any extra modifiers still collides.
    pub fn combo(&self) -> (u32, u32, u32) {
        // Shift and Win are not part of collision identity in the Python app (mods, vk is),
        // but the raw pair is what it compares, so keep that exact behaviour.
        (0, self.mods, self.vk)
    }
}

impl Default for Hotkey {
    fn default() -> Self {
        default_hotkeys().0
    }
}

/// Default hotkeys in the order the tray and settings UI expect.
pub fn default_hotkeys() -> (Hotkey, Hotkey, Hotkey, Hotkey, Hotkey) {
    #[cfg(target_os = "macos")]
    {
        // macOS blocks combos whose only modifiers are Option/Shift, so defaults use Control + Option.
        let mk = |vk: u32, text: &str| Hotkey {
            mods: 3,
            vk,
            text: text.into(),
        };
        (
            mk(0x22, "⌃⌥I"),
            mk(0x01, "⌃⌥S"),
            mk(0x0F, "⌃⌥R"),
            mk(0x23, "⌃⌥P"),
            mk(0x1D, "⌃⌥0"),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mk = |vk: u32, text: &str| Hotkey {
            mods: 1,
            vk,
            text: text.into(),
        };
        (
            mk(0x78, "Alt + F9"),
            mk(0x79, "Alt + F10"),
            mk(0x7A, "Alt + F11"),
            mk(0x7B, "Alt + F12"),
            mk(0x30, "Alt + 0"),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    // Replay
    pub clip_seconds: u32,
    pub bitrate_mbps: u32,
    pub fps: u32,
    pub resolution: Resolution,
    pub monitor: usize,
    /// `auto` or an [`crate::ffmpeg::EncoderKind`] key.
    pub encoder: String,
    pub codec: Codec,
    pub capture_cursor: bool,
    // Audio
    pub system_audio: bool,
    pub system_volume: i32,
    pub mic_enabled: bool,
    pub mic_device: String,
    pub mic_volume: i32,
    pub audio_offset_ms: i32,
    // Saving
    pub output_dir: PathBuf,
    // Screenshots
    pub screenshot_format: ScreenshotFormat,
    pub screenshot_sound: bool,
    // General
    pub start_replay_on_launch: bool,
    pub copy_after_save: bool,
    pub sound_enabled: bool,
    pub sound_style: String,
    pub sound_custom: String,
    pub sound_volume: i32,
    pub toggle_sound: bool,
    /// Keys this build does not model (overlay, hotkey text, tutorial flags…) kept verbatim.
    extra: BTreeMap<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            clip_seconds: 30,
            bitrate_mbps: 50,
            fps: 60,
            resolution: Resolution::Native,
            monitor: 0,
            encoder: "auto".into(),
            codec: Codec::H264,
            capture_cursor: true,
            system_audio: true,
            system_volume: 100,
            mic_enabled: false,
            mic_device: String::new(),
            mic_volume: 100,
            audio_offset_ms: 0,
            output_dir: default_clips_dir(),
            screenshot_format: ScreenshotFormat::Png,
            screenshot_sound: true,
            start_replay_on_launch: true,
            copy_after_save: false,
            sound_enabled: true,
            sound_style: "chime".into(),
            sound_custom: String::new(),
            sound_volume: 70,
            toggle_sound: true,
            extra: BTreeMap::new(),
        }
    }
}

/// Every key the engine knows about; anything else round-trips through `extra`.
const KNOWN_KEYS: &[&str] = &[
    "clip_seconds",
    "bitrate_mbps",
    "fps",
    "resolution",
    "monitor",
    "encoder",
    "codec",
    "capture_cursor",
    "system_audio",
    "system_volume",
    "mic_enabled",
    "mic_device",
    "mic_volume",
    "audio_offset_ms",
    "output_dir",
    "screenshot_format",
    "screenshot_sound",
    "start_replay_on_launch",
    "copy_after_save",
    "sound_enabled",
    "sound_style",
    "sound_custom",
    "sound_volume",
    "toggle_sound",
];

impl Settings {
    pub fn path() -> PathBuf {
        data_dir().join("settings.json")
    }

    /// Load `settings.json`, falling back to defaults when missing or unreadable.
    ///
    /// Values are type-checked and clamped exactly like the Python loader, so a
    /// hand-edited file can never make the encoder fail.
    pub fn load() -> Settings {
        match fs::read(Self::path()) {
            Ok(bytes) => Settings::from_json(&bytes),
            Err(_) => Settings::default(),
        }
    }

    pub fn load_from(path: &std::path::Path) -> io::Result<Settings> {
        Ok(Settings::from_json(&fs::read(path)?))
    }

    pub fn from_json(bytes: &[u8]) -> Settings {
        let mut s = Settings::default();
        let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) else {
            return s;
        };
        let mut extra = BTreeMap::new();
        for (key, value) in map {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                extra.insert(key, value);
                continue;
            }
            s.apply(&key, &value);
        }
        s.extra = extra;
        s.clamp_and_migrate();
        s
    }

    fn apply(&mut self, key: &str, v: &Value) {
        let int = |v: &Value| -> Option<i64> {
            match v {
                // The Python loader takes floats and rejects bools for int fields.
                Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
                _ => None,
            }
        };
        let boolean = |v: &Value| -> Option<bool> { v.as_bool() };
        let text = |v: &Value| -> Option<String> { v.as_str().map(|s| s.to_string()) };

        match key {
            "clip_seconds" => self.clip_seconds = int(v).unwrap_or(self.clip_seconds as i64) as u32,
            "bitrate_mbps" => self.bitrate_mbps = int(v).unwrap_or(self.bitrate_mbps as i64) as u32,
            "fps" => self.fps = int(v).unwrap_or(self.fps as i64) as u32,
            "monitor" => self.monitor = int(v).unwrap_or(self.monitor as i64).max(0) as usize,
            "system_volume" => {
                self.system_volume = int(v).unwrap_or(self.system_volume as i64) as i32
            }
            "mic_volume" => self.mic_volume = int(v).unwrap_or(self.mic_volume as i64) as i32,
            "audio_offset_ms" => {
                self.audio_offset_ms = int(v).unwrap_or(self.audio_offset_ms as i64) as i32
            }
            "sound_volume" => self.sound_volume = int(v).unwrap_or(self.sound_volume as i64) as i32,
            "resolution" => {
                if let Some(t) = text(v) {
                    self.resolution = Resolution::parse(&t);
                }
            }
            "encoder" => {
                if let Some(t) = text(v) {
                    self.encoder = t;
                }
            }
            "codec" => {
                if let Some(t) = text(v) {
                    self.codec = Codec::parse(&t);
                }
            }
            "mic_device" => {
                if let Some(t) = text(v) {
                    self.mic_device = t;
                }
            }
            "output_dir" => {
                if let Some(t) = text(v) {
                    if !t.is_empty() {
                        self.output_dir = PathBuf::from(t);
                    }
                }
            }
            "screenshot_format" => {
                if let Some(t) = text(v) {
                    self.screenshot_format = if t == "jpg" {
                        ScreenshotFormat::Jpg
                    } else {
                        ScreenshotFormat::Png
                    };
                }
            }
            "sound_style" => {
                if let Some(t) = text(v) {
                    self.sound_style = t;
                }
            }
            "sound_custom" => {
                if let Some(t) = text(v) {
                    self.sound_custom = t;
                }
            }
            "capture_cursor" => self.capture_cursor = boolean(v).unwrap_or(self.capture_cursor),
            "system_audio" => self.system_audio = boolean(v).unwrap_or(self.system_audio),
            "mic_enabled" => self.mic_enabled = boolean(v).unwrap_or(self.mic_enabled),
            "screenshot_sound" => {
                self.screenshot_sound = boolean(v).unwrap_or(self.screenshot_sound)
            }
            "start_replay_on_launch" => {
                self.start_replay_on_launch = boolean(v).unwrap_or(self.start_replay_on_launch)
            }
            "copy_after_save" => self.copy_after_save = boolean(v).unwrap_or(self.copy_after_save),
            "sound_enabled" => self.sound_enabled = boolean(v).unwrap_or(self.sound_enabled),
            "toggle_sound" => self.toggle_sound = boolean(v).unwrap_or(self.toggle_sound),
            _ => {}
        }
    }

    fn clamp_and_migrate(&mut self) {
        self.clip_seconds = self.clip_seconds.clamp(15, 60);
        self.bitrate_mbps = self.bitrate_mbps.clamp(2, 200);
        self.system_volume = self.system_volume.clamp(0, 100);
        self.mic_volume = self.mic_volume.clamp(0, 100);
        self.sound_volume = self.sound_volume.clamp(0, 100);
        if self.fps == 0 {
            self.fps = 60;
        }
    }

    /// Hotkey bound to `action` (`toggle`, `clip`, `record`, `screenshot`, `open`).
    ///
    /// Hotkeys live in the passthrough map because the engine itself never uses them;
    /// the shell reads them through here so the JSON shape stays in one place.
    pub fn hotkey(&self, action: &str) -> Hotkey {
        let key = format!("hotkey_{action}");
        let fallback = match action {
            "toggle" => default_hotkeys().0,
            "clip" => default_hotkeys().1,
            "record" => default_hotkeys().2,
            "screenshot" => default_hotkeys().3,
            "open" => default_hotkeys().4,
            _ => default_hotkeys().0,
        };
        let Some(Value::Object(map)) = self.extra.get(&key) else {
            return fallback;
        };
        let mods = map
            .get("mods")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(fallback.mods);
        let vk = map
            .get("vk")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(fallback.vk);
        let text = map
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or(fallback.text);
        Hotkey { mods, vk, text }
    }

    /// Replace one action's hotkey, keeping the rest of the file untouched.
    pub fn set_hotkey(&mut self, action: &str, hotkey: &Hotkey) {
        let key = format!("hotkey_{action}");
        self.extra.insert(
            key,
            serde_json::json!({ "mods": hotkey.mods, "vk": hotkey.vk, "text": hotkey.text }),
        );
    }

    /// All five hotkeys, in the order the tray and settings page list them.
    pub fn hotkeys(&self) -> Vec<(String, Hotkey)> {
        ["toggle", "clip", "record", "screenshot", "open"]
            .iter()
            .map(|name| ((*name).to_string(), self.hotkey(name)))
            .collect()
    }

    /// Serialise back to the on-disk shape: known keys first, unknown keys preserved.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        for (k, v) in &self.extra {
            map.insert(k.clone(), v.clone());
        }
        macro_rules! set {
            ($k:literal, $v:expr) => {
                map.insert($k.to_string(), serde_json::json!($v));
            };
        }
        set!("clip_seconds", self.clip_seconds);
        set!("bitrate_mbps", self.bitrate_mbps);
        set!("fps", self.fps);
        set!("resolution", self.resolution.as_str());
        set!("monitor", self.monitor);
        set!("encoder", self.encoder);
        set!("codec", self.codec.as_str());
        set!("capture_cursor", self.capture_cursor);
        set!("system_audio", self.system_audio);
        set!("system_volume", self.system_volume);
        set!("mic_enabled", self.mic_enabled);
        set!("mic_device", self.mic_device);
        set!("mic_volume", self.mic_volume);
        set!("audio_offset_ms", self.audio_offset_ms);
        set!("output_dir", self.output_dir.to_string_lossy());
        set!("screenshot_format", self.screenshot_format.extension());
        set!("screenshot_sound", self.screenshot_sound);
        set!("start_replay_on_launch", self.start_replay_on_launch);
        set!("copy_after_save", self.copy_after_save);
        set!("sound_enabled", self.sound_enabled);
        set!("sound_style", self.sound_style);
        set!("sound_custom", self.sound_custom);
        set!("sound_volume", self.sound_volume);
        set!("toggle_sound", self.toggle_sound);
        Value::Object(map)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Settings::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let text = serde_json::to_vec_pretty(&self.to_json_value())?;
        write_atomic(path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_shipping_app() {
        let s = Settings::default();
        assert_eq!(s.clip_seconds, 30);
        assert_eq!(s.bitrate_mbps, 50);
        assert_eq!(s.fps, 60);
        assert_eq!(s.encoder, "auto");
        assert_eq!(s.codec, Codec::H264);
        assert!(s.system_audio);
        assert!(s.capture_cursor);
        assert!(!s.mic_enabled);
        assert!(s.output_dir.ends_with("Rewind"));
    }

    #[test]
    fn load_clamps_out_of_range_values() {
        let s = Settings::from_json(br#"{"clip_seconds": 600, "bitrate_mbps": 1, "fps": 0}"#);
        assert_eq!(s.clip_seconds, 60);
        assert_eq!(s.bitrate_mbps, 2);
        assert_eq!(s.fps, 60);

        let s = Settings::from_json(br#"{"clip_seconds": 1, "bitrate_mbps": 999}"#);
        assert_eq!(s.clip_seconds, 15);
        assert_eq!(s.bitrate_mbps, 200);
    }

    #[test]
    fn load_rejects_wrong_types_without_failing() {
        let s = Settings::from_json(
            br#"{"clip_seconds": "45", "capture_cursor": "yes", "fps": 120, "monitor": -3}"#,
        );
        assert_eq!(s.clip_seconds, 30, "a string is not an int");
        assert!(s.capture_cursor, "a string is not a bool");
        assert_eq!(s.fps, 120);
        assert_eq!(s.monitor, 0);
    }

    #[test]
    fn load_accepts_float_for_int_like_python() {
        let s = Settings::from_json(br#"{"fps": 59.94}"#);
        assert_eq!(s.fps, 59);
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let raw = br#"{"clip_seconds": 20, "overlay_corner": "top-left", "hotkey_clip": {"mods": 1}, "tutorial_done": true}"#;
        let s = Settings::from_json(raw);
        let out = s.to_json_value();
        assert_eq!(out["overlay_corner"], "top-left");
        assert_eq!(out["hotkey_clip"]["mods"], 1);
        assert_eq!(out["tutorial_done"], true);
        assert_eq!(out["clip_seconds"], 20);
    }

    #[test]
    fn save_then_load_keeps_values() {
        let dir = std::env::temp_dir().join(format!("rewind-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let s = Settings {
            clip_seconds: 45,
            codec: Codec::Hevc,
            resolution: Resolution::Height(1080),
            mic_enabled: true,
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        let back = Settings::load_from(&path).unwrap();
        assert_eq!(back.clip_seconds, 45);
        assert_eq!(back.codec, Codec::Hevc);
        assert_eq!(back.resolution, Resolution::Height(1080));
        assert!(back.mic_enabled);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_file_falls_back_to_defaults() {
        let s = Settings::from_json(b"not json at all");
        assert_eq!(s.clip_seconds, 30);
        let s = Settings::from_json(br#"[1,2,3]"#);
        assert_eq!(s.clip_seconds, 30);
    }

    #[test]
    fn hotkey_defaults_are_present_and_distinct() {
        let (toggle, clip, record, shot, open) = default_hotkeys();
        let all = [&toggle, &clip, &record, &shot, &open];
        for hk in all.iter() {
            assert!(!hk.text.is_empty());
            assert_ne!(hk.vk, 0);
            assert_ne!(hk.mods, 0);
        }
        let mut combos: Vec<_> = all.iter().map(|h| h.combo()).collect();
        combos.sort();
        combos.dedup();
        assert_eq!(combos.len(), 5, "default hotkeys must not collide");
    }
}
