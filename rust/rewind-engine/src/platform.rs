//! Host display enumeration and screen-capture permission checks.
//!
//! Monitor geometry is reported in physical pixels, in the same order FFmpeg's
//! `ddagrab` (Windows) and `avfoundation` (macOS) number the displays, because the
//! capture command lines index into it.

use std::path::Path;
use std::time::Duration;

use crate::ffmpeg::run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub primary: bool,
    /// Windows device name (`\\.\DISPLAY1`); on macOS the CGDirectDisplayID.
    pub device: String,
}

impl Monitor {
    fn synthetic(index: usize) -> Monitor {
        Monitor {
            name: format!("Display {}", index + 1),
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
            primary: index == 0,
            device: String::new(),
        }
    }
}

/// Report display geometry in the space FFmpeg captures in on Windows.
///
/// A DPI-unaware process gets *virtualised* rectangles from `GetMonitorInfoW`: a
/// 1920×1080 panel scaled to 150% is reported as 1280×720. Capture indexes that
/// geometry — `gdigrab` takes `-video_size` and `-offset_x`/`-offset_y` from it, and
/// the resolution setting is compared against its height — so on a scaled desktop the
/// fallback path would grab a cropped corner at the wrong size. The Python app never
/// saw this because Qt declares per-monitor awareness before its first window; a Rust
/// process has to say so itself.
///
/// Windows locks awareness when a thread creates its first window, and a later call
/// fails, so call this before creating any window. Idempotent, and a no-op off Windows.
pub fn init_process() {
    #[cfg(target_os = "windows")]
    {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            use windows::Win32::UI::HiDpi::{
                SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            };
            // SAFETY: sets this process's DPI policy and passes no memory. Failure means
            // the policy was already fixed (a manifest, or an earlier call), which is
            // equally good.
            unsafe {
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
        });
    }
}

/// Displays available for capture. Never empty: falls back to a 1920×1080 primary.
pub fn list_monitors() -> Vec<Monitor> {
    // Best effort for callers that never call it themselves; a process that already
    // created a window is past this point and keeps the policy it had.
    init_process();
    let mut monitors = platform_monitors();
    if monitors.is_empty() {
        return vec![Monitor::synthetic(0)];
    }
    for (i, m) in monitors.iter_mut().enumerate() {
        if m.name.is_empty() {
            m.name = format!("Display {}", i + 1);
        }
    }
    monitors
}

/// Clamp a stored monitor index onto the current set of displays.
pub fn clamp_monitor(index: usize, monitors: &[Monitor]) -> usize {
    if index < monitors.len() {
        index
    } else {
        0
    }
}

#[cfg(target_os = "windows")]
fn platform_monitors() -> Vec<Monitor> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
    };

    extern "system" fn collect(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is a correctly sized MONITORINFOEXW for this call.
        let ok = unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) };
        if ok.as_bool() {
            let r = info.monitorInfo.rcMonitor;
            let device = String::from_utf16_lossy(
                &info.szDevice[..info.szDevice.iter().position(|c| *c == 0).unwrap_or(32)],
            );
            out.push(Monitor {
                name: String::new(),
                x: r.left,
                y: r.top,
                w: (r.right - r.left).max(0) as u32,
                h: (r.bottom - r.top).max(0) as u32,
                primary: info.monitorInfo.dwFlags & 1 != 0,
                device,
            });
        }
        TRUE
    }

    let mut monitors: Vec<Monitor> = Vec::new();
    // SAFETY: the callback only touches the Vec we hand it, for the duration of the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut monitors as *mut Vec<Monitor> as isize),
        );
    }
    // Stable order across runs, matching how ddagrab indexes outputs.
    monitors.sort_by(|a, b| a.device.cmp(&b.device));
    monitors
}

#[cfg(target_os = "macos")]
fn platform_monitors() -> Vec<Monitor> {
    use core_graphics::display::CGDisplay;

    let ids = CGDisplay::active_displays().unwrap_or_default();
    let main = CGDisplay::main().id;
    let mut monitors: Vec<Monitor> = Vec::new();
    for id in ids {
        let display = CGDisplay::new(id);
        let bounds = display.bounds();
        let scale = if display.pixels_wide() > 0 && bounds.size.width > 0.0 {
            display.pixels_wide() as f64 / bounds.size.width
        } else {
            1.0
        };
        // Even dimensions: H.264 chroma subsampling needs them.
        let w = ((bounds.size.width * scale) as u32) / 2 * 2;
        let h = ((bounds.size.height * scale) as u32) / 2 * 2;
        monitors.push(Monitor {
            name: String::new(),
            x: (bounds.origin.x * scale) as i32,
            y: (bounds.origin.y * scale) as i32,
            w: w.max(2),
            h: h.max(2),
            primary: id == main,
            device: id.to_string(),
        });
    }
    // Put the primary display first so `monitor: 0` means "the main screen" there too.
    monitors.sort_by_key(|m| !m.primary);
    monitors
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_monitors() -> Vec<Monitor> {
    Vec::new()
}

/// Whether this process may read the screen. `ask` may raise the OS prompt.
///
/// Only macOS needs permission; Windows and Linux always return `true`.
pub fn screen_capture_allowed(ask: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        use core_graphics::access::ScreenCaptureAccess;
        let access = ScreenCaptureAccess;
        if access.preflight() {
            return true;
        }
        if ask {
            return access.request();
        }
        false
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ask;
        true
    }
}

/// Open macOS's Screen Recording settings pane.
pub fn open_privacy_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn();
    }
}

/// Display indices macOS's AVFoundation backend exposes as `Capture screen N`.
///
/// FFmpeg lists capture *devices*; audio devices share the numbering, so the screen
/// entries must be found by name rather than by position.
pub fn avfoundation_screens(ffmpeg: &Path) -> Vec<u32> {
    let out = run(
        ffmpeg,
        &[
            "-hide_banner".into(),
            "-f".into(),
            "avfoundation".into(),
            "-list_devices".into(),
            "true".into(),
            "-i".into(),
            String::new(),
        ],
        Duration::from_secs(15),
        None,
    );
    parse_avfoundation_screens(&String::from_utf8_lossy(&out.stderr))
}

/// Parse `[<index>] Capture screen N` lines out of `ffmpeg -list_devices` output.
pub fn parse_avfoundation_screens(stderr: &str) -> Vec<u32> {
    let mut screens = Vec::new();
    for line in stderr.lines() {
        // Lines look like `[AVFoundation indev @ 0x1] [3] Capture screen 1`, so the
        // device index is the bracket pair immediately before the device name.
        let Some(pos) = line.find("Capture screen") else {
            continue;
        };
        let head = &line[..pos];
        let Some(close) = head.rfind(']') else {
            continue;
        };
        let Some(open) = head[..close].rfind('[') else {
            continue;
        };
        if let Ok(index) = head[open + 1..close].trim().parse::<u32>() {
            screens.push(index);
        }
    }
    screens
}

/// AVFoundation `-i` value for a display: `"<device index>:none"`.
///
/// Falls back to the first capture screen, then to device 0, exactly like the
/// Python helper did.
pub fn avf_screen_input(ffmpeg: &Path, monitor_idx: usize) -> String {
    let screens = avfoundation_screens(ffmpeg);
    let dev = if monitor_idx < screens.len() {
        screens[monitor_idx]
    } else if let Some(first) = screens.first() {
        *first
    } else {
        0
    };
    format!("{dev}:none")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_avfoundation_screen_indices() {
        let listing = "\
[AVFoundation indev @ 0x1] AVFoundation video devices:
[AVFoundation indev @ 0x1] [0] OBS Virtual Camera
[AVFoundation indev @ 0x1] [1] Capture screen 0
[AVFoundation indev @ 0x1] [3] Capture screen 1
[AVFoundation indev @ 0x1] AVFoundation audio devices:
[AVFoundation indev @ 0x1] [0] HyperX QuadCast 2
";
        assert_eq!(parse_avfoundation_screens(listing), vec![1, 3]);
    }

    #[test]
    fn no_screens_yields_empty() {
        assert!(parse_avfoundation_screens("nothing interesting").is_empty());
    }

    #[test]
    fn monitor_list_is_never_empty() {
        let monitors = list_monitors();
        assert!(!monitors.is_empty());
        assert!(monitors[0].w >= 2 && monitors[0].h >= 2);
        assert!(monitors[0].name.starts_with("Display"));
    }

    #[test]
    fn clamp_keeps_index_in_range() {
        let monitors = list_monitors();
        assert_eq!(clamp_monitor(0, &monitors), 0);
        assert_eq!(clamp_monitor(9999, &monitors), 0);
        assert_eq!(clamp_monitor(monitors.len(), &monitors), 0);
        assert_eq!(
            clamp_monitor(monitors.len() - 1, &monitors),
            monitors.len() - 1
        );
    }

    #[test]
    fn synthetic_monitor_is_primary_and_named() {
        let m = Monitor::synthetic(0);
        assert_eq!(m.name, "Display 1");
        assert!(m.primary);
        assert_eq!((m.w, m.h), (1920, 1080));
    }
}
