//! Feedback sounds: the clip chime, and the rest.
//!
//! Every sound is synthesised — there are no audio assets — so this is a direct port of
//! `app/sounds.py`. The WAVs are cached by style and volume under `sounds/`, because the
//! synthesised buffer is rebuilt only when the volume changes.

use std::path::{Path, PathBuf};

use crate::paths::sub_dir;
use crate::wav::write_wav;

pub const RATE: u32 = 44_100;

/// The styles the Sound page offers, with the labels it shows.
pub const STYLES: [(&str, &str); 4] = [
    ("chime", "Chime"),
    ("pop", "Pop"),
    ("blip", "Double blip"),
    ("custom", "Custom .wav file…"),
];

/// Frequencies and envelopes are the Python app's, so the chime sounds the same.
///
/// The parameter list mirrors `_tone` in `app/sounds.py`; keeping it identical is what
/// makes the two builds sound alike.
#[allow(clippy::too_many_arguments)]
fn tone(
    out: &mut [f64],
    start: f64,
    freq: f64,
    amp: f64,
    decay: f64,
    duration: Option<f64>,
    harmonics: &[f64],
    attack: f64,
) {
    let start = (start * RATE as f64) as usize;
    if start >= out.len() {
        return;
    }
    let n = match duration {
        Some(d) => (d * RATE as f64) as usize,
        None => out.len() - start,
    }
    .min(out.len() - start);
    let w = 2.0 * std::f64::consts::PI * freq / RATE as f64;
    for i in 0..n {
        let t = i as f64 / RATE as f64;
        let env = (t / attack).min(1.0) * (-t * decay).exp();
        let mut value = 0.0;
        for (k, h) in harmonics.iter().enumerate() {
            value += h * (w * (k + 1) as f64 * i as f64).sin();
        }
        out[start + i] += amp * env * value;
    }
}

/// A deterministic noise source, so the shutter is the same twice and tests can rely on it.
fn noise(seed: u64) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        // xorshift64*, mapped to -1.0..1.0.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let value = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// The samples for one style, in -1.0..1.0. Unknown styles fall back to the chime.
pub fn synth(style: &str) -> Vec<f64> {
    match style {
        "pop" => {
            let n = (RATE as f64 * 0.16) as usize;
            let mut out = vec![0.0; n];
            let mut phase = 0.0;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f64 / RATE as f64;
                let freq = 260.0 + 900.0 * (-t * 28.0).exp();
                phase += 2.0 * std::f64::consts::PI * freq / RATE as f64;
                let env = (t / 0.002).min(1.0) * (-t * 30.0).exp();
                *sample = env * phase.sin();
            }
            out
        }
        "shutter" => {
            let mut out = vec![0.0; (RATE as f64 * 0.2) as usize];
            let mut rnd = noise(7);
            for (start, amp) in [(0.0, 1.0), (0.065, 0.75)] {
                let start = (start * RATE as f64) as usize;
                let mut prev = 0.0;
                for i in 0..(RATE as f64 * 0.05) as usize {
                    let t = i as f64 / RATE as f64;
                    // A soft low-pass on the noise, plus a body thump.
                    prev = 0.55 * prev + 0.45 * rnd();
                    let thump =
                        0.6 * (2.0 * std::f64::consts::PI * 190.0 * t).sin() * (-t * 60.0).exp();
                    if start + i < out.len() {
                        out[start + i] += amp * (prev * (-t * 110.0).exp() + thump);
                    }
                }
            }
            out
        }
        "blip" => {
            let mut out = vec![0.0; (RATE as f64 * 0.22) as usize];
            for start in [0.0, 0.085] {
                tone(
                    &mut out,
                    start,
                    1318.5,
                    0.8,
                    38.0,
                    Some(0.07),
                    &[1.0, 0.0, 0.25],
                    0.003,
                );
            }
            out
        }
        "toggle_on" => {
            let mut out = vec![0.0; (RATE as f64 * 0.35) as usize];
            tone(
                &mut out,
                0.0,
                783.99,
                0.6,
                16.0,
                None,
                &[1.0, 0.18, 0.06],
                0.004,
            );
            tone(
                &mut out,
                0.07,
                1174.66,
                0.7,
                14.0,
                None,
                &[1.0, 0.18, 0.06],
                0.004,
            );
            out
        }
        "toggle_off" => {
            let mut out = vec![0.0; (RATE as f64 * 0.35) as usize];
            tone(
                &mut out,
                0.0,
                1174.66,
                0.6,
                16.0,
                None,
                &[1.0, 0.18, 0.06],
                0.004,
            );
            tone(
                &mut out,
                0.07,
                783.99,
                0.7,
                14.0,
                None,
                &[1.0, 0.18, 0.06],
                0.004,
            );
            out
        }
        _ => {
            let mut out = vec![0.0; (RATE as f64 * 0.8) as usize];
            tone(
                &mut out,
                0.0,
                1046.5,
                0.8,
                7.5,
                None,
                &[1.0, 0.18, 0.06],
                0.004,
            );
            tone(
                &mut out,
                0.09,
                1567.98,
                0.9,
                6.5,
                None,
                &[1.0, 0.18, 0.06],
                0.004,
            );
            out
        }
    }
}

/// Samples to 16-bit PCM, peaked at 0.9 and scaled by `volume` (0-100), as the Python
/// writer did so a quiet chime and a loud one sound like the same chime.
fn to_pcm(samples: &[f64], volume: i32) -> Vec<u8> {
    let peak = samples.iter().fold(1e-6_f64, |worst, s| worst.max(s.abs()));
    let gain = 0.9 / peak * volume.clamp(0, 100) as f64 / 100.0;
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let scaled = (sample * gain).clamp(-1.0, 1.0);
        let value = (scaled * 32767.0) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    pcm
}

/// Path of the cached WAV for a style and volume, writing it if needed.
pub fn sound_file(style: &str, volume: i32, custom: &str) -> Option<PathBuf> {
    let style = if style == "custom" {
        if !custom.is_empty() && Path::new(custom).exists() {
            return scaled_custom(Path::new(custom), volume);
        }
        "chime"
    } else {
        style
    };
    let volume = volume.clamp(0, 100);
    let path = sub_dir("sounds").join(format!("{style}_{volume}.wav"));
    if !path.exists() {
        let pcm = to_pcm(&synth(style), volume);
        write_wav(&path, &pcm, RATE, 1).ok()?;
    }
    Some(path)
}

/// A custom WAV, rescaled to the chosen volume. Falls back to the file untouched when
/// it is not 16-bit PCM, which is what the Python helper did on any parse error.
fn scaled_custom(path: &Path, volume: i32) -> Option<PathBuf> {
    let out = sub_dir("sounds").join(format!(
        "custom_{}_{}.wav",
        crate::paths::file_key(path),
        volume.clamp(0, 100)
    ));
    if out.exists() {
        return Some(out);
    }
    let bytes = std::fs::read(path).ok()?;
    let Some((format, mut pcm)) = split_wav(&bytes) else {
        return Some(path.to_path_buf());
    };
    let gain = volume.clamp(0, 100) as f64 / 100.0;
    for chunk in pcm.chunks_exact_mut(2) {
        let value = i16::from_le_bytes([chunk[0], chunk[1]]);
        let scaled = (f64::from(value) * gain).clamp(-32768.0, 32767.0) as i16;
        chunk.copy_from_slice(&scaled.to_le_bytes());
    }
    write_wav(&out, &pcm, format.0, format.1).ok()?;
    Some(out)
}

/// Minimal WAV split: returns (rate, channels) and the 16-bit samples.
fn split_wav(bytes: &[u8]) -> Option<((u32, u32), Vec<u8>)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12;
    let mut format = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let len = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let body = bytes.get(offset + 8..offset + 8 + len)?;
        if id == b"fmt " {
            if body.len() < 16 {
                return None;
            }
            let channels = u16::from_le_bytes(body[2..4].try_into().ok()?) as u32;
            let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            let bits = u16::from_le_bytes(body[14..16].try_into().ok()?);
            if bits != 16 {
                return None;
            }
            format = Some((rate, channels));
        } else if id == b"data" {
            return Some((format?, body.to_vec()));
        }
        offset += 8 + len + (len & 1); // chunks are word-aligned
    }
    None
}

/// Play a feedback sound. Windows uses `PlaySound`; macOS shells out to `afplay`, which
/// the Python app never did — its sounds were silent there, because `play_file` only had
/// a Windows branch.
pub fn play(style: &str, volume: i32, custom: &str) {
    let Some(path) = sound_file(style, volume, custom) else {
        return;
    };
    play_file(&path);
}

pub fn play_file(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        use windows::core::HSTRING;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
        let wide = HSTRING::from(path.to_string_lossy().as_ref());
        // SAFETY: the string is NUL-terminated for the duration of the call, and
        // SND_ASYNC means the system keeps its own copy of what it needs.
        unsafe {
            let _ = PlaySoundW(
                windows::core::PCWSTR(wide.as_ptr()),
                None,
                SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        // `afplay` blocks until the sound ends, so it runs as a child we never wait on.
        let _ = std::process::Command::new("afplay")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_produces_sound_of_the_expected_length() {
        for (style, expected_seconds) in [
            ("chime", 0.8),
            ("pop", 0.16),
            ("blip", 0.22),
            ("shutter", 0.2),
            ("toggle_on", 0.35),
            ("toggle_off", 0.35),
        ] {
            let samples = synth(style);
            let seconds = samples.len() as f64 / RATE as f64;
            assert!(
                (seconds - expected_seconds).abs() < 0.01,
                "{style}: {seconds}s"
            );
            let peak = samples.iter().fold(0.0_f64, |worst, s| worst.max(s.abs()));
            assert!(peak > 0.05, "{style} is silent (peak {peak})");
        }
    }

    #[test]
    fn an_unknown_style_is_the_chime() {
        assert_eq!(synth("nonsense").len(), synth("chime").len());
        assert_eq!(synth("").len(), synth("chime").len());
    }

    #[test]
    fn toggle_on_and_off_are_different() {
        assert_ne!(synth("toggle_on"), synth("toggle_off"));
    }

    #[test]
    fn pcm_is_peaked_and_follows_the_volume() {
        let samples = synth("chime");
        let loud = to_pcm(&samples, 100);
        let quiet = to_pcm(&samples, 50);
        let peak = loud
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
            .max()
            .unwrap();
        // 0.9 of full scale, within rounding.
        assert!((29300..=29500).contains(&peak), "peak {peak}");
        let quiet_peak = quiet
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
            .max()
            .unwrap();
        assert!(quiet_peak < peak / 2 + 400, "{quiet_peak} vs {peak}");
    }

    #[test]
    fn a_zero_volume_is_silence_not_a_missing_file() {
        let pcm = to_pcm(&synth("chime"), 0);
        assert!(pcm.chunks_exact(2).all(|c| c == [0, 0]));
    }

    #[test]
    fn a_wav_round_trips_through_the_splitter() {
        let pcm = to_pcm(&synth("blip"), 100);
        let path = std::env::temp_dir().join(format!("rewind-sound-{}.wav", std::process::id()));
        write_wav(&path, &pcm, RATE, 1).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let ((rate, channels), data) = split_wav(&bytes).unwrap();
        assert_eq!((rate, channels), (RATE, 1));
        assert_eq!(data, pcm);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn something_that_is_not_a_wav_is_refused() {
        assert!(split_wav(b"not a wave file at all, nope").is_none());
        assert!(split_wav(&[]).is_none());
    }
}
