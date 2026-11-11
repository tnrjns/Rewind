//! One-frame screen grabs for the screenshot hotkey.
//!
//! Same capture backends as the recorder (DXGI desktop duplication on Windows,
//! AVFoundation on macOS), so screenshots also work over games in exclusive
//! fullscreen where a normal window grab would come back black.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ffmpeg::run;
use crate::platform::avf_screen_input;
use crate::settings::ScreenshotFormat;

/// Capture one frame of a monitor to `out_path`.
///
/// Returns `true` only when FFmpeg succeeded *and* wrote a non-empty file.
pub fn take_screenshot(
    ffmpeg: &Path,
    monitor_idx: usize,
    out_path: &Path,
    cursor: bool,
    test_source: bool,
) -> bool {
    let jpg = out_path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        .unwrap_or(false);
    let mut quality: Vec<String> = if jpg {
        vec!["-q:v".into(), "2".into()]
    } else {
        vec!["-compression_level".into(), "3".into()]
    };
    let pix = if jpg { "yuvj444p" } else { "rgb24" };

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
    ];
    let filter: Vec<String> = if test_source {
        args.extend(
            ["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30"]
                .iter()
                .map(|s| s.to_string()),
        );
        vec!["-vf".into(), format!("format={pix}")]
    } else if cfg!(target_os = "macos") {
        let input = avf_screen_input(ffmpeg, monitor_idx);
        args.extend(
            [
                "-f",
                "avfoundation",
                "-capture_cursor",
                if cursor { "1" } else { "0" },
                "-framerate",
                "30",
                "-pixel_format",
                "bgr0",
                "-i",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        args.push(input);
        vec!["-vf".into(), format!("format={pix}")]
    } else {
        args.extend(
            [
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "ddagrab=output_idx={monitor_idx}:framerate=30:draw_mouse={}",
                    u8::from(cursor)
                ),
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        vec!["-vf".into(), format!("hwdownload,format=bgra,format={pix}")]
    };

    args.extend(["-frames:v".into(), "1".into()]);
    args.extend(filter);
    args.append(&mut quality);
    args.push(out_path.to_string_lossy().into_owned());

    let out = run(ffmpeg, &args, Duration::from_secs(15), None);
    out.ok() && file_size(out_path) > 0
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Build the output path for a new screenshot, honouring the configured format.
pub fn screenshot_path(dir: &Path, format: ScreenshotFormat) -> PathBuf {
    crate::paths::unique_path(dir, "Rewind_Screenshot_", format.extension())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_path_uses_the_configured_extension() {
        let dir = std::env::temp_dir().join(format!("rewind-shot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let png = screenshot_path(&dir, ScreenshotFormat::Png);
        assert_eq!(png.extension().unwrap(), "png");
        assert!(png
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("Rewind_Screenshot_"));
        let jpg = screenshot_path(&dir, ScreenshotFormat::Jpg);
        assert_eq!(jpg.extension().unwrap(), "jpg");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_binary_fails_cleanly_instead_of_panicking() {
        let dir = std::env::temp_dir().join(format!("rewind-shot-fail-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("shot.png");
        assert!(!take_screenshot(
            Path::new("/nonexistent/ffmpeg"),
            0,
            &out,
            true,
            true
        ));
        assert!(!out.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_source_produces_a_real_png() {
        let Some(ffmpeg) = crate::ffmpeg::find_ffmpeg() else {
            eprintln!("skipping: no ffmpeg on this machine");
            return;
        };
        let dir = std::env::temp_dir().join(format!("rewind-shot-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("shot.png");
        assert!(
            take_screenshot(&ffmpeg, 0, &out, true, true),
            "test source screenshot should succeed"
        );
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[1..4], b"PNG", "not a PNG file");
        assert!(bytes.len() > 1000, "suspiciously small: {}", bytes.len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_source_produces_a_real_jpeg() {
        let Some(ffmpeg) = crate::ffmpeg::find_ffmpeg() else {
            eprintln!("skipping: no ffmpeg on this machine");
            return;
        };
        let dir = std::env::temp_dir().join(format!("rewind-shot-jpg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("shot.jpg");
        assert!(take_screenshot(&ffmpeg, 0, &out, true, true));
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "not a JPEG file");
        let _ = fs::remove_dir_all(&dir);
    }
}
