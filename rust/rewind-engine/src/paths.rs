//! Filesystem layout: where Rewind keeps settings, buffers, thumbnails and clips.
//!
//! Mirrors the Python app so an existing install keeps its settings and clips:
//! Windows `%LOCALAPPDATA%\Rewind`, macOS `~/Library/Application Support/Rewind`,
//! and everything else under `~/.local/share/Rewind`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const APP_NAME: &str = "Rewind";

/// Base folder for settings, logs, buffer and thumbnails.
pub fn data_dir() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home_dir)
    } else if cfg!(target_os = "macos") {
        home_dir().join("Library").join("Application Support")
    } else {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".local").join("share"))
    };
    base.join(APP_NAME)
}

/// A named sub-folder of [`data_dir`], created on demand.
pub fn sub_dir(name: &str) -> PathBuf {
    let d = data_dir().join(name);
    let _ = fs::create_dir_all(&d);
    d
}

/// Folder the app runs from, where a bundled FFmpeg and assets live.
///
/// `REWIND_BASE_DIR` wins when set (useful for tests and for the Python app shell
/// that launches the engine out of the repo); otherwise the executable's own folder,
/// matching what the frozen Python build did.
pub fn app_base_dir() -> PathBuf {
    if let Some(dir) = env::var_os("REWIND_BASE_DIR") {
        return PathBuf::from(dir);
    }
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn home_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(p) = env::var_os("USERPROFILE") {
            return PathBuf::from(p);
        }
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where clips and screenshots go by default: `Videos\Rewind` or `Movies/Rewind`.
pub fn default_clips_dir() -> PathBuf {
    let videos = if cfg!(target_os = "windows") {
        known_videos_dir().unwrap_or_else(|| home_dir().join("Videos"))
    } else {
        home_dir().join("Movies")
    };
    videos.join(APP_NAME)
}

#[cfg(target_os = "windows")]
fn known_videos_dir() -> Option<PathBuf> {
    use windows::core::PWSTR;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Videos, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

    unsafe {
        let raw: PWSTR = SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None).ok()?;
        let text = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const core::ffi::c_void));
        let text = text?;
        if text.is_empty() {
            None
        } else {
            Some(PathBuf::from(text))
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn known_videos_dir() -> Option<PathBuf> {
    None
}

/// Timestamp Rewind puts in every file name: `%Y-%m-%d_%H-%M-%S`, local time.
pub fn file_stamp() -> String {
    let local = local_now();
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        local.year, local.month, local.day, local.hour, local.minute, local.second
    )
}

/// A path in `dir` named `prefix<stamp><ext>`, with `_2`, `_3`… appended when it already exists.
pub fn unique_path(dir: &Path, prefix: &str, ext: &str) -> PathBuf {
    unique_path_at(dir, prefix, ext, file_stamp())
}

fn unique_path_at(dir: &Path, prefix: &str, ext: &str, stamp: String) -> PathBuf {
    let first = dir.join(format!("{prefix}{stamp}.{ext}"));
    if !first.exists() {
        return first;
    }
    let mut n = 2;
    loop {
        let candidate = dir.join(format!("{prefix}{stamp}_{n}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// `clip_trim.mp4`, then `clip_trim2.mp4`, `clip_trim3.mp4`… for repeated trims of one clip.
pub fn trim_target(video: &Path) -> PathBuf {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = video
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mp4".into());
    let dir = video.parent().unwrap_or(Path::new("."));
    let mut candidate = dir.join(format!("{stem}_trim.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}_trim{n}.{ext}"));
        n += 1;
    }
    candidate
}

/// Filename without its extension — the key Rewind uses in `clips.json` and for thumbnails.
pub fn file_key(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn thumb_path(video: &Path) -> PathBuf {
    sub_dir("thumbs").join(format!("{}.jpg", file_key(video)))
}

/// Open a file or folder in the OS file manager.
pub fn open_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

/// Reveal a file in the OS file manager, selecting it where supported.
pub fn reveal_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

/// Write `bytes` to `path` via a sibling temp file, so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp: OsString = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

// ------------------------------------------------------------------ time

const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Local wall-clock time, derived from UTC plus the platform's offset for *now*.
pub struct LocalTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub fn local_now() -> LocalTime {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    let offset = utc_offset_seconds(secs);
    let shifted = secs + offset;
    if offset == 0 {
        LocalTime {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: s,
        }
    } else {
        let (y, mo, d, h, mi, s) = civil_from_unix(shifted);
        LocalTime {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: s,
        }
    }
}

/// Seconds to add to UTC to get local time right now (DST included).
fn utc_offset_seconds(unix_secs: i64) -> i64 {
    #[cfg(unix)]
    unsafe {
        let t: libc::time_t = unix_secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
    #[cfg(not(unix))]
    {
        let _ = unix_secs;
        0
    }
}

/// Unix seconds → (year, month, day, hour, minute, second) in UTC (Howard Hinnant's algorithm).
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m as u32,
        d as u32,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// Days since the Unix epoch for a civil date, used by tests.
#[cfg(test)]
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[allow(dead_code)]
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
#[allow(dead_code)]
fn days_in_month(y: i64, m: u32) -> u32 {
    if m == 2 && is_leap(y) {
        29
    } else {
        MONTH_DAYS[(m - 1) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_known_dates() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 11, 10), (1999, 12, 31)] {
            let days = days_from_civil(y, m, d);
            let (ry, rm, rd, h, mi, s) = civil_from_unix(days * 86_400);
            assert_eq!((ry, rm, rd), (y, m, d));
            assert_eq!((h, mi, s), (0, 0, 0));
        }
    }

    #[test]
    fn civil_handles_time_of_day() {
        let secs = days_from_civil(2026, 11, 10) * 86_400 + 13 * 3600 + 45 * 60 + 7;
        let (y, m, d, h, mi, s) = civil_from_unix(secs);
        assert_eq!((y, m, d, h, mi, s), (2026, 11, 10, 13, 45, 7));
    }

    #[test]
    fn stamp_is_filename_safe() {
        let stamp = file_stamp();
        assert_eq!(stamp.len(), 19, "{stamp}");
        assert!(!stamp.contains(['/', '\\', ':', ' ']), "{stamp}");
        assert_eq!(stamp.as_bytes()[4], b'-');
        assert_eq!(stamp.as_bytes()[10], b'_');
    }

    #[test]
    fn unique_path_suffixes_when_taken() {
        let dir = env::temp_dir().join(format!("rewind-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let a = unique_path_at(&dir, "Rewind_", "mp4", "2026-01-01_00-00-00".into());
        assert_eq!(a.file_name().unwrap(), "Rewind_2026-01-01_00-00-00.mp4");
        fs::write(&a, b"x").unwrap();
        let b = unique_path_at(&dir, "Rewind_", "mp4", "2026-01-01_00-00-00".into());
        assert_eq!(b.file_name().unwrap(), "Rewind_2026-01-01_00-00-00_2.mp4");
        fs::write(&b, b"x").unwrap();
        let c = unique_path_at(&dir, "Rewind_", "mp4", "2026-01-01_00-00-00".into());
        assert_eq!(c.file_name().unwrap(), "Rewind_2026-01-01_00-00-00_3.mp4");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trim_target_counts_up() {
        let dir = env::temp_dir().join(format!("rewind-trim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Rewind_2026-01-01_00-00-00.mp4");
        fs::write(&video, b"x").unwrap();
        let t = trim_target(&video);
        assert_eq!(t, dir.join("Rewind_2026-01-01_00-00-00_trim.mp4"));
        fs::write(&t, b"x").unwrap();
        assert_eq!(
            trim_target(&video),
            dir.join("Rewind_2026-01-01_00-00-00_trim2.mp4")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_ends_with_app_name() {
        assert!(data_dir().ends_with(APP_NAME));
    }
}
