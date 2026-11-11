//! Pure formatting, filtering and rename rules for the Clips page.
//!
//! None of this touches egui, so the wording, the size ladder and the name sanitising
//! can be tested without a window — and stay identical to the Python app's.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};

use crate::controller::{ClipEntry, ClipKind};

/// `940 B`, `812 KB`, `3.4 MB`, `12.3 GB` — the same ladder the Python app uses.
pub fn fmt_size(bytes: u64) -> String {
    fn unit(n: f64, unit: &str) -> String {
        match (unit, n < 10.0) {
            ("B" | "KB", _) => format!("{n:.0} {unit}"),
            (_, true) => format!("{n:.1} {unit}"),
            (_, false) => format!("{n:.0} {unit}"),
        }
    }
    let mut n = bytes as f64;
    for step in ["B", "KB", "MB"] {
        if n < 1024.0 {
            return unit(n, step);
        }
        n /= 1024.0;
    }
    unit(n, "GB")
}

/// `0:07`, `1:23`; `—` when the duration is not known yet.
pub fn fmt_dur(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds else {
        return "—".to_string();
    };
    let total = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// `Nov 11, 06:04 PM`, in local time.
pub fn fmt_when(when: SystemTime) -> String {
    let secs = when
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    match Local.timestamp_opt(secs, 0).single() {
        Some(local) => local.format("%b %d, %I:%M %p").to_string(),
        None => String::new(),
    }
}

/// The detail line under a clip's name.
pub fn meta_text(entry: &ClipEntry) -> String {
    let lead = match entry.kind {
        ClipKind::Video => fmt_dur(entry.duration),
        ClipKind::Image => "Screenshot".to_string(),
    };
    format!(
        "{lead}  ·  {}  ·  {}",
        fmt_size(entry.size),
        fmt_when(entry.modified)
    )
}

/// A clip's display name: file name without its extension.
pub fn title(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Which items the page is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Clips,
    Screenshots,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Clips, Filter::Screenshots];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Clips => "Clips",
            Filter::Screenshots => "Screenshots",
        }
    }

    pub fn allows(self, kind: ClipKind) -> bool {
        match self {
            Filter::All => true,
            Filter::Clips => kind == ClipKind::Video,
            Filter::Screenshots => kind == ClipKind::Image,
        }
    }
}

/// Total size of what is on screen, for the footer line.
pub fn total_size(entries: &[ClipEntry]) -> u64 {
    entries.iter().map(|e| e.size).sum()
}

/// The name a rename should actually use, or `None` when nothing usable is left.
///
/// Windows refuses `< > : " / \ | ? *` in file names and both platforms dislike control
/// characters, so those become underscores rather than failing the rename after the user
/// has typed a long name.
pub fn clean_name(input: &str) -> Option<String> {
    let trimmed = input.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(
        trimmed
            .chars()
            .map(|c| {
                if "<>:\"/\\|?*".contains(c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect(),
    )
}

/// Where a rename of `path` to `input` would land, or why it cannot.
pub fn rename_target(path: &Path, input: &str) -> Result<PathBuf, String> {
    let Some(name) = clean_name(input) else {
        return Err("Enter a name".to_string());
    };
    let Some(parent) = path.parent() else {
        return Err("That file has no folder".to_string());
    };
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let target = parent.join(format!("{name}{ext}"));
    if target == path {
        return Err("That is already its name".to_string());
    }
    if target.exists() {
        return Err("A clip with that name already exists".to_string());
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_the_byte_kilobyte_megabyte_ladder() {
        assert_eq!(fmt_size(940), "940 B");
        assert_eq!(fmt_size(1024), "1 KB");
        assert_eq!(fmt_size(1536), "2 KB");
        assert_eq!(fmt_size(3 * 1024 * 1024 + 400_000), "3.4 MB");
        assert_eq!(fmt_size(60 * 1024 * 1024), "60 MB");
        assert_eq!(fmt_size(12 * 1024 * 1024 * 1024), "12 GB");
    }

    #[test]
    fn durations_are_minutes_and_seconds() {
        assert_eq!(fmt_dur(Some(7.4)), "0:07");
        assert_eq!(fmt_dur(Some(83.0)), "1:23");
        assert_eq!(fmt_dur(None), "—");
    }

    #[test]
    fn filters_split_clips_from_screenshots() {
        assert!(Filter::All.allows(ClipKind::Video));
        assert!(Filter::All.allows(ClipKind::Image));
        assert!(Filter::Clips.allows(ClipKind::Video));
        assert!(!Filter::Clips.allows(ClipKind::Image));
        assert!(Filter::Screenshots.allows(ClipKind::Image));
        assert!(!Filter::Screenshots.allows(ClipKind::Video));
    }

    #[test]
    fn a_rename_keeps_the_extension_and_replaces_the_stem() {
        let target = rename_target(Path::new("/clips/Rewind_1.mp4"), "Boss fight").unwrap();
        assert_eq!(target, PathBuf::from("/clips/Boss fight.mp4"));
    }

    #[test]
    fn a_rename_to_the_same_name_is_refused() {
        let err = rename_target(Path::new("/clips/old.mp4"), "old").unwrap_err();
        assert!(err.contains("already"), "{err}");
    }

    #[test]
    fn forbidden_characters_become_underscores() {
        assert_eq!(clean_name("a/b:c*?").as_deref(), Some("a_b_c__"));
        assert_eq!(clean_name("   ").as_deref(), None);
        assert_eq!(clean_name("...").as_deref(), None);
    }

    #[test]
    fn an_empty_rename_is_refused_before_touching_the_filesystem() {
        assert!(rename_target(Path::new("/clips/old.mp4"), "  ").is_err());
    }
}
