//! Capture diagnostic: try every capture path on this machine and report what works.
//!
//! Windows capture has never run on Windows, and CI cannot cover it — GitHub's runners
//! have no desktop to duplicate. So this exists to be run by a human on a real machine
//! and answer the question precisely:
//!
//! ```text
//! cargo run -p rewind-engine --release --example capture_diagnostic
//! ```
//!
//! For each rung of the fallback ladder it prints the exact FFmpeg command, whether it
//! produced video, and FFmpeg's own words when it did not. It writes to a temp folder
//! and cleans up after itself.

use std::time::{Duration, Instant};

use rewind_engine::ffmpeg::{detect_encoders, find_ffmpeg, probe_duration, run};
use rewind_engine::recorder::{capture_plan, Attempt};
use rewind_engine::{list_monitors, Settings};

/// How long a rung gets to produce its first frames.
const WARMUP: Duration = Duration::from_secs(8);

fn main() {
    println!("Rewind capture diagnostic on {}", std::env::consts::OS);
    println!("------------------------------------------------");

    let Some(ffmpeg) = find_ffmpeg() else {
        println!("FAIL: FFmpeg was not found.");
        println!("  Windows: run install.bat, or put ffmpeg.exe in Rewind\\ffmpeg\\bin\\");
        println!("  macOS:   brew install ffmpeg, or let Rewind download its own copy");
        std::process::exit(1);
    };
    println!("ffmpeg: {}", ffmpeg.display());
    match run(
        &ffmpeg,
        &["-hide_banner".into(), "-version".into()],
        Duration::from_secs(20),
        None,
    ) {
        out if out.ok() => {
            let first = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            println!("version: {first}");
        }
        out => println!("FAIL: ffmpeg does not run ({})", out.stderr_tail(300)),
    }

    let monitors = list_monitors();
    println!("\nmonitors ({}):", monitors.len());
    for (i, m) in monitors.iter().enumerate() {
        println!(
            "  [{i}] {} {}x{} at ({},{}){}{}",
            m.name,
            m.w,
            m.h,
            m.x,
            m.y,
            if m.primary { " primary" } else { "" },
            if m.device.is_empty() {
                String::new()
            } else {
                format!(" device={}", m.device)
            },
        );
    }

    println!("\nencoders that work here:");
    for encoder in detect_encoders(Some(&ffmpeg)) {
        println!("  {} ({})", encoder.label(), encoder.key());
    }

    let settings = Settings::default();
    let encoders = detect_encoders(Some(&ffmpeg));
    let encoder = encoders
        .first()
        .copied()
        .unwrap_or(rewind_engine::EncoderKind::X264);
    println!("\nattempting each capture path with {}:", encoder.label());

    // Every rung, not just the first that works: the point is to see which of them are
    // healthy, because a silently broken hardware path is what makes a fallback matter.
    let mut passed = 0;
    let mut failed: Vec<String> = Vec::new();
    for (index, attempt) in capture_plan(&settings, &monitors, encoder, &ffmpeg, false)
        .into_iter()
        .enumerate()
    {
        match test_attempt(index, &attempt, &settings, &ffmpeg) {
            Ok(()) => passed += 1,
            Err(()) => failed.push(attempt.name.clone()),
        }
    }

    println!();
    if failed.is_empty() {
        println!("All {passed} capture path(s) worked.");
    } else {
        println!(
            "{passed} path(s) worked, {} failed: {}",
            failed.len(),
            failed.join(", ")
        );
        println!("The failure message above is FFmpeg's own reason, which names the fix.");
        std::process::exit(2);
    }
}

/// Returns whether this path produced video.
fn test_attempt(
    index: usize,
    attempt: &Attempt,
    settings: &Settings,
    ffmpeg: &std::path::Path,
) -> Result<(), ()> {
    let dir = std::env::temp_dir().join(format!("rewind-diag-{}-{index}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join("capture.mkv");
    let args = attempt.args(settings, &out);

    println!(
        "\n[{index}] {} ({} @ {} fps)",
        attempt.name, attempt.size, settings.fps
    );
    println!("    {}", args.join(" "));

    let started = Instant::now();
    let mut child = match std::process::Command::new(ffmpeg)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            println!("    FAIL: could not start ffmpeg: {e}");
            let _ = std::fs::remove_dir_all(&dir);
            return Err(());
        }
    };

    let deadline = Instant::now() + WARMUP;
    let mut produced = false;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        if size > 2048 {
            produced = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let outcome = if produced {
        // Let it run a moment so the duration is meaningful, then stop it cleanly.
        std::thread::sleep(Duration::from_millis(1500));
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            let _ = stdin.write_all(b"q");
            let _ = stdin.flush();
        }
        let _ = child.wait();
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let duration = probe_duration(ffmpeg, &out);
        println!(
            "    OK: {} bytes{} in {:.1}s",
            size,
            duration.map(|d| format!(", {d:.2}s")).unwrap_or_default(),
            started.elapsed().as_secs_f64()
        );
        Ok(())
    } else {
        let _ = child.kill();
        let output = child.wait_with_output().ok();
        let reason = output
            .as_ref()
            .map(|o| {
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join("\n      ")
            })
            .unwrap_or_default();
        if reason.trim().is_empty() {
            println!("    FAIL: exited with no frames and no message");
        } else {
            println!("    FAIL: no frames written. ffmpeg said:");
            println!("      {reason}");
        }
        Err(())
    };
    let _ = std::fs::remove_dir_all(&dir);
    outcome
}
