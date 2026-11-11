//! FFmpeg discovery, process plumbing, encoder selection and media probing.

use std::ffi::OsStr;
use std::fs;
#[cfg(target_os = "macos")]
use std::io::Write;
use std::io::{self};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::paths::{data_dir, sub_dir};

/// Static macOS FFmpeg build, same source the Python app downloads from.
#[cfg(target_os = "macos")]
pub const MAC_FFMPEG_URL: &str =
    "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderKind {
    Nvenc,
    Amf,
    Qsv,
    VideoToolbox,
    X264,
}

impl EncoderKind {
    pub fn key(self) -> &'static str {
        match self {
            EncoderKind::Nvenc => "nvenc",
            EncoderKind::Amf => "amf",
            EncoderKind::Qsv => "qsv",
            EncoderKind::VideoToolbox => "vt",
            EncoderKind::X264 => "x264",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EncoderKind::Nvenc => "NVIDIA NVENC",
            EncoderKind::Amf => "AMD AMF",
            EncoderKind::Qsv => "Intel Quick Sync",
            EncoderKind::VideoToolbox => "Apple VideoToolbox",
            EncoderKind::X264 => "CPU (x264)",
        }
    }

    pub fn from_key(key: &str) -> Option<EncoderKind> {
        Some(match key {
            "nvenc" => EncoderKind::Nvenc,
            "amf" => EncoderKind::Amf,
            "qsv" => EncoderKind::Qsv,
            "vt" => EncoderKind::VideoToolbox,
            "x264" => EncoderKind::X264,
            _ => return None,
        })
    }

    /// FFmpeg encoder name for this hardware/library at the requested codec.
    pub fn ffmpeg_name(self, codec: crate::settings::Codec) -> &'static str {
        use crate::settings::Codec;
        match (self, codec) {
            (EncoderKind::Nvenc, Codec::H264) => "h264_nvenc",
            (EncoderKind::Nvenc, Codec::Hevc) => "hevc_nvenc",
            (EncoderKind::Amf, Codec::H264) => "h264_amf",
            (EncoderKind::Amf, Codec::Hevc) => "hevc_amf",
            (EncoderKind::Qsv, Codec::H264) => "h264_qsv",
            (EncoderKind::Qsv, Codec::Hevc) => "hevc_qsv",
            (EncoderKind::VideoToolbox, Codec::H264) => "h264_videotoolbox",
            (EncoderKind::VideoToolbox, Codec::Hevc) => "hevc_videotoolbox",
            (EncoderKind::X264, Codec::H264) => "libx264",
            (EncoderKind::X264, Codec::Hevc) => "libx265",
        }
    }
}

/// Result of a finished child process, mirroring `subprocess.CompletedProcess`.
#[derive(Debug, Clone)]
pub struct Output {
    /// Exit code, `124` on timeout, `127` when the binary could not be executed.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == 0
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Last `n` bytes of stderr, for log lines.
    pub fn stderr_tail(&self, n: usize) -> String {
        let text = self.stderr_lossy();
        let start = text.len().saturating_sub(n);
        // Do not split a UTF-8 sequence: lossy decode already guarantees a char boundary grid.
        let mut idx = start;
        while idx < text.len() && !text.is_char_boundary(idx) {
            idx += 1;
        }
        text[idx..].to_string()
    }
}

/// Run a command with a timeout, capturing both pipes. Never fails: a missing binary
/// yields status 127 and a timeout yields 124, exactly like the Python helper.
pub fn run<S: AsRef<OsStr>>(
    program: S,
    args: &[String],
    timeout: Duration,
    cwd: Option<&Path>,
) -> Output {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    configure_child(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Output {
                status: 127,
                stdout: Vec::new(),
                stderr: e.to_string().into_bytes(),
            };
        }
    };

    // Drain pipes on threads so a chatty child can never block on a full pipe buffer.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = io::Read::read_to_end(p, &mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = io::Read::read_to_end(p, &mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    if timed_out {
        let mut err = stderr;
        err.extend_from_slice(b"\ntimeout");
        return Output {
            status: 124,
            stdout,
            stderr: err,
        };
    }
    let code = status.and_then(|s| s.code()).unwrap_or(0);
    Output {
        status: code,
        stdout,
        stderr,
    }
}

#[cfg(target_os = "windows")]
fn configure_child(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`: no console flashes when FFmpeg starts from the tray app.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_child(_cmd: &mut Command) {}

/// Candidate locations for the FFmpeg binary, most specific first.
pub fn ffmpeg_candidates(base: &Path) -> Vec<PathBuf> {
    let exe = if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let mut out = Vec::new();
    for rel in [vec!["ffmpeg", "bin"], vec!["ffmpeg"], vec!["bin"], vec![]] {
        let mut p = base.to_path_buf();
        for part in rel {
            if !part.is_empty() {
                p.push(part);
            }
        }
        out.push(p.join(exe));
    }
    out
}

/// Find FFmpeg next to the app first (bundled copies win), then on `PATH`, then in
/// the well-known macOS/Windows install spots.
pub fn find_ffmpeg() -> Option<PathBuf> {
    for base in [crate::paths::app_base_dir(), data_dir()] {
        for candidate in ffmpeg_candidates(&base) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(found) = which("ffmpeg") {
        return Some(found);
    }
    if cfg!(target_os = "macos") {
        // Apps started from Finder get a bare PATH, so look where Homebrew puts it.
        let own = sub_dir("ffmpeg").join("ffmpeg");
        if own.is_file() && is_executable(&own) {
            return Some(own);
        }
        for candidate in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
            let p = PathBuf::from(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if cfg!(target_os = "windows") {
        // WinGet installs under LocalAppData; glob the package folder without a glob crate.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let root = PathBuf::from(local)
                .join("Microsoft")
                .join("WinGet")
                .join("Packages");
            if let Ok(entries) = fs::read_dir(&root) {
                let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
                names.sort();
                for package in names {
                    if !package
                        .file_name()
                        .map(|n| n.to_string_lossy().to_ascii_lowercase().contains("ffmpeg"))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    for sub in fs::read_dir(&package).into_iter().flatten().flatten() {
                        let exe = sub.path().join("bin").join("ffmpeg.exe");
                        if exe.is_file() {
                            return Some(exe);
                        }
                    }
                }
            }
        }
    }
    None
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = if cfg!(target_os = "windows") {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(&exe))
        .find(|c| c.is_file())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Download a static FFmpeg into `Rewind/ffmpeg`, the way the Windows installer does.
///
/// Returns the binary path only if the downloaded build actually runs. On Windows the
/// installer supplies FFmpeg, so there is nothing to fetch.
#[cfg(target_os = "macos")]
pub fn download_ffmpeg_mac<F: FnMut(&str)>(mut log: F) -> Option<PathBuf> {
    let dest = sub_dir("ffmpeg");
    let zip_path = dest.join("ffmpeg.zip");
    let result = (|| -> Option<PathBuf> {
        let bytes: Vec<u8> = {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(120)))
                .build();
            let agent: ureq::Agent = config.into();
            let mut resp = agent
                .get(MAC_FFMPEG_URL)
                .header("User-Agent", "Rewind")
                .call()
                .ok()?;
            resp.body_mut().read_to_vec().ok()?
        };
        fs::write(&zip_path, &bytes).ok()?;

        let file = fs::File::open(&zip_path).ok()?;
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let mut entry = archive.by_name("ffmpeg").ok()?;
        let mut out = fs::File::create(dest.join("ffmpeg")).ok()?;
        io::copy(&mut entry, &mut out).ok()?;
        out.flush().ok()?;
        drop(out);

        let path = dest.join("ffmpeg");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).ok()?;
        }
        let ok = run(
            &path,
            &["-hide_banner".into(), "-version".into()],
            Duration::from_secs(30),
            None,
        )
        .ok();
        if ok {
            Some(path)
        } else {
            let _ = fs::remove_file(&path);
            None
        }
    })();
    if result.is_none() {
        log("ffmpeg download failed");
    }
    let _ = fs::remove_file(&zip_path);
    result
}

#[cfg(not(target_os = "macos"))]
pub fn download_ffmpeg_mac<F: FnMut(&str)>(_log: F) -> Option<PathBuf> {
    None
}

/// Media duration in seconds from FFmpeg's banner, or `None` when it cannot be read.
pub fn probe_duration(ffmpeg: &Path, path: &Path) -> Option<f64> {
    let out = run(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-i".into(),
            path.to_string_lossy().into_owned(),
        ],
        Duration::from_secs(20),
        None,
    );
    parse_duration(&String::from_utf8_lossy(&out.stderr))
}

/// Parse `Duration: HH:MM:SS.ss` out of an FFmpeg banner.
pub fn parse_duration(stderr: &str) -> Option<f64> {
    let idx = stderr.find("Duration:")?;
    let rest = stderr[idx + "Duration:".len()..].trim_start();
    let mut parts = rest.splitn(3, ':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let sec_text: String = parts
        .next()?
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let s: f64 = sec_text.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Encoder flags: constant bitrate, a keyframe every second so clips cut cleanly,
/// and no B-frames where the encoder allows it (they add latency to a live capture).
pub fn encoder_args(
    kind: EncoderKind,
    codec: crate::settings::Codec,
    mbps: u32,
    fps: u32,
) -> Vec<String> {
    use crate::settings::Codec;
    let name = kind.ffmpeg_name(codec);
    let rate = format!("{mbps}M");
    let mut common: Vec<String> = vec![
        "-b:v".into(),
        rate.clone(),
        "-maxrate".into(),
        rate,
        "-bufsize".into(),
        format!("{}M", mbps * 2),
        "-g".into(),
        fps.to_string(),
        "-force_key_frames".into(),
        "expr:gte(t,n_forced*1)".into(),
    ];
    let mut head: Vec<String> = match kind {
        EncoderKind::Nvenc => vec![
            "-c:v".into(),
            name.into(),
            "-preset".into(),
            "p4".into(),
            "-rc".into(),
            "cbr".into(),
            "-forced-idr".into(),
            "1".into(),
            "-bf".into(),
            "0".into(),
        ],
        EncoderKind::Amf => vec![
            "-c:v".into(),
            name.into(),
            "-usage".into(),
            "transcoding".into(),
            "-quality".into(),
            "balanced".into(),
            "-rc".into(),
            "cbr".into(),
        ],
        EncoderKind::Qsv => vec![
            "-c:v".into(),
            name.into(),
            "-preset".into(),
            "veryfast".into(),
        ],
        EncoderKind::VideoToolbox => vec![
            "-c:v".into(),
            name.into(),
            "-realtime".into(),
            "1".into(),
            "-bf".into(),
            "0".into(),
        ],
        EncoderKind::X264 => match codec {
            Codec::Hevc => vec![
                "-c:v".into(),
                name.into(),
                "-preset".into(),
                "ultrafast".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
                "-x265-params".into(),
                "log-level=error".into(),
            ],
            Codec::H264 => vec![
                "-c:v".into(),
                name.into(),
                "-preset".into(),
                "veryfast".into(),
                "-pix_fmt".into(),
                "yuv420p".into(),
            ],
        },
    };
    head.append(&mut common);
    head
}

/// Which encoders this machine can actually run, hardware first, `x264` always last.
pub fn detect_encoders(ffmpeg: Option<&Path>) -> Vec<EncoderKind> {
    let mut found = Vec::new();
    if let Some(ff) = ffmpeg {
        let order: &[EncoderKind] = if cfg!(target_os = "macos") {
            &[EncoderKind::VideoToolbox]
        } else {
            &[EncoderKind::Nvenc, EncoderKind::Amf, EncoderKind::Qsv]
        };
        for kind in order {
            let args: Vec<String> = vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "color=black:s=640x360:r=30".into(),
                "-frames:v".into(),
                "3".into(),
                "-c:v".into(),
                kind.ffmpeg_name(crate::settings::Codec::H264).into(),
                "-f".into(),
                "null".into(),
                "-".into(),
            ];
            if run(ff, &args, Duration::from_secs(20), None).ok() {
                found.push(*kind);
            }
        }
    }
    found.push(EncoderKind::X264);
    found
}

/// Honour the user's encoder choice when this machine has it, else take the best found.
pub fn resolve_encoder(preferred: &str, available: &[EncoderKind]) -> EncoderKind {
    if preferred != "auto" {
        if let Some(kind) = EncoderKind::from_key(preferred) {
            if available.contains(&kind) {
                return kind;
            }
        }
    }
    available.first().copied().unwrap_or(EncoderKind::X264)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Codec;

    #[test]
    fn parses_duration_from_ffmpeg_banner() {
        let banner = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'clip.mp4':\n  Duration: 00:01:07.52, start: 0.000000, bitrate: 8100 kb/s\n";
        assert_eq!(parse_duration(banner), Some(67.52));
    }

    #[test]
    fn parses_hour_long_duration() {
        let banner = "Duration: 02:03:04.50, start: 0.0";
        assert_eq!(parse_duration(banner), Some(7384.5));
    }

    #[test]
    fn missing_duration_is_none() {
        assert_eq!(parse_duration("no banner here"), None);
        assert_eq!(parse_duration("Duration: N/A, start: 0.0"), None);
    }

    #[test]
    fn encoder_names_follow_codec() {
        assert_eq!(EncoderKind::Nvenc.ffmpeg_name(Codec::H264), "h264_nvenc");
        assert_eq!(EncoderKind::Nvenc.ffmpeg_name(Codec::Hevc), "hevc_nvenc");
        assert_eq!(
            EncoderKind::VideoToolbox.ffmpeg_name(Codec::Hevc),
            "hevc_videotoolbox"
        );
        assert_eq!(EncoderKind::X264.ffmpeg_name(Codec::Hevc), "libx265");
    }

    #[test]
    fn keys_round_trip() {
        for kind in [
            EncoderKind::Nvenc,
            EncoderKind::Amf,
            EncoderKind::Qsv,
            EncoderKind::VideoToolbox,
            EncoderKind::X264,
        ] {
            assert_eq!(EncoderKind::from_key(kind.key()), Some(kind));
        }
        assert_eq!(EncoderKind::from_key("auto"), None);
    }

    #[test]
    fn encoder_args_are_constant_bitrate_with_one_second_keyframes() {
        let args = encoder_args(EncoderKind::VideoToolbox, Codec::H264, 50, 60);
        let joined = args.join(" ");
        assert!(joined.contains("-c:v h264_videotoolbox"));
        assert!(joined.contains("-b:v 50M"));
        assert!(joined.contains("-maxrate 50M"));
        assert!(joined.contains("-bufsize 100M"));
        assert!(joined.contains("-g 60"));
        assert!(joined.contains("-force_key_frames expr:gte(t,n_forced*1)"));
    }

    #[test]
    fn x265_gets_pixel_format_and_quiet_logging() {
        let args = encoder_args(EncoderKind::X264, Codec::Hevc, 20, 30);
        let joined = args.join(" ");
        assert!(joined.contains("-c:v libx265"));
        assert!(joined.contains("-pix_fmt yuv420p"));
        assert!(joined.contains("-x265-params log-level=error"));
    }

    #[test]
    fn resolve_prefers_choice_then_best_available() {
        let available = [EncoderKind::VideoToolbox, EncoderKind::X264];
        assert_eq!(
            resolve_encoder("auto", &available),
            EncoderKind::VideoToolbox
        );
        assert_eq!(resolve_encoder("x264", &available), EncoderKind::X264);
        assert_eq!(
            resolve_encoder("nvenc", &available),
            EncoderKind::VideoToolbox,
            "a missing choice falls back to the best detected"
        );
    }

    #[test]
    fn missing_binary_reports_127() {
        let out = run(
            "/nonexistent/definitely-not-ffmpeg",
            &[],
            Duration::from_secs(1),
            None,
        );
        assert_eq!(out.status, 127);
    }

    #[test]
    fn timeout_reports_124() {
        // A process that outlives the timeout must be killed and reported as 124, the
        // same code the Python helper used. Needs a command that exists on every
        // platform the engine builds for, so it sleeps using the shell each OS has.
        #[cfg(target_os = "windows")]
        let (program, args) = (
            "cmd",
            vec!["/C".to_string(), "ping -n 30 127.0.0.1 > nul".to_string()],
        );
        #[cfg(not(target_os = "windows"))]
        let (program, args) = ("/bin/sh", vec!["-c".to_string(), "sleep 30".to_string()]);

        let started = std::time::Instant::now();
        let out = run(program, &args, Duration::from_millis(400), None);
        assert_eq!(out.status, 124, "stderr: {}", out.stderr_lossy());
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the child was not killed at the deadline"
        );
    }
}
