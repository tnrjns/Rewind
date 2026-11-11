//! Trim a clip: cut a span and save it as a new file next to the original.
//!
//! The cut is a stream copy — no re-encode, so it is instant and loses no quality.
//! Cuts therefore land on the nearest keyframe, which is one second in practice
//! because the recorder forces a keyframe every second.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ffmpeg::{probe_duration, run};
use crate::paths::trim_target;

#[derive(Debug, Clone, PartialEq)]
pub struct TrimResult {
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug)]
pub enum TrimError {
    TooShort,
    Failed(String),
}

impl std::fmt::Display for TrimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrimError::TooShort => write!(f, "Pick at least half a second to keep"),
            TrimError::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for TrimError {}

/// Shortest span the trim UI allows; below it the cut is not worth writing.
pub const MIN_SPAN: f64 = 0.5;

/// Normalise two handle positions into an ordered, in-bounds span.
///
/// Handles can be dragged past each other, so the smaller one always wins. The
/// span is clamped to the clip; `None` when the result is shorter than [`MIN_SPAN`].
pub fn span(a: f64, b: f64, duration: f64) -> Option<(f64, f64)> {
    let start = a.min(b).max(0.0);
    let end = a.max(b).min(duration.max(0.0));
    if end - start >= MIN_SPAN {
        Some((start, end))
    } else {
        None
    }
}

/// `m:ss.s` clock used next to the trim handles.
pub fn clock(secs: f64) -> String {
    let secs = secs.max(0.0);
    format!("{}:{:04.1}", (secs / 60.0) as u64, secs % 60.0)
}

/// Cut `start..end` out of `video` into a new `*_trim` file beside it.
pub fn trim_clip(
    ffmpeg: &Path,
    video: &Path,
    start: f64,
    end: f64,
) -> Result<TrimResult, TrimError> {
    let duration = probe_duration(ffmpeg, video).unwrap_or(f64::MAX);
    let Some((start, end)) = span(start, end, duration) else {
        return Err(TrimError::TooShort);
    };
    let out = trim_target(video);
    let result = run(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-ss".into(),
            format!("{start:.2}"),
            "-to".into(),
            format!("{end:.2}"),
            "-i".into(),
            video.to_string_lossy().into_owned(),
            "-map".into(),
            "0".into(),
            "-c".into(),
            "copy".into(),
            "-avoid_negative_ts".into(),
            "make_zero".into(),
            "-movflags".into(),
            "+faststart".into(),
            out.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(120),
        None,
    );
    let wrote_file = fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false);
    if !result.ok() || !wrote_file {
        let _ = fs::remove_file(&out);
        let detail = result.stderr_tail(400);
        return Err(TrimError::Failed(if detail.is_empty() {
            "Couldn't save the trimmed clip".into()
        } else {
            format!("Couldn't save the trimmed clip: {detail}")
        }));
    }
    Ok(TrimResult {
        path: out,
        start,
        end,
    })
}

/// Write a single still of the clip at `t` seconds, for the trim preview frames.
pub fn preview_frame(ffmpeg: &Path, video: &Path, t: f64, out: &Path) -> bool {
    let result = run(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-ss".into(),
            format!("{:.2}", t.max(0.0)),
            "-i".into(),
            video.to_string_lossy().into_owned(),
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            "scale=320:180:force_original_aspect_ratio=decrease".into(),
            out.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(20),
        None,
    );
    result.ok() && fs::metadata(out).map(|m| m.len() > 0).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_orders_reversed_handles() {
        assert_eq!(span(9.0, 3.0, 20.0), Some((3.0, 9.0)));
        assert_eq!(span(3.0, 9.0, 20.0), Some((3.0, 9.0)));
    }

    #[test]
    fn span_clamps_to_the_clip() {
        assert_eq!(span(-5.0, 100.0, 12.0), Some((0.0, 12.0)));
    }

    #[test]
    fn span_rejects_short_or_empty_selections() {
        assert_eq!(span(5.0, 5.2, 20.0), None);
        assert_eq!(span(5.0, 5.0, 20.0), None);
        assert_eq!(span(5.0, 5.5, 20.0), Some((5.0, 5.5)));
    }

    #[test]
    fn span_rejects_zero_length_clip() {
        assert_eq!(span(0.0, 10.0, 0.0), None);
    }

    #[test]
    fn clock_formats_minutes_and_tenths() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(9.4), "0:09.4");
        assert_eq!(clock(61.25), "1:01.2");
        assert_eq!(clock(-3.0), "0:00.0");
    }

    #[test]
    fn missing_file_reports_failure_and_writes_nothing() {
        let Some(ffmpeg) = crate::ffmpeg::find_ffmpeg() else {
            eprintln!("skipping: no ffmpeg on this machine");
            return;
        };
        let dir = std::env::temp_dir().join(format!("rewind-trim-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("missing.mp4");
        let err = trim_clip(&ffmpeg, &video, 0.0, 5.0).unwrap_err();
        assert!(matches!(err, TrimError::Failed(_)), "{err:?}");
        assert!(!dir.join("missing_trim.mp4").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trims_a_real_clip_and_keeps_the_original() {
        let Some(ffmpeg) = crate::ffmpeg::find_ffmpeg() else {
            eprintln!("skipping: no ffmpeg on this machine");
            return;
        };
        let dir = std::env::temp_dir().join(format!("rewind-trim-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Rewind_clip.mp4");
        // Six seconds of testsrc with a one-second keyframe interval, like the recorder makes.
        let made = run(
            &ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc2=size=320x180:rate=30".into(),
                "-t".into(),
                "6".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "ultrafast".into(),
                "-g".into(),
                "30".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                video.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(60),
            None,
        );
        assert!(
            made.ok(),
            "could not build fixture: {}",
            made.stderr_tail(400)
        );

        let result = trim_clip(&ffmpeg, &video, 1.0, 4.0).unwrap();
        assert!(result.path.exists());
        assert!(video.exists(), "the original must survive a trim");
        assert_eq!(result.path.file_name().unwrap(), "Rewind_clip_trim.mp4");
        let duration = probe_duration(&ffmpeg, &result.path).unwrap();
        assert!(
            (2.0..=4.1).contains(&duration),
            "trimmed clip is {duration}s, expected about 3"
        );

        // A second trim must not overwrite the first.
        let again = trim_clip(&ffmpeg, &video, 0.0, 2.0).unwrap();
        assert_eq!(again.path.file_name().unwrap(), "Rewind_clip_trim2.mp4");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_frame_writes_a_still() {
        let Some(ffmpeg) = crate::ffmpeg::find_ffmpeg() else {
            eprintln!("skipping: no ffmpeg on this machine");
            return;
        };
        let dir = std::env::temp_dir().join(format!("rewind-trim-frame-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("clip.mp4");
        let made = run(
            &ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc2=size=320x180:rate=30".into(),
                "-t".into(),
                "2".into(),
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                "ultrafast".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                video.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(60),
            None,
        );
        assert!(
            made.ok(),
            "could not build fixture: {}",
            made.stderr_tail(400)
        );
        let frame = dir.join("f_10.jpg");
        assert!(preview_frame(&ffmpeg, &video, 1.0, &frame));
        assert!(fs::metadata(&frame).unwrap().len() > 500);
        let _ = fs::remove_dir_all(&dir);
    }
}
