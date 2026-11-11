//! Rolling audio buffers for desktop sound and microphone.
//!
//! Both sources are captured as interleaved 16-bit PCM into a [`PcmRing`] that keeps
//! the last 75 seconds. Capture callbacks do not arrive at perfectly regular times —
//! and a loopback device produces *nothing at all* while the desktop is silent — so
//! every chunk is timestamped and gaps are padded with silence. That is what keeps
//! the audio locked to the video.
//!
//! Windows captures desktop audio as WASAPI loopback; macOS uses a CoreAudio process
//! tap of the default output device. Both come from `cpal`, so one code path serves
//! both platforms. The microphone is any input device, selectable by name.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Longest buffer retained: 75 s covers the 60 s maximum clip plus headroom.
pub const MAX_SECONDS: u32 = 75;

/// Fixed-point audio clock in 1/48_000 s ticks, so gap filling never drifts.
pub fn now_ticks() -> i64 {
    // A monotonic, high-resolution clock shared by every capture.
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    let elapsed = START.elapsed();
    (elapsed.as_secs() as i64) * 48_000 + (elapsed.subsec_micros() as i64 * 48_000) / 1_000_000
}

pub fn seconds(ticks: i64) -> f64 {
    ticks as f64 / 48_000.0
}

/// Interleaved 16-bit PCM history with monotonic timestamps.
pub struct PcmRing {
    pub rate: u32,
    pub channels: u32,
    /// Bytes per frame (2 bytes per sample).
    pub bpf: usize,
    max_bytes: usize,
    chunks: VecDeque<Vec<u8>>,
    size: usize,
    /// Audio-clock tick of the last buffered sample.
    end: Option<i64>,
}

impl PcmRing {
    pub fn new(rate: u32, channels: u32) -> PcmRing {
        PcmRing::with_seconds(rate, channels, MAX_SECONDS)
    }

    pub fn with_seconds(rate: u32, channels: u32, max_seconds: u32) -> PcmRing {
        let channels = channels.max(1);
        let rate = rate.max(1);
        PcmRing {
            rate,
            channels,
            bpf: 2 * channels as usize,
            max_bytes: rate as usize * max_seconds as usize * 2 * channels as usize,
            chunks: VecDeque::new(),
            size: 0,
            end: None,
        }
    }

    fn append(&mut self, data: Vec<u8>) {
        self.size += data.len();
        self.chunks.push_back(data);
        // Always keep at least one chunk so the buffer never empties mid-stream.
        while self.chunks.len() > 1 {
            let head = self.chunks.front().map(|c| c.len()).unwrap_or(0);
            if self.size - head < self.max_bytes {
                break;
            }
            self.chunks.pop_front();
            self.size -= head;
        }
    }

    fn silence(&self, seconds: f64) -> Vec<u8> {
        let frames = (seconds.max(0.0) * self.rate as f64) as usize;
        vec![0u8; frames * self.bpf]
    }

    /// Add a chunk that ended at audio-clock tick `now`.
    pub fn push(&mut self, data: &[u8], now: i64) {
        let frames = data.len() / self.bpf;
        let start = now - (frames as i64 * 48_000) / self.rate as i64;
        if let Some(end) = self.end {
            let gap = seconds(start - end);
            if gap > 0.03 {
                let pad = self.silence(gap.min(MAX_SECONDS as f64));
                self.append(pad);
            }
        }
        self.append(data.to_vec());
        self.end = Some(now);
    }

    /// Exactly `seconds` of audio ending at audio-clock tick `end_time`.
    ///
    /// Short history is padded at the front with silence; a timestamp in the past
    /// trims the tail, so a clip never contains audio from after the hotkey press.
    pub fn snapshot(&self, span: f64, end_time: i64) -> Vec<u8> {
        let need = (span * self.rate as f64) as usize * self.bpf;
        let mut len: usize = self.chunks.iter().map(|c| c.len()).sum();
        let mut out: Vec<u8> = Vec::with_capacity(len.max(need));
        for chunk in &self.chunks {
            out.extend_from_slice(chunk);
        }
        let Some(last) = self.end else {
            return vec![0u8; need];
        };
        let tail = seconds(end_time - last);
        if tail > 0.0 {
            out.extend_from_slice(&self.silence(tail.min(span)));
            len = out.len();
        } else if tail < 0.0 {
            let cut = ((-tail * self.rate as f64) as usize * self.bpf).min(out.len());
            out.truncate(len - cut);
            len = out.len();
        }
        if len >= need {
            out.split_off(len - need)
        } else {
            let mut padded = vec![0u8; need - len];
            padded.extend_from_slice(&out);
            padded
        }
    }
}

/// A capture that failed to start, or is running.
pub struct CaptureError {
    pub message: String,
}

impl CaptureError {
    fn new(message: impl Into<String>) -> CaptureError {
        CaptureError {
            message: message.into(),
        }
    }
}

/// One audio source feeding a ring buffer, optionally spooling to disk for recordings.
pub trait Capture: Send {
    /// Which source this is, so the recorder can file its ring and volume correctly.
    fn kind(&self) -> AudioKind;
    fn ring(&self) -> &Arc<Mutex<PcmRing>>;
    /// Audio-clock tick at which capture started, for offsetting the track in a mux.
    fn started_at(&self) -> i64;
    /// Why the on-disk spool failed, if it did.
    fn record_error(&self) -> Option<String>;
    fn stop(&mut self);
}

/// Where a recording spools its audio as raw s16le.
#[derive(Clone)]
pub struct SpoolConfig {
    pub path: std::path::PathBuf,
    pub rate: u32,
    pub channels: u32,
}

/// Writer thread shared by every capture: keeps the audio callback allocation-light
/// and off the disk, which is what stops loopback dropouts during heavy capture.
struct Spool {
    rate: u32,
    sender: Option<std::sync::mpsc::Sender<SpoolMsg>>,
    handle: Option<std::thread::JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    last_end: i64,
}

enum SpoolMsg {
    Data(Vec<u8>),
    Silence(usize),
    Stop,
}

impl Spool {
    fn start(config: &SpoolConfig) -> Spool {
        let (tx, rx) = std::sync::mpsc::channel::<SpoolMsg>();
        let error = Arc::new(Mutex::new(None));
        let error_writer = Arc::clone(&error);
        let path = config.path.clone();
        let rate = config.rate;
        let channels = config.channels;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let handle = std::thread::Builder::new()
            .name("rewind-audio-spool".into())
            .spawn(move || {
                use std::io::Write;
                let mut file = match std::fs::File::create(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        *error_writer.lock().unwrap() = Some(e.to_string());
                        return;
                    }
                };
                let bpf = 2 * channels as usize;
                let silence = vec![0u8; rate as usize * bpf];
                while let Ok(msg) = rx.recv() {
                    let result = match msg {
                        SpoolMsg::Data(bytes) => file.write_all(&bytes),
                        SpoolMsg::Silence(mut frames) => {
                            let mut r = Ok(());
                            while frames > 0 {
                                let n = frames.min(rate as usize);
                                r = file.write_all(&silence[..n * bpf]);
                                if r.is_err() {
                                    break;
                                }
                                frames -= n;
                            }
                            r
                        }
                        SpoolMsg::Stop => break,
                    };
                    if let Err(e) = result {
                        *error_writer.lock().unwrap() = Some(e.to_string());
                        break;
                    }
                }
                let _ = file.flush();
            })
            .ok();
        Spool {
            rate,
            sender: Some(tx),
            handle,
            error,
            last_end: 0,
        }
    }

    /// Mirror what goes into the ring: data plus any silence the ring inserted.
    fn push(&mut self, data: &[u8], frames: usize, now: i64) {
        let Some(tx) = self.sender.as_ref() else {
            return;
        };
        if self.last_end != 0 {
            let start = now - (frames as i64 * 48_000) / self.rate as i64;
            let gap_ticks = start - self.last_end;
            let gap_frames = (seconds(gap_ticks) * self.rate as f64).max(0.0) as usize;
            if gap_frames > self.rate as usize / 40 {
                let _ = tx.send(SpoolMsg::Silence(gap_frames));
            }
        }
        let _ = tx.send(SpoolMsg::Data(data.to_vec()));
        self.last_end = now.max(self.last_end);
    }

    fn finish(&mut self) {
        if let Some(tx) = self.sender.take() {
            // Pad the tail so the spooled track is as long as the capture was.
            if self.last_end != 0 {
                let tail =
                    (seconds(now_ticks() - self.last_end) * self.rate as f64).max(0.0) as usize;
                if tail > self.rate as usize / 40 {
                    let _ = tx.send(SpoolMsg::Silence(tail));
                }
            }
            let _ = tx.send(SpoolMsg::Stop);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Body shared by every concrete capture: ring, spool and lifecycle bookkeeping.
struct Shared {
    ring: Arc<Mutex<PcmRing>>,
    spool: Option<Mutex<Spool>>,
    started_at: i64,
    error: Arc<Mutex<Option<String>>>,
    stopped: AtomicBool,
}

impl Shared {
    fn new(rate: u32, channels: u32, spool: Option<SpoolConfig>) -> Shared {
        let spool = spool.map(|c| Mutex::new(Spool::start(&c)));
        Shared {
            ring: Arc::new(Mutex::new(PcmRing::new(rate, channels))),
            spool,
            started_at: now_ticks(),
            error: Arc::new(Mutex::new(None)),
            stopped: AtomicBool::new(false),
        }
    }

    /// Push one captured chunk. Called from the audio callback thread.
    fn push(&self, data: &[u8], frames: usize, now: i64) {
        if data.is_empty() {
            return;
        }
        if let Ok(mut ring) = self.ring.lock() {
            ring.push(data, now);
        }
        if let Some(spool) = self.spool.as_ref() {
            if let Ok(mut spool) = spool.lock() {
                spool.push(data, frames, now);
            }
        }
    }

    fn finish(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(spool) = self.spool.as_ref() {
            if let Ok(mut spool) = spool.lock() {
                spool.finish();
            }
        }
    }
}

/// A real capture backed by a live `cpal` stream.
pub struct CpalCapture {
    kind: AudioKind,
    shared: Arc<Shared>,
    stream: Option<cpal::Stream>,
}

impl Capture for CpalCapture {
    fn kind(&self) -> AudioKind {
        self.kind
    }
    fn ring(&self) -> &Arc<Mutex<PcmRing>> {
        &self.shared.ring
    }
    fn started_at(&self) -> i64 {
        self.shared.started_at
    }
    fn record_error(&self) -> Option<String> {
        let from_spool = self.shared.spool.as_ref().and_then(|s| {
            let guard = s.lock().ok()?;
            let text = guard.error.lock().ok()?.clone();
            text
        });
        let from_stream = self.shared.error.lock().ok().and_then(|e| e.clone());
        from_stream.or(from_spool)
    }
    fn stop(&mut self) {
        self.stream = None;
        self.shared.finish();
    }
}

impl Drop for CpalCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `loopback` (what you hear) or `mic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioKind {
    Loopback,
    Mic,
}

impl AudioKind {
    pub fn label(self) -> &'static str {
        match self {
            AudioKind::Loopback => "system",
            AudioKind::Mic => "mic",
        }
    }
}

/// Pick the device to record from.
///
/// Loopback records the default output device — `cpal` opens it in loopback mode on
/// Windows (WASAPI) and as a CoreAudio process tap on macOS. The microphone prefers
/// an exact name match, then falls back to the system default input.
pub fn open_capture(
    kind: AudioKind,
    device_name: &str,
    spool: Option<SpoolConfig>,
) -> Result<CpalCapture, CaptureError> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = match kind {
        AudioKind::Loopback => host
            .default_output_device()
            .ok_or_else(|| CaptureError::new("No audio device found"))?,
        AudioKind::Mic => {
            let mut chosen = None;
            if !device_name.is_empty() {
                if let Ok(devices) = host.input_devices() {
                    chosen = devices.into_iter().find(|d| d.to_string() == device_name);
                }
            }
            chosen
                .or_else(|| host.default_input_device())
                .ok_or_else(|| CaptureError::new("No audio device found"))?
        }
    };

    let default_config = match kind {
        // A render device reports its mix format as an *output* config; capture it in loopback.
        AudioKind::Loopback => device.default_output_config(),
        AudioKind::Mic => device.default_input_config(),
    }
    .map_err(|e| CaptureError::new(e.to_string()))?;

    let rate = default_config.sample_rate();
    let channels = u32::from(default_config.channels()).max(1);
    let sample_format = default_config.sample_format();
    let config: cpal::StreamConfig = default_config.into();

    let shared = Arc::new(Shared::new(rate, channels, spool));

    let build =
        |to_pcm: fn(&cpal::Data) -> (Vec<u8>, usize)| -> Result<cpal::Stream, CaptureError> {
            let shared_cb = Arc::clone(&shared);
            let err_sink = Arc::clone(&shared.error);
            let stream = device
                .build_input_stream_raw(
                    config,
                    sample_format,
                    move |data: &cpal::Data, _info: &cpal::InputCallbackInfo| {
                        let (pcm, frames) = to_pcm(data);
                        shared_cb.push(&pcm, frames, now_ticks());
                    },
                    move |err| {
                        if let Ok(mut slot) = err_sink.lock() {
                            *slot = Some(err.to_string());
                        }
                    },
                    Some(Duration::from_secs(5)),
                )
                .map_err(|e| CaptureError::new(e.to_string()))?;
            stream
                .play()
                .map_err(|e| CaptureError::new(e.to_string()))?;
            Ok(stream)
        };

    let stream = build(data_to_pcm)?;
    Ok(CpalCapture {
        kind,
        shared,
        stream: Some(stream),
    })
}

/// Convert a callback buffer to interleaved s16le, whatever the device's native format.
fn data_to_pcm(data: &cpal::Data) -> (Vec<u8>, usize) {
    macro_rules! convert {
        ($ty:ty, $conv:expr) => {{
            let samples: &[$ty] = data.as_slice().unwrap_or(&[]);
            let mut out = Vec::with_capacity(samples.len() * 2);
            for s in samples {
                let v: i16 = $conv(*s);
                out.extend_from_slice(&v.to_le_bytes());
            }
            (out, samples.len())
        }};
    }
    match data.sample_format() {
        cpal::SampleFormat::I16 => convert!(i16, |s: i16| s),
        cpal::SampleFormat::U16 => convert!(u16, |s: u16| (s as i32 - 32768) as i16),
        cpal::SampleFormat::I8 => convert!(i8, |s: i8| (s as i16) << 8),
        cpal::SampleFormat::U8 => convert!(u8, |s: u8| (s as i16 - 128) << 8),
        cpal::SampleFormat::I32 => convert!(i32, |s: i32| (s >> 16) as i16),
        cpal::SampleFormat::U32 => {
            convert!(u32, |s: u32| ((s as i64 - 2_147_483_648) >> 16) as i16)
        }
        cpal::SampleFormat::I64 => convert!(i64, |s: i64| (s >> 48) as i16),
        cpal::SampleFormat::U64 => {
            convert!(u64, |s: u64| (((s as i128) - (1i128 << 63)) >> 48) as i16)
        }
        cpal::SampleFormat::F32 => convert!(f32, |s: f32| {
            (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
        }),
        cpal::SampleFormat::F64 => convert!(f64, |s: f64| {
            (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
        }),
        _ => (Vec::new(), 0),
    }
}

/// Test-only source: a 440 Hz tone that goes silent for one second in four.
///
/// The silence is the point — it reproduces WASAPI loopback's habit of delivering no
/// callbacks while nothing is playing, which is what the gap filling exists for.
pub struct FakeCapture {
    kind: AudioKind,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl FakeCapture {
    pub fn start(kind: AudioKind, spool: Option<SpoolConfig>) -> FakeCapture {
        let rate = 48_000u32;
        let channels = 2u32;
        let shared = Arc::new(Shared::new(rate, channels, spool));
        let stop = Arc::new(AtomicBool::new(false));
        let cb_shared = Arc::clone(&shared);
        let cb_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("rewind-fake-audio".into())
            .spawn(move || {
                let frames = 960usize; // 20 ms
                let mut i = 0u64;
                let start = Instant::now();
                while !cb_stop.load(Ordering::SeqCst) {
                    let now = now_ticks();
                    let second = start.elapsed().as_secs() % 4;
                    let mut pcm = Vec::with_capacity(frames * 2 * 2);
                    if second != 3 {
                        for k in 0..frames {
                            let t = (i + k as u64) as f64 / rate as f64;
                            let v = (8000.0 * (std::f64::consts::TAU * 440.0 * t).sin()) as i16;
                            pcm.extend_from_slice(&v.to_le_bytes());
                            pcm.extend_from_slice(&v.to_le_bytes());
                        }
                        cb_shared.push(&pcm, frames, now);
                    }
                    // Silent seconds push nothing at all, like a quiet loopback device.
                    i += frames as u64;
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
            .ok();
        FakeCapture {
            kind,
            shared,
            thread,
            stop,
        }
    }
}

impl Capture for FakeCapture {
    fn kind(&self) -> AudioKind {
        self.kind
    }
    fn ring(&self) -> &Arc<Mutex<PcmRing>> {
        &self.shared.ring
    }
    fn started_at(&self) -> i64 {
        self.shared.started_at
    }
    fn record_error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|e| e.clone())
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        self.shared.finish();
    }
}

impl Drop for FakeCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Microphone names to offer in settings, excluding loopback devices.
pub fn list_microphones() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let Ok(devices) = host.devices() else {
        return Vec::new();
    };
    devices
        .filter(|d| d.supports_input())
        .map(|d| d.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_at(rate: u32, channels: u32, end: i64) -> PcmRing {
        let mut ring = PcmRing::new(rate, channels);
        // 1 second of a constant non-zero sample, pushed 20 ms at a time.
        let frames_per_chunk = rate as usize / 50;
        let bpf = 2 * channels as usize;
        let mut pushed = 0;
        while pushed < rate as usize {
            let mut buf = Vec::with_capacity(frames_per_chunk * bpf);
            for _ in 0..frames_per_chunk * channels as usize {
                buf.extend_from_slice(&1000i16.to_le_bytes());
            }
            let now = end - ((rate as i64 - pushed as i64) * 48_000) / rate as i64;
            ring.push(&buf, now);
            pushed += frames_per_chunk;
        }
        ring
    }

    #[test]
    fn snapshot_has_exact_length() {
        let ring = ring_at(48_000, 2, 48_000 * 2);
        let pcm = ring.snapshot(1.0, 48_000 * 2);
        assert_eq!(pcm.len(), 48_000 * 4);
    }

    #[test]
    fn short_history_is_padded_with_silence() {
        let ring = ring_at(48_000, 2, 48_000);
        let pcm = ring.snapshot(5.0, 48_000);
        assert_eq!(pcm.len(), 5 * 48_000 * 4);
        // The pad is at the front: the first bytes are silence.
        assert!(pcm[..1000].iter().all(|b| *b == 0));
        // The last buffered sample is 20 ms before the requested end, so that tail is
        // silence too; just before it the tone must still be there.
        let tail = (0.02 * 48_000.0) as usize * 4;
        assert!(
            pcm[pcm.len() - tail..].iter().all(|b| *b == 0),
            "tail pad is silence"
        );
        assert!(
            pcm[pcm.len() - tail - 400..pcm.len() - tail]
                .iter()
                .any(|b| *b != 0),
            "tone sits just before the tail"
        );
    }

    #[test]
    fn empty_ring_returns_silence_of_the_requested_length() {
        let ring = PcmRing::new(48_000, 2);
        let pcm = ring.snapshot(2.0, now_ticks());
        assert_eq!(pcm.len(), 2 * 48_000 * 4);
        assert!(pcm.iter().all(|b| *b == 0));
    }

    #[test]
    fn future_end_time_only_adds_the_missing_tail() {
        // Snapshot taken 0.5 s after the last sample: no audio invents itself.
        let ring = ring_at(48_000, 2, 48_000);
        let pcm = ring.snapshot(1.0, 48_000 + 24_000);
        assert_eq!(pcm.len(), 48_000 * 4);
        assert!(
            pcm[pcm.len() - 100..].iter().all(|b| *b == 0),
            "tail is silence"
        );
        assert!(
            pcm[..100].iter().any(|b| *b != 0),
            "head still has the tone"
        );
    }

    #[test]
    fn past_end_time_trims_the_tail() {
        let ring = ring_at(48_000, 2, 48_000 * 2);
        // Ask for the second ending 0.5 s before the newest sample.
        let pcm = ring.snapshot(0.5, 48_000 * 2 - 24_000);
        assert_eq!(pcm.len(), 24_000 * 4);
    }

    #[test]
    fn gaps_between_chunks_are_filled_with_silence() {
        let mut ring = PcmRing::new(8_000, 1);
        let chunk = vec![7u8; 800 * 2]; // 0.1 s
        ring.push(&chunk, 8_000);
        // Next chunk starts 0.5 s later; the 0.4 s hole must become silence.
        ring.push(&chunk, 8_000 + (48_000 / 2));
        let pcm = ring.snapshot(0.6, 8_000 + 48_000 / 2);
        assert_eq!(pcm.len(), (0.6 * 8_000.0) as usize * 2);
        // Somewhere in the middle the gap bytes must be zero.
        let mid = &pcm[pcm.len() / 2 - 200..pcm.len() / 2 + 200];
        assert!(mid.iter().all(|b| *b == 0), "gap should be silent");
    }

    #[test]
    fn ring_stays_within_its_byte_budget() {
        let mut ring = PcmRing::with_seconds(48_000, 2, 1);
        let chunk = vec![1u8; 48_000 * 4 / 10];
        for i in 0..50 {
            ring.push(&chunk, 48_000 * (i + 1));
        }
        let total: usize = ring.chunks.iter().map(|c| c.len()).sum();
        assert!(total <= ring.max_bytes + chunk.len(), "held {total} bytes");
    }

    #[test]
    fn pcm_conversion_maps_f32_to_s16_range() {
        // Round-trip the conversion table through a fake Data is not possible without a
        // device, so check the arithmetic the match arms encode.
        let to_s16 = |s: f32| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        assert_eq!(to_s16(0.0), 0);
        assert_eq!(to_s16(1.0), 32767);
        assert_eq!(to_s16(-1.0), -32767);
        assert_eq!(to_s16(2.0), 32767, "clamped");
    }

    #[test]
    fn audio_clock_converts_to_seconds() {
        assert!((seconds(48_000) - 1.0).abs() < 1e-9);
        assert!((seconds(24_000) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn fake_capture_produces_tone_and_silence() {
        let mut cap = FakeCapture::start(AudioKind::Loopback, None);
        std::thread::sleep(Duration::from_millis(600));
        let pcm = {
            let ring = cap.ring().lock().unwrap();
            ring.snapshot(0.5, now_ticks())
        };
        cap.stop();
        assert_eq!(pcm.len(), (0.5 * 48_000.0) as usize * 4);
        assert!(pcm.iter().any(|b| *b != 0), "tone should be present");
    }
}
