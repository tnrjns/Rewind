//! Dump a ScreenCaptureKit capture to a raw NV12 file.
//!
//! ```text
//! cargo run -p rewind-engine --example sck_capture -- <display-id> <seconds> <out.raw> [exclude-title]
//! ```
//!
//! Frames are 1920×1080 unless the display reports another size. Play the result
//! back with `ffmpeg -f rawvideo -pix_fmt nv12 -s <w>x<h> -framerate 60 -i out.raw`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use rewind_engine::platform::list_monitors;
use rewind_engine::sck::Capture;

const FPS: u32 = 60;
const FALLBACK_WIDTH: u32 = 1920;
const FALLBACK_HEIGHT: u32 = 1080;

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: sck_capture <display-id> <seconds> <out.raw> [exclude-title]";
    let display: u32 = args
        .next()
        .ok_or(usage)?
        .parse()
        .map_err(|_| format!("invalid display id\n{usage}"))?;
    let seconds: f64 = args
        .next()
        .ok_or(usage)?
        .parse()
        .map_err(|_| format!("invalid duration\n{usage}"))?;
    let out = args.next().ok_or(usage)?;
    let exclude = args.next();
    if seconds <= 0.0 {
        return Err("duration must be positive".to_string());
    }

    let monitors = list_monitors();
    for (i, m) in monitors.iter().enumerate() {
        println!(
            "#{i}: {} {}x{} at ({}, {}){}{}",
            m.name,
            m.w,
            m.h,
            m.x,
            m.y,
            if m.primary { " primary" } else { "" },
            if m.device.is_empty() {
                String::new()
            } else {
                format!(" id={}", m.device)
            }
        );
    }
    // The macOS monitor list carries the CGDirectDisplayID in `device`.
    let monitor = monitors.iter().find(|m| m.device == display.to_string());
    let (width, height) = monitor
        .map(|m| (m.w, m.h))
        .unwrap_or((FALLBACK_WIDTH, FALLBACK_HEIGHT));

    println!(
        "capturing display {display} at {width}x{height}, {FPS} fps for {seconds}s{}",
        match &exclude {
            Some(title) => format!(", excluding window {title:?}"),
            None => String::new(),
        }
    );

    let file = File::create(&out).map_err(|e| format!("cannot create {out}: {e}"))?;
    let sink = Box::new(BufWriter::new(file)) as Box<dyn Write + Send>;

    let started = Instant::now();
    let capture = Capture::start(sink, display, width, height, FPS, true, exclude)?;
    std::thread::sleep(Duration::from_secs_f64(seconds));
    capture.stop();
    let elapsed = started.elapsed();

    let size = std::fs::metadata(&out)
        .map_err(|e| format!("cannot stat {out}: {e}"))?
        .len();
    let frame_bytes = u64::from(width) * u64::from(height) * 3 / 2;
    println!(
        "wrote {size} bytes ({:.2} frames of {frame_bytes}) in {:.2}s",
        size as f64 / frame_bytes as f64,
        elapsed.as_secs_f64()
    );
    Ok(())
}
