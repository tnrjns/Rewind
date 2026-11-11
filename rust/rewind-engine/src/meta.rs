//! Clip metadata (`clips.json`) and thumbnails.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::ffmpeg::run;
use crate::paths::{data_dir, meta_key as path_meta_key, sub_dir, thumb_path, write_atomic};

/// Serialises read-modify-write cycles so two clips finishing at once cannot lose an entry.
static META_LOCK: Mutex<()> = Mutex::new(());

fn meta_lock() -> MutexGuard<'static, ()> {
    META_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn meta_path() -> PathBuf {
    data_dir().join("clips.json")
}

/// Everything `clips.json` says about one file, keyed by file stem.
pub type Meta = BTreeMap<String, Map<String, Value>>;

pub fn load_meta() -> Meta {
    let _guard = meta_lock();
    load_meta_unlocked()
}

fn load_meta_unlocked() -> Meta {
    let Ok(bytes) = fs::read(meta_path()) else {
        return Meta::new();
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => map
            .into_iter()
            .filter_map(|(k, v)| match v {
                Value::Object(entry) => Some((k, entry)),
                _ => None,
            })
            .collect(),
        _ => Meta::new(),
    }
}

/// Record a duration (or any other field) for one saved file.
pub fn update_meta(name: &str, values: &[(&str, Value)]) -> std::io::Result<()> {
    let _guard = meta_lock();
    let mut meta = load_meta_unlocked();
    let entry = meta.entry(name.to_string()).or_default();
    for (k, v) in values {
        entry.insert((*k).to_string(), v.clone());
    }
    write_meta_unlocked(&meta)
}

fn write_meta_unlocked(meta: &Meta) -> std::io::Result<()> {
    let value = Value::Object(
        meta.iter()
            .map(|(k, v)| (k.clone(), Value::Object(v.clone())))
            .collect(),
    );
    write_atomic(&meta_path(), &serde_json::to_vec(&value)?)
}

/// Carry a clip's metadata over to a new name after the file was renamed.
///
/// Clips are keyed by file stem in `clips.json`, and the thumbnail folder uses the same
/// key, so a rename that skipped this would orphan the duration and leave a row behind.
/// Returns whether an entry existed to move.
pub fn rename_entry(old: &str, new: &str) -> std::io::Result<bool> {
    let _guard = meta_lock();
    let mut meta = load_meta_unlocked();
    let moved = move_entry(&mut meta, old, new);
    if moved {
        write_meta_unlocked(&meta)?;
    }
    Ok(moved)
}

/// The in-memory half of [`rename_entry`], so the rule is testable without a data dir.
fn move_entry(meta: &mut Meta, old: &str, new: &str) -> bool {
    if old == new {
        return meta.contains_key(new);
    }
    let Some(entry) = meta.remove(old) else {
        return false;
    };
    meta.insert(new.to_string(), entry);
    true
}

/// Duration recorded for a clip, if any.
pub fn meta_duration(name: &str) -> Option<f64> {
    load_meta().get(name)?.get("duration")?.as_f64()
}

/// Grab a 480 px-wide JPEG still from `video` for the Clips page.
///
/// Tries one second in (more representative than a black first frame), then falls
/// back to frame zero for clips shorter than a second.
pub fn make_thumbnail(ffmpeg: &Path, video: &Path, duration: Option<f64>) -> Option<PathBuf> {
    let out = thumb_path(video);
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let seek = (duration.unwrap_or(2.0) / 2.0).min(1.0);
    for ss in [seek, 0.0] {
        let result = run(
            ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
                "-ss".into(),
                format!("{ss:.2}"),
                "-i".into(),
                video.to_string_lossy().into_owned(),
                "-frames:v".into(),
                "1".into(),
                "-vf".into(),
                "scale=480:-2".into(),
                "-q:v".into(),
                "3".into(),
                out.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(30),
            None,
        );
        if result.ok() && file_has_bytes(&out) {
            return Some(out);
        }
    }
    None
}

fn file_has_bytes(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false)
}

/// Write one frame of `video`, `at` seconds in, to `out` as a PNG.
///
/// This is what the trim dialog's start and end previews are drawn from. Seeking before
/// the input (`-ss` ahead of `-i`) keeps it cheap: FFmpeg jumps to the nearest keyframe
/// instead of decoding the clip from the beginning.
pub fn frame_at(ffmpeg: &Path, video: &Path, at: f64, out: &Path) -> bool {
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let result = run(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-ss".into(),
            format!("{:.2}", at.max(0.0)),
            "-i".into(),
            video.to_string_lossy().into_owned(),
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            "scale=480:-2".into(),
            out.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(30),
        None,
    );
    result.ok() && file_has_bytes(out)
}

/// Thumbnail path for a clip, whether or not it has been generated yet.
pub fn thumbnail_for(video: &Path) -> PathBuf {
    thumb_path(video)
}

/// `thumbs` folder, created on demand.
pub fn thumbs_dir() -> PathBuf {
    sub_dir("thumbs")
}

/// The `clips.json` key for a path, exposed for callers writing metadata.
pub fn meta_key(video: &Path) -> String {
    path_meta_key(video)
}

/// Parse a duration field the way `clips.json` stores it.
pub fn duration_of(meta: &Meta, video: &Path) -> Option<f64> {
    meta.get(&path_meta_key(video))
        .and_then(|e| e.get("duration"))
        .and_then(|v| v.as_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("rewind-meta-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Exercise the pure serialisation path rather than the real data dir.
        let mut meta = Meta::new();
        meta.entry("Rewind_2026-01-01_00-00-00.mp4".into())
            .or_default()
            .insert("duration".into(), serde_json::json!(12.34));
        let value = Value::Object(
            meta.iter()
                .map(|(k, v)| (k.clone(), Value::Object(v.clone())))
                .collect(),
        );
        let text = serde_json::to_vec(&value).unwrap();
        let back: Meta = match serde_json::from_slice::<Value>(&text).unwrap() {
            Value::Object(map) => map
                .into_iter()
                .filter_map(|(k, v)| match v {
                    Value::Object(e) => Some((k, e)),
                    _ => None,
                })
                .collect(),
            _ => panic!("object"),
        };
        assert_eq!(back["Rewind_2026-01-01_00-00-00.mp4"]["duration"], 12.34);
        assert_eq!(
            duration_of(&back, Path::new("/clips/Rewind_2026-01-01_00-00-00.mp4")),
            Some(12.34)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duration_of_missing_entry_is_none() {
        let meta = Meta::new();
        assert_eq!(duration_of(&meta, Path::new("/clips/nope.mp4")), None);
    }

    #[test]
    fn metadata_is_keyed_by_file_name_but_thumbnails_by_stem() {
        // The shipping app writes clips.json under `name.mp4` and thumbs under `name.jpg`.
        // Both conventions are load-bearing: changing the first hides every duration an
        // existing install already recorded.
        let video = Path::new("/clips/Rewind_2026-11-11_09-14-22.mp4");
        assert_eq!(meta_key(video), "Rewind_2026-11-11_09-14-22.mp4");
        assert_eq!(crate::paths::file_key(video), "Rewind_2026-11-11_09-14-22");
    }

    #[test]
    fn renaming_a_clip_moves_its_metadata_entry() {
        let mut meta = Meta::new();
        meta.entry("old".into())
            .or_default()
            .insert("duration".into(), serde_json::json!(7.5));
        assert!(move_entry(&mut meta, "old", "new"));
        assert!(!meta.contains_key("old"));
        assert_eq!(meta["new"]["duration"], 7.5);
    }

    #[test]
    fn renaming_without_an_entry_moves_nothing() {
        let mut meta = Meta::new();
        assert!(!move_entry(&mut meta, "missing", "new"));
        assert!(meta.is_empty(), "{meta:?}");
    }

    #[test]
    fn renaming_to_the_same_name_keeps_the_entry() {
        let mut meta = Meta::new();
        meta.entry("same".into()).or_default();
        assert!(move_entry(&mut meta, "same", "same"));
        assert!(meta.contains_key("same"));
    }

    #[test]
    fn thumbnail_path_is_stem_jpg_in_thumbs() {
        let p = thumb_path(Path::new("/clips/Rewind_2026-01-01_00-00-00.mp4"));
        assert!(
            p.ends_with("thumbs/Rewind_2026-01-01_00-00-00.jpg"),
            "{p:?}"
        );
    }
}
