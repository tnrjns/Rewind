//! Screen capture for instant replay and full-length recordings.
//!
//! FFmpeg captures the desktop (DXGI Desktop Duplication on Windows, AVFoundation on
//! macOS) straight into the GPU encoder and muxes it into a ring of one-second
//! MPEG-TS segments. Saving a clip stitches the newest N segments together with a
//! stream copy — no re-encode — then muxes in the last N seconds of desktop and mic
//! audio held in RAM.
//!
//! Full recordings write the same video stream to a temporary Matroska file and
//! spool audio to disk, muxing it into the final MP4 on stop.
//!
//! Threading rules that keep the UI responsive:
//! * all state lives in one mutex-guarded [`Core`];
//! * only the worker thread mutates it, driven by a command channel;
//! * the worker never holds the lock across an FFmpeg call, so readers never block
//!   on a 12-second capture start;
//! * clip muxing runs on its own thread with a snapshot of what it needs.

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::audio::{
    self, now_ticks, seconds, AudioKind, Capture, FakeCapture, PcmRing, SpoolConfig,
};
use crate::ffmpeg::{
    self, detect_encoders, download_ffmpeg_mac, encoder_args, find_ffmpeg, probe_duration,
    resolve_encoder, EncoderKind,
};
use crate::meta::{make_thumbnail, update_meta};
use crate::paths::{data_dir, unique_path};
use crate::platform::{
    avf_screen_input, clamp_monitor, list_monitors, screen_capture_allowed, Monitor,
};
use crate::settings::{Codec, Resolution, Settings};
use crate::wav::write_wav;

/// Segments kept on disk: more than the 60 s maximum clip plus headroom.
pub const SEGMENT_WRAP: u32 = 75;

pub const PERMISSION_NOTE: &str = "Rewind needs permission to record your screen.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Off,
    Starting,
    On,
    Saving,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Starting => "starting",
            State::On => "on",
            State::Saving => "saving",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Replay,
    Recording,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Replay => "replay",
            Mode::Recording => "recording",
        }
    }
}

/// Everything the app layer reacts to, in the order it happened.
#[derive(Debug, Clone)]
pub enum Event {
    StateChanged(State),
    Error(String),
    ClipStarted,
    ClipSaved { path: PathBuf, duration: f64 },
    ClipFailed(String),
    RecordingSaved { path: PathBuf, duration: f64 },
    RecordingFailed(String),
    ThumbnailReady(PathBuf),
    EncodersDetected(Vec<EncoderKind>),
}

#[derive(Debug, Clone)]
enum Cmd {
    Start,
    Stop { emit: bool },
    Restart,
    RecordStart,
    RecordStop,
    Revive,
    RecordCrash,
}

/// One source handed back by the audio-opening thread:
/// ring key, capture (or `None`), a failure note, and the spool path when recording.
type OpenedAudio = (
    &'static str,
    Option<Box<dyn Capture>>,
    Option<String>,
    Option<PathBuf>,
);

type AudioJob = std::thread::JoinHandle<Vec<OpenedAudio>>;

/// Audio spooled to disk for a full recording.
struct RecordAudio {
    kind: AudioKind,
    path: PathBuf,
    capture: Box<dyn Capture>,
}

/// One audio track to mux: where it starts relative to the video, and its gain.
#[derive(Debug, Clone)]
struct AudioTrack {
    path: PathBuf,
    volume: f64,
    /// Seconds after the video's first frame at which this track's audio begins.
    offset: f64,
}

/// Mutable engine state, guarded by one mutex.
struct Core {
    settings: Settings,
    state: State,
    mode: Mode,
    proc: Option<Child>,
    pipeline: String,
    active_encoder: String,
    started_at: Option<Instant>,
    encoders: Option<Vec<EncoderKind>>,
    monitors: Vec<Monitor>,
    ffmpeg: Option<PathBuf>,
    audio: HashMap<&'static str, Box<dyn Capture>>,
    audio_note: String,
    record_audio: Vec<RecordAudio>,
    record_video: Option<PathBuf>,
    record_started_ticks: i64,
    record_codec: Codec,
    record_volumes: HashMap<&'static str, f64>,
    /// Start ticks of capture attempts in the last minute, for the revive budget.
    revives: Vec<Instant>,
    /// Set while a clip is being muxed; a recording start must wait for it.
    saving: Arc<AtomicBool>,
}

/// Paths and flags that never change for the life of a recorder.
struct Env {
    buf_dir: PathBuf,
    work_dir: PathBuf,
    log_path: PathBuf,
    pid_path: PathBuf,
    test_source: bool,
}

pub struct Recorder {
    core: Arc<Mutex<Core>>,
    env: Env,
    commands: Sender<Cmd>,
    events_rx: Receiver<Event>,
    events_tx: Sender<Event>,
    saving: Arc<AtomicBool>,
}

impl Recorder {
    /// Create a recorder: clears a stale FFmpeg left by a force-quit, then starts the
    /// worker and watchdog threads.
    pub fn new(settings: Settings) -> Recorder {
        Recorder::with_options(settings, std::env::var_os("REWIND_TEST_SOURCE").is_some())
    }

    /// [`Recorder::new`] with the synthetic capture source forced on or off.
    pub fn with_options(settings: Settings, test_source: bool) -> Recorder {
        Recorder::with_data_dir(settings, test_source, &data_dir())
    }

    /// Build a recorder whose buffers, spools and log live under `data_root`.
    ///
    /// The app keeps one recorder per user profile, but tests and side-by-side
    /// instances need their own folders: two recorders sharing a buffer directory
    /// clear each other's segments and fight over the capture.
    pub fn with_data_dir(settings: Settings, test_source: bool, data_root: &Path) -> Recorder {
        let buf_dir = data_root.join("buffer");
        let work_dir = data_root.join("work");
        let _ = fs::create_dir_all(&buf_dir);
        let _ = fs::create_dir_all(&work_dir);
        let log_path = data_root.join("ffmpeg.log");
        let pid_path = data_root.join("ffmpeg.pid");
        let monitors = list_monitors();
        kill_stale_ffmpeg(&pid_path, &buf_dir, &work_dir);
        let saving_flag = Arc::new(AtomicBool::new(false));

        let core = Core {
            settings,
            state: State::Off,
            mode: Mode::Off,
            proc: None,
            pipeline: String::new(),
            active_encoder: String::new(),
            started_at: None,
            encoders: None,
            monitors,
            ffmpeg: find_ffmpeg(),
            audio: HashMap::new(),
            audio_note: String::new(),
            record_audio: Vec::new(),
            record_video: None,
            record_started_ticks: 0,
            record_codec: Codec::H264,
            record_volumes: HashMap::new(),
            revives: Vec::new(),
            saving: Arc::clone(&saving_flag),
        };

        let (commands_tx, commands_rx) = mpsc::channel::<Cmd>();
        let (events_tx, events_rx) = mpsc::channel::<Event>();

        let recorder = Recorder {
            core: Arc::new(Mutex::new(core)),
            env: Env {
                buf_dir,
                work_dir,
                log_path,
                pid_path,
                test_source,
            },
            commands: commands_tx,
            events_rx,
            events_tx,
            saving: Arc::clone(&saving_flag),
        };
        recorder.spawn_worker(commands_rx);
        recorder.spawn_watchdog();
        recorder
    }

    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn emit(&self, event: Event) {
        let _ = self.events_tx.send(event);
    }

    // ---------------------------------------------------------------- observation

    pub fn state(&self) -> State {
        self.lock().state
    }

    pub fn mode(&self) -> Mode {
        self.lock().mode
    }

    /// `Apple VideoToolbox · H264 · 1920×1080 @ 60 fps`, for the status pill and tray.
    pub fn pipeline(&self) -> String {
        self.lock().pipeline.clone()
    }

    pub fn active_encoder(&self) -> String {
        self.lock().active_encoder.clone()
    }

    /// Warning naming any audio source that could not be opened.
    pub fn audio_note(&self) -> String {
        self.lock().audio_note.clone()
    }

    pub fn monitors(&self) -> Vec<Monitor> {
        self.lock().monitors.clone()
    }

    pub fn ffmpeg_path(&self) -> Option<PathBuf> {
        self.lock().ffmpeg.clone()
    }

    pub fn encoders(&self) -> Option<Vec<EncoderKind>> {
        self.lock().encoders.clone()
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    /// Apply changed settings. The caller decides whether to [`Recorder::restart`].
    pub fn update_settings(&self, settings: Settings) {
        self.lock().settings = settings;
    }

    /// Seconds currently held in the replay buffer.
    pub fn buffered_seconds(&self) -> f64 {
        let core = self.lock();
        match (core.state, core.started_at) {
            (State::On, Some(start)) => start.elapsed().as_secs_f64(),
            _ => 0.0,
        }
    }

    /// Blocking event read. `None` once every sender is gone.
    pub fn recv(&self) -> Option<Event> {
        self.events_rx.recv().ok()
    }

    /// Non-blocking event read for UI loops.
    pub fn try_recv(&self) -> Option<Event> {
        self.events_rx.try_recv().ok()
    }

    // ---------------------------------------------------------------- commands

    pub fn start(&self) {
        let _ = self.commands.send(Cmd::Start);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Cmd::Stop { emit: true });
    }

    pub fn restart(&self) {
        let _ = self.commands.send(Cmd::Restart);
    }

    pub fn start_recording(&self) {
        let _ = self.commands.send(Cmd::RecordStart);
    }

    pub fn stop_recording(&self) {
        let _ = self.commands.send(Cmd::RecordStop);
    }

    /// Stop capture and finish any recording. Waits up to six seconds for the file
    /// to be muxed, so a quit never loses a recording.
    pub fn shutdown(&self) {
        let _ = self.commands.send(Cmd::Stop { emit: false });
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline && self.state() != State::Off {
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Probe which encoders work here; cached after the first call.
    pub fn detect_encoders(&self) -> Vec<EncoderKind> {
        if let Some(cached) = self.encoders() {
            return cached;
        }
        let ffmpeg = self.ffmpeg_path();
        let found = if self.env.test_source {
            vec![EncoderKind::X264]
        } else {
            detect_encoders(ffmpeg.as_deref())
        };
        self.lock().encoders = Some(found.clone());
        self.emit(Event::EncodersDetected(found.clone()));
        found
    }

    /// Encoder this capture will use, honouring the setting only when available.
    pub fn resolve_encoder(&self) -> EncoderKind {
        let available = self.encoders().unwrap_or_else(|| vec![EncoderKind::X264]);
        resolve_encoder(&self.settings().encoder, &available)
    }

    // ---------------------------------------------------------------- clipping

    /// Save the last `seconds` of the replay buffer as a clip.
    ///
    /// `false` when it cannot start (replay off, or a save already running); the
    /// outcome arrives as [`Event::ClipSaved`] or [`Event::ClipFailed`].
    pub fn save_clip(&self, seconds: Option<u32>) -> bool {
        {
            let core = self.lock();
            if core.state != State::On || core.mode != Mode::Replay {
                drop(core);
                self.emit(Event::ClipFailed("Instant Replay is off".into()));
                return false;
            }
        }
        if self.saving.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.emit(Event::ClipStarted);

        let ctx = {
            let core = self.lock();
            ClipJob {
                ffmpeg: core.ffmpeg.clone(),
                buf_dir: self.env.buf_dir.clone(),
                work_dir: self.env.work_dir.clone(),
                log_path: self.env.log_path.clone(),
                output_dir: core.settings.output_dir.clone(),
                codec: core.settings.codec,
                audio_offset: Duration::from_millis(core.settings.audio_offset_ms.max(0) as u64),
                sources: core
                    .audio
                    .iter()
                    .map(|(name, cap)| AudioSource {
                        name,
                        ring: Arc::clone(cap.ring()),
                        volume: volume_for(name, &core.settings),
                    })
                    .collect(),
                seconds: seconds.unwrap_or(core.settings.clip_seconds),
                events: self.events_tx.clone(),
                saving: Arc::clone(&self.saving),
            }
        };
        std::thread::Builder::new()
            .name("rewind-clip".into())
            .spawn(move || ctx.run())
            .ok();
        true
    }

    // ---------------------------------------------------------------- threads

    fn spawn_worker(&self, rx: Receiver<Cmd>) {
        let core = Arc::clone(&self.core);
        let env = Env {
            buf_dir: self.env.buf_dir.clone(),
            work_dir: self.env.work_dir.clone(),
            log_path: self.env.log_path.clone(),
            pid_path: self.env.pid_path.clone(),
            test_source: self.env.test_source,
        };
        let events = self.events_tx.clone();
        std::thread::Builder::new()
            .name("rewind-worker".into())
            .spawn(move || {
                let mut worker = Worker { core, env, events };
                while let Ok(cmd) = rx.recv() {
                    worker.handle(cmd);
                }
            })
            .ok();
    }

    fn spawn_watchdog(&self) {
        let core = Arc::clone(&self.core);
        let commands = self.commands.clone();
        let events = self.events_tx.clone();
        std::thread::Builder::new()
            .name("rewind-watchdog".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let mut core = core.lock().unwrap_or_else(|p| p.into_inner());
                if core.state != State::On {
                    continue;
                }
                let Some(proc) = core.proc.as_mut() else {
                    continue;
                };
                let Ok(Some(status)) = proc.try_wait() else {
                    continue;
                };
                let code = status.code().unwrap_or(-1);
                let recording = core.mode == Mode::Recording;
                log_line(
                    &core_log_path(&core),
                    &format!("capture died (code {code})"),
                );
                core.proc = None;
                let now = Instant::now();
                core.revives
                    .retain(|t| now.duration_since(*t) < Duration::from_secs(60));
                let budget_left = core.revives.len() < 3;
                if budget_left {
                    core.revives.push(now);
                }
                drop(core);
                if recording {
                    let _ = commands.send(Cmd::RecordCrash);
                } else if budget_left {
                    let _ = commands.send(Cmd::Revive);
                } else {
                    let _ = commands.send(Cmd::Stop { emit: false });
                    let _ = events.send(Event::Error(
                        "Screen capture keeps stopping. Check ffmpeg.log or try another encoder."
                            .into(),
                    ));
                }
            })
            .ok();
    }
}

/// `ffmpeg.log` lives beside settings; the watchdog reads it through the core.
fn core_log_path(_core: &Core) -> PathBuf {
    data_dir().join("ffmpeg.log")
}

fn volume_for(name: &str, settings: &Settings) -> f64 {
    let raw = if name == "system" {
        settings.system_volume
    } else {
        settings.mic_volume
    };
    raw.clamp(0, 100) as f64 / 100.0
}

/// Append one line to `ffmpeg.log`, with a wall-clock time.
fn log_line(path: &Path, message: &str) {
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let now = crate::paths::local_now();
    let _ = writeln!(
        file,
        "[{:02}:{:02}:{:02}] {message}",
        now.hour, now.minute, now.second
    );
}

/// Kill an FFmpeg orphaned by a Rewind that was force-quit; it would still hold the screen.
fn kill_stale_ffmpeg(pid_path: &Path, buf_dir: &Path, work_dir: &Path) {
    let Ok(text) = fs::read_to_string(pid_path) else {
        return;
    };
    let _ = fs::remove_file(pid_path);
    let Ok(pid) = text.trim().parse::<u32>() else {
        return;
    };
    #[cfg(unix)]
    {
        // Only kill it if the command line mentions our buffer, so a recycled PID is safe.
        let out = crate::ffmpeg::run(
            "/bin/ps",
            &["-p".into(), pid.to_string(), "-o".into(), "command=".into()],
            Duration::from_secs(5),
            None,
        );
        let cmdline = String::from_utf8_lossy(&out.stdout);
        let ours = cmdline.contains(&buf_dir.to_string_lossy().to_string())
            || cmdline.contains(&work_dir.to_string_lossy().to_string());
        if ours {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, buf_dir, work_dir);
    }
}

/// Record the capture PID so the next launch can clean up after a crash.
fn write_pid(path: &Path, pid: u32) {
    let _ = fs::write(path, pid.to_string());
}

// ---------------------------------------------------------------- the worker

struct Worker {
    core: Arc<Mutex<Core>>,
    env: Env,
    events: Sender<Event>,
}

impl Worker {
    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn log(&self, message: &str) {
        log_line(&self.env.log_path, message);
    }

    fn set_state(&self, state: State) {
        let mut core = self.lock();
        if core.state != state {
            core.state = state;
            drop(core);
            self.emit(Event::StateChanged(state));
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Start => self.start_capture(Mode::Replay),
            Cmd::Stop { emit } => self.stop_capture(emit),
            Cmd::Restart => {
                if self.lock().mode == Mode::Recording {
                    return;
                }
                self.stop_capture(false);
                self.start_capture(Mode::Replay);
            }
            Cmd::RecordStart => {
                // A clip save must finish before the buffer is reused.
                while self.lock().saving.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                let (state, mode) = {
                    let core = self.lock();
                    (core.state, core.mode)
                };
                if state == State::On && mode == Mode::Recording {
                    return;
                }
                if state != State::Off {
                    self.stop_capture(false);
                }
                self.start_capture(Mode::Recording);
            }
            Cmd::RecordStop => {
                if self.lock().mode == Mode::Recording {
                    self.stop_capture(true);
                }
            }
            Cmd::Revive => {
                self.stop_capture(false);
                self.start_capture(Mode::Replay);
            }
            Cmd::RecordCrash => {
                if self.lock().mode == Mode::Recording {
                    self.emit(Event::Error(
                        "Screen capture stopped unexpectedly. Saving the recording so far.".into(),
                    ));
                    self.stop_capture(true);
                }
            }
        }
    }

    // ------------------------------------------------------------ start / stop

    fn start_capture(&mut self, mode: Mode) {
        {
            let mut core = self.lock();
            if core.proc.is_some() {
                if core.mode == Mode::Recording && mode == Mode::Replay {
                    return;
                }
                if core.mode != mode {
                    return;
                }
                drop(core);
                self.set_state(State::On);
                return;
            }
            core.mode = mode;
        }
        self.set_state(State::Starting);

        let ffmpeg = {
            let mut core = self.lock();
            if core.ffmpeg.is_none() {
                core.ffmpeg = find_ffmpeg();
            }
            let missing = core.ffmpeg.is_none();
            if missing && cfg!(target_os = "macos") && !self.env.test_source {
                drop(core);
                self.log("downloading FFmpeg");
                let downloaded = download_ffmpeg_mac(|line| self.log(line));
                let mut core = self.lock();
                core.ffmpeg = downloaded;
                core.ffmpeg.clone()
            } else {
                core.ffmpeg.clone()
            }
        };

        let Some(ffmpeg) = ffmpeg else {
            self.fail_start(mode, missing_ffmpeg_message());
            return;
        };

        if cfg!(target_os = "macos") && !self.env.test_source && !screen_capture_allowed(true) {
            self.fail_start(
                mode,
                format!(
                    "{PERMISSION_NOTE} Turn on Rewind in System Settings → Privacy & Security → \
                     Screen & System Audio Recording, then quit and reopen Rewind."
                ),
            );
            return;
        }

        let encoders = if self.lock().encoders.is_some() {
            self.lock().encoders.clone().unwrap_or_default()
        } else {
            let found = if self.env.test_source {
                vec![EncoderKind::X264]
            } else {
                detect_encoders(Some(&ffmpeg))
            };
            self.lock().encoders = Some(found.clone());
            self.emit(Event::EncodersDetected(found.clone()));
            found
        };

        // Snapshot everything the launch needs, then release the lock for the slow part.
        let (settings, monitors, encoder) = {
            let mut core = self.lock();
            core.monitors = list_monitors();
            let encoder = resolve_encoder(&core.settings.encoder, &encoders);
            core.record_codec = core.settings.codec;
            (core.settings.clone(), core.monitors.clone(), encoder)
        };

        if mode == Mode::Recording {
            clear_stale_spools(&self.env.work_dir);
        }
        let record_video = if mode == Mode::Recording {
            // Capture starts writing here, before the launch wait below, so a recording
            // begins roughly a second before the keypress — enough to catch the moment
            // without the user having to anticipate it. Audio is offset by the same gap
            // at mux time, so picture and sound still line up.
            let token = token();
            let mut core = self.lock();
            core.record_started_ticks = now_ticks();
            core.record_volumes = HashMap::from([
                ("system", volume_for("system", &settings)),
                ("mic", volume_for("mic", &settings)),
            ]);
            core.record_audio.clear();
            let path = self.env.work_dir.join(format!("recording_{token}.mkv"));
            core.record_video = Some(path.clone());
            Some(path)
        } else {
            clear_buffer(&self.env.buf_dir);
            None
        };

        let attempts = build_attempts(
            &settings,
            &monitors,
            encoder,
            &ffmpeg,
            self.env.test_source,
            &self.env.buf_dir,
            record_video.as_deref(),
        );

        // Audio devices are slow to open — a CoreAudio process tap takes over a second —
        // and capture keeps running to disk while they do. Open both at once so the
        // time to `On` is the slower of the two, not their sum.
        let audio_job = self.spawn_audio(mode == Mode::Recording);

        let mut launched = None;
        for attempt in attempts {
            match self.launch(&attempt, record_video.as_deref(), &settings) {
                Ok(proc) => {
                    launched = Some((proc, attempt));
                    break;
                }
                Err(reason) => {
                    self.log(&format!("{}: {reason}", attempt.name));
                    match &record_video {
                        Some(path) => {
                            let _ = fs::remove_file(path);
                        }
                        None => clear_buffer(&self.env.buf_dir),
                    }
                }
            }
        }

        let Some((proc, attempt)) = launched else {
            {
                let mut core = self.lock();
                core.mode = Mode::Off;
                core.record_video = None;
            }
            self.set_state(State::Off);
            let msg = "Couldn't start screen capture. Try another encoder on the Replay page \
                       (details are in ffmpeg.log).";
            if mode == Mode::Recording {
                self.emit(Event::RecordingFailed(msg.into()));
            }
            self.emit(Event::Error(msg.into()));
            return;
        };

        write_pid(&self.env.pid_path, proc.id());
        {
            let mut core = self.lock();
            core.proc = Some(proc);
            core.active_encoder = attempt.encoder.key().to_string();
            core.pipeline = format!(
                "{} · {} · {} @ {} fps",
                attempt.encoder.label(),
                settings.codec.as_str().to_uppercase(),
                attempt.size,
                settings.fps
            );
        }

        let opened = audio_job
            .and_then(|job| job.join().ok())
            .unwrap_or_default();
        {
            let mut core = self.lock();
            core.audio = HashMap::new();
            core.record_audio.clear();
            let mut notes = Vec::new();
            for (key, capture, note, spool) in opened {
                match capture {
                    Some(capture) => {
                        // Recording keeps the capture alive in `record_audio`, which owns
                        // the spool; replay only needs its ring.
                        match (mode == Mode::Recording, spool) {
                            (true, Some(path)) => core.record_audio.push(RecordAudio {
                                kind: if key == "system" {
                                    AudioKind::Loopback
                                } else {
                                    AudioKind::Mic
                                },
                                path,
                                capture,
                            }),
                            _ => {
                                core.audio.insert(key, capture);
                            }
                        }
                    }
                    None => {
                        if let Some(note) = note {
                            notes.push(note);
                        }
                    }
                }
            }
            core.audio_note = notes.join("; ");
            if !core.audio_note.is_empty() {
                let note = core.audio_note.clone();
                drop(core);
                self.log(&format!("audio: {note}"));
                core = self.lock();
            }
            core.started_at = Some(Instant::now());
        }
        self.set_state(State::On);
    }

    /// Handles the "cannot start at all" path shared by the early failures.
    fn fail_start(&self, mode: Mode, message: String) {
        {
            let mut core = self.lock();
            core.mode = Mode::Off;
        }
        self.set_state(State::Off);
        if mode == Mode::Recording {
            self.emit(Event::RecordingFailed(message.clone()));
        }
        self.emit(Event::Error(message));
    }

    /// Open the configured audio sources on a background thread.
    ///
    /// Returns handles as `(ring key, capture, note)`; a `None` capture carries the reason
    /// it could not be opened. Opening a device can take over a second, and it must not
    /// delay the moment capture is considered running.
    #[allow(clippy::type_complexity)]
    fn spawn_audio(&mut self, recording: bool) -> Option<AudioJob> {
        self.stop_audio();
        let settings = self.lock().settings.clone();
        let work_dir = self.env.work_dir.clone();
        let test_source = self.env.test_source;
        let token = token();
        // Spooled tracks are installed by the audio thread below and read back by the
        // muxer, so the spool paths are computed up front and matched by name.
        let spooled: Vec<(AudioKind, PathBuf, SpoolConfig)> = {
            let mut out = Vec::new();
            if settings.system_audio {
                let cfg = SpoolConfig {
                    path: work_dir.join(format!("recording_system_{token}.pcm")),
                    rate: 48_000,
                    channels: 2,
                };
                out.push((AudioKind::Loopback, cfg.path.clone(), cfg));
            }
            if settings.mic_enabled {
                let cfg = SpoolConfig {
                    path: work_dir.join(format!("recording_mic_{token}.pcm")),
                    rate: 48_000,
                    channels: 2,
                };
                out.push((AudioKind::Mic, cfg.path.clone(), cfg));
            }
            out
        };
        let wanted: Vec<(AudioKind, String, &'static str)> = {
            let mut out = Vec::new();
            if settings.system_audio {
                out.push((AudioKind::Loopback, String::new(), "system"));
            }
            if settings.mic_enabled {
                out.push((AudioKind::Mic, settings.mic_device.clone(), "mic"));
            }
            out
        };

        let handle = std::thread::Builder::new()
            .name("rewind-audio-open".into())
            .spawn(move || {
                let mut out: Vec<OpenedAudio> = Vec::new();
                for (kind, device, key) in wanted {
                    let cfg = if recording {
                        spooled
                            .iter()
                            .find(|(k, _, _)| *k == kind)
                            .map(|(_, _, cfg)| cfg.clone())
                    } else {
                        None
                    };
                    let spool_path = cfg.as_ref().map(|c| c.path.clone());
                    let capture: Result<Box<dyn Capture>, String> = if test_source {
                        Ok(Box::new(FakeCapture::start(kind, cfg)))
                    } else {
                        audio::open_capture(kind, &device, cfg)
                            .map(|c| Box::new(c) as Box<dyn Capture>)
                            .map_err(|e| e.message)
                    };
                    match capture {
                        Ok(capture) => out.push((key, Some(capture), None, spool_path)),
                        Err(message) => {
                            let label = match kind {
                                AudioKind::Loopback => "desktop audio",
                                AudioKind::Mic => "microphone",
                            };
                            out.push((
                                key,
                                None,
                                Some(format!("{label} unavailable ({message})")),
                                None,
                            ));
                        }
                    }
                }
                out
            })
            .ok();
        handle
    }

    fn stop_audio(&mut self) {
        let mut core = self.lock();
        for capture in core.audio.values_mut() {
            capture.stop();
        }
        core.audio.clear();
        for track in core.record_audio.iter_mut() {
            track.capture.stop();
        }
    }

    fn stop_capture(&mut self, emit: bool) {
        let previous_mode = self.lock().mode;
        let proc = { self.lock().proc.take() };
        if let Some(mut proc) = proc {
            // FFmpeg's segmenter and muxers finalise on a clean `q`; kill only if it hangs.
            if let Some(stdin) = proc.stdin.as_mut() {
                let _ = stdin.write_all(b"q");
                let _ = stdin.flush();
            }
            if wait_with_timeout(&mut proc, Duration::from_secs(4)).is_none() {
                let _ = proc.kill();
                let _ = proc.wait();
            }
        }
        let _ = fs::remove_file(&self.env.pid_path);
        self.stop_audio();

        {
            let mut core = self.lock();
            core.mode = Mode::Off;
        }

        if previous_mode == Mode::Recording && self.lock().record_video.is_some() {
            self.set_state(if emit { State::Saving } else { State::Off });
            self.finish_recording(emit);
        } else if emit {
            self.set_state(State::Off);
        } else {
            self.lock().state = State::Off;
        }
    }

    // ------------------------------------------------------------ launching

    fn launch(
        &self,
        attempt: &Attempt,
        record_video: Option<&Path>,
        settings: &Settings,
    ) -> Result<Child, String> {
        let ffmpeg = self
            .lock()
            .ffmpeg
            .clone()
            .ok_or_else(|| "ffmpeg missing".to_string())?;
        let mut args: Vec<String> = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-nostats".into(),
            "-y".into(),
        ];
        args.extend(attempt.input.iter().cloned());
        args.extend(attempt.filters.iter().cloned());
        args.extend(["-map".into(), "0:v".into(), "-an".into()]);
        args.extend(encoder_args(
            attempt.encoder,
            settings.codec,
            settings.bitrate_mbps.max(2),
            settings.fps.max(1),
        ));
        let expected_output: PathBuf = match record_video {
            Some(path) => {
                args.extend([
                    "-f".into(),
                    "matroska".into(),
                    path.to_string_lossy().into_owned(),
                ]);
                path.to_path_buf()
            }
            None => {
                let pattern = self.env.buf_dir.join("seg%03d.ts");
                args.extend([
                    "-f".into(),
                    "segment".into(),
                    "-segment_time".into(),
                    "1".into(),
                    "-segment_wrap".into(),
                    SEGMENT_WRAP.to_string(),
                    "-segment_format".into(),
                    "mpegts".into(),
                    pattern.to_string_lossy().into_owned(),
                ]);
                self.env.buf_dir.clone()
            }
        };

        self.log(&format!(
            "try {}: {ffmpeg:?} {}",
            attempt.name,
            args.join(" ")
        ));

        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.env.log_path)
            .map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&ffmpeg);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log.try_clone().map_err(|e| e.to_string())?));
        configure_child(&mut cmd);
        let mut proc = cmd.spawn().map_err(|e| e.to_string())?;

        // Wait for real output: a capture that can open the screen still fails later.
        let timeout = if cfg!(target_os = "macos") {
            Duration::from_secs(12)
        } else {
            Duration::from_secs(7)
        };
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = proc.try_wait() {
                return Err(format!("exited with {}", status.code().unwrap_or(-1)));
            }
            let produced = if record_video.is_some() {
                fs::metadata(&expected_output)
                    .map(|m| m.len() > 2048)
                    .unwrap_or(false)
            } else {
                buffer_has_data(&expected_output)
            };
            if produced {
                let _ = writeln!(log, "started {}", attempt.name);
                return Ok(proc);
            }
            if Instant::now() >= deadline {
                let _ = proc.kill();
                let _ = proc.wait();
                return Err("produced no output".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // ------------------------------------------------------------ recording finish

    fn finish_recording(&mut self, emit: bool) {
        let (video, codec, started_ticks, volumes) = {
            let core = self.lock();
            (
                core.record_video.clone(),
                core.record_codec,
                core.record_started_ticks,
                core.record_volumes.clone(),
            )
        };
        let tracks: Vec<AudioTrack> = {
            let core = self.lock();
            core.record_audio
                .iter()
                .filter_map(|track| {
                    if let Some(err) = track.capture.record_error() {
                        self.emit(Event::Error(format!(
                            "{} audio could not be saved: {err}",
                            title_case(track.kind.label())
                        )));
                        return None;
                    }
                    let has_bytes = fs::metadata(&track.path)
                        .map(|m| m.len() > 0)
                        .unwrap_or(false);
                    if !has_bytes {
                        return None;
                    }
                    let offset = seconds(track.capture.started_at() - started_ticks).max(0.0);

                    let volume = volumes
                        .get(track.kind.label())
                        .copied()
                        .unwrap_or_else(|| volume_for(track.kind.label(), &core.settings));
                    Some(AudioTrack {
                        path: track.path.clone(),
                        volume,
                        offset,
                    })
                })
                .collect()
        };

        let Some(video) = video else {
            self.reset_recording();
            self.set_state(State::Off);
            self.emit(Event::RecordingFailed("No video was captured".into()));
            return;
        };
        let ffmpeg = self.lock().ffmpeg.clone();
        let video_ok = fs::metadata(&video).map(|m| m.len() > 0).unwrap_or(false);
        let Some(ffmpeg) = ffmpeg.filter(|_| video_ok) else {
            cleanup_recording(&video, &tracks);
            self.reset_recording();
            self.set_state(State::Off);
            self.emit(Event::RecordingFailed("No video was captured".into()));
            return;
        };

        let out = unique_path(
            &self.lock().settings.output_dir.clone(),
            "Rewind_Recording_",
            "mp4",
        );
        if let Some(parent) = out.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // The video file is the authority on length; the wall clock is only a fallback,
        // because opening a capture device can take seconds before the first frame.
        let duration =
            probe_duration(&ffmpeg, &video).unwrap_or_else(|| seconds(now_ticks() - started_ticks));
        let timeout = Duration::from_secs((duration * 2.0).max(180.0) as u64);

        let mut result = mux_recording(&ffmpeg, &video, &out, &tracks, codec, duration, timeout);
        if !result.ok() && !tracks.is_empty() {
            self.log(&format!(
                "recording audio mux failed: {}",
                result.stderr_tail(800)
            ));
            result = mux_recording(&ffmpeg, &video, &out, &[], codec, duration, timeout);
            if result.ok() {
                self.emit(Event::Error(
                    "Recording saved without audio because audio muxing failed.".into(),
                ));
            }
        }

        let wrote = fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false);
        if !result.ok() || !wrote {
            self.log(&format!(
                "recording mux failed: {}",
                result.stderr_tail(800)
            ));
            let _ = fs::remove_file(&out);
            cleanup_recording(&video, &tracks);
            self.reset_recording();
            self.set_state(State::Off);
            self.emit(Event::RecordingFailed("Couldn't save the recording".into()));
            return;
        }

        cleanup_recording(&video, &tracks);
        self.reset_recording();
        let _ = update_meta(
            &crate::paths::file_key(&out),
            &[(
                "duration",
                serde_json::json!((duration * 100.0).round() / 100.0),
            )],
        );
        self.set_state(State::Off);
        if emit {
            self.emit(Event::RecordingSaved {
                path: out.clone(),
                duration,
            });
        }
        if make_thumbnail(&ffmpeg, &out, Some(duration)).is_some() {
            self.emit(Event::ThumbnailReady(out));
        }
    }

    fn reset_recording(&mut self) {
        let mut core = self.lock();
        core.record_audio.clear();
        core.record_video = None;
    }
}

fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn cleanup_recording(video: &Path, tracks: &[AudioTrack]) {
    let _ = fs::remove_file(video);
    for track in tracks {
        let _ = fs::remove_file(&track.path);
    }
}

fn missing_ffmpeg_message() -> String {
    if cfg!(target_os = "macos") {
        "Couldn't download FFmpeg, which Rewind records with. Check your internet connection and \
         turn Instant Replay on again."
            .into()
    } else {
        "FFmpeg wasn't found. Run install.bat, or put ffmpeg.exe in Rewind's ffmpeg\\bin folder."
            .into()
    }
}

fn token() -> String {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{n:x}{nanos:x}")
}

/// Wait for a child to exit, killing it if it takes longer than `timeout`.
fn wait_with_timeout(proc: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match proc.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Remove half-written audio and video spools left by a crash or force-quit.
fn clear_stale_spools(work_dir: &Path) {
    let Ok(entries) = fs::read_dir(work_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = path
            .file_name()
            .map(|n| {
                let n = n.to_string_lossy();
                n.starts_with("recording_") && (n.ends_with(".pcm") || n.ends_with(".mkv"))
            })
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(path);
        }
    }
}

fn clear_buffer(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_segment = path
            .file_name()
            .map(|n| {
                let n = n.to_string_lossy();
                n.starts_with("seg") && n.ends_with(".ts")
            })
            .unwrap_or(false);
        if is_segment {
            let _ = fs::remove_file(path);
        }
    }
}

fn buffer_has_data(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        let is_segment = path
            .file_name()
            .map(|n| {
                let n = n.to_string_lossy();
                n.starts_with("seg") && n.ends_with(".ts")
            })
            .unwrap_or(false);
        is_segment && fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false)
    })
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

/// One capture command line: input, filters, encoder and expected size.
struct Attempt {
    name: String,
    input: Vec<String>,
    filters: Vec<String>,
    encoder: EncoderKind,
    size: String,
}

/// The fallback ladder: hardware encoder first, then the same source on CPU, then a
/// different capture backend, so one broken path never kills Instant Replay.
fn build_attempts(
    settings: &Settings,
    monitors: &[Monitor],
    encoder: EncoderKind,
    ffmpeg: &Path,
    test_source: bool,
    _buf_dir: &Path,
    _record_video: Option<&Path>,
) -> Vec<Attempt> {
    let fps = settings.fps.max(1);
    let cursor = if settings.capture_cursor { "1" } else { "0" };

    if test_source {
        return vec![Attempt {
            name: "test".into(),
            input: vec![
                "-re".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!("testsrc2=size=1280x720:rate={fps}"),
            ],
            filters: vec!["-vf".into(), "format=yuv420p".into()],
            encoder: EncoderKind::X264,
            size: "1280×720".into(),
        }];
    }

    let idx = clamp_monitor(settings.monitor, monitors);
    let monitor = &monitors[idx];
    let target_h = match settings.resolution {
        Resolution::Native => None,
        Resolution::Height(h) if h >= monitor.h => None,
        Resolution::Height(h) => Some(h),
    };
    let scale = target_h
        .map(|h| format!(",scale=-2:{h}:flags=bicubic"))
        .unwrap_or_default();
    let out_w = match target_h {
        None => monitor.w,
        Some(h) => {
            let ratio = h as f64 / monitor.h as f64;
            ((monitor.w as f64 * ratio) as f32 / 2.0).round() as u32 * 2
        }
    };
    let out_h = target_h.unwrap_or(monitor.h);
    let size = format!("{out_w}×{out_h}");
    let pix = if encoder == EncoderKind::X264 {
        "yuv420p"
    } else {
        "nv12"
    };
    let filter_tail = {
        let s = scale.trim_start_matches(',');
        if s.is_empty() {
            "null".to_string()
        } else {
            s.to_string()
        }
    };

    if cfg!(target_os = "macos") {
        let input = vec![
            "-f".into(),
            "avfoundation".into(),
            "-capture_cursor".into(),
            cursor.into(),
            "-framerate".into(),
            fps.to_string(),
            "-pixel_format".into(),
            "nv12".into(),
            "-i".into(),
            avf_screen_input(ffmpeg, idx),
        ];
        let cfr = vec![
            "-fps_mode".into(),
            "cfr".into(),
            "-r".into(),
            fps.to_string(),
        ];
        let mut attempts = vec![Attempt {
            name: "avfoundation".into(),
            input: input.clone(),
            filters: {
                let mut f = cfr.clone();
                f.extend(["-vf".into(), format!("{filter_tail},format={pix}")]);
                f
            },
            encoder,
            size: size.clone(),
        }];
        if encoder != EncoderKind::X264 {
            attempts.push(Attempt {
                name: "avfoundation-cpu".into(),
                input,
                filters: {
                    let mut f = cfr;
                    f.extend(["-vf".into(), format!("{filter_tail},format=yuv420p")]);
                    f
                },
                encoder: EncoderKind::X264,
                size,
            });
        }
        return attempts;
    }

    let dda = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("ddagrab=output_idx={idx}:framerate={fps}:draw_mouse={cursor}"),
    ];
    let mut attempts = Vec::new();
    if matches!(encoder, EncoderKind::Nvenc | EncoderKind::Amf) && target_h.is_none() {
        attempts.push(Attempt {
            name: "ddagrab-gpu".into(),
            input: dda.clone(),
            filters: Vec::new(),
            encoder,
            size: size.clone(),
        });
    }
    attempts.push(Attempt {
        name: "ddagrab".into(),
        input: dda,
        filters: vec![
            "-vf".into(),
            format!("hwdownload,format=bgra{scale},format={pix}"),
        ],
        encoder,
        size: size.clone(),
    });
    let gdi = vec![
        "-f".into(),
        "gdigrab".into(),
        "-framerate".into(),
        fps.to_string(),
        "-draw_mouse".into(),
        cursor.into(),
        "-offset_x".into(),
        monitor.x.to_string(),
        "-offset_y".into(),
        monitor.y.to_string(),
        "-video_size".into(),
        format!("{}x{}", monitor.w, monitor.h),
        "-i".into(),
        "desktop".into(),
    ];
    attempts.push(Attempt {
        name: "gdigrab".into(),
        input: gdi.clone(),
        filters: vec!["-vf".into(), format!("{filter_tail},format={pix}")],
        encoder,
        size: size.clone(),
    });
    if encoder != EncoderKind::X264 {
        attempts.push(Attempt {
            name: "gdigrab-cpu".into(),
            input: gdi,
            filters: vec!["-vf".into(), format!("{filter_tail},format=yuv420p")],
            encoder: EncoderKind::X264,
            size,
        });
    }
    attempts
}

// ---------------------------------------------------------------- clip job

/// A clip's audio snapshot: (source name, PCM, rate, channels, bytes per frame, gain).
type AudioSnapshot = (&'static str, Vec<u8>, u32, u32, usize, f64);

/// One audio track snapshotted for a clip.
struct AudioSource {
    name: &'static str,
    ring: Arc<Mutex<PcmRing>>,
    volume: f64,
}

/// Everything a clip needs, captured so the save thread never touches engine state.
struct ClipJob {
    ffmpeg: Option<PathBuf>,
    buf_dir: PathBuf,
    work_dir: PathBuf,
    log_path: PathBuf,
    output_dir: PathBuf,
    codec: Codec,
    audio_offset: Duration,
    sources: Vec<AudioSource>,
    seconds: u32,
    events: Sender<Event>,
    saving: Arc<AtomicBool>,
}

impl ClipJob {
    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn log(&self, message: &str) {
        log_line(&self.log_path, message);
    }

    fn run(self) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.clip()));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(message)) => self.emit(Event::ClipFailed(message)),
            Err(_) => self.emit(Event::ClipFailed("Couldn't write the clip file".into())),
        }
        self.saving.store(false, Ordering::SeqCst);
    }

    fn clip(&self) -> Result<(), String> {
        let Some(ffmpeg) = self.ffmpeg.clone() else {
            return Err("FFmpeg wasn't found".into());
        };
        // Let the encoder flush the last few frames so the clip ends where it should.
        std::thread::sleep(Duration::from_millis(350));

        let end_ticks = now_ticks() - (self.audio_offset.as_millis() as i64 * 48) / 1000;
        let snaps: Vec<AudioSnapshot> = self
            .sources
            .iter()
            .filter_map(|source| {
                let ring = source.ring.lock().ok()?;
                let pcm = ring.snapshot(f64::from(self.seconds) + 4.0, end_ticks);
                Some((
                    source.name,
                    pcm,
                    ring.rate,
                    ring.channels,
                    ring.bpf,
                    source.volume,
                ))
            })
            .collect();

        let mut segments = list_segments(&self.buf_dir);
        if segments.is_empty() {
            return Err("Nothing recorded yet".into());
        }
        segments.sort_by_key(|entry| entry.1);
        let keep = (self.seconds as usize) + 1;
        let chosen: Vec<PathBuf> = segments
            .into_iter()
            .rev()
            .take(keep)
            .map(|(path, _)| path)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let concat_input = format!(
            "concat:{}",
            chosen
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("|")
        );
        let tmp_ts = self.work_dir.join("clip.ts");
        let joined = ffmpeg::run(
            &ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
                "-i".into(),
                concat_input,
                "-map".into(),
                "0:v".into(),
                "-c".into(),
                "copy".into(),
                "-f".into(),
                "mpegts".into(),
                tmp_ts.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(90),
            None,
        );
        if !joined.ok() || !file_has_bytes(&tmp_ts) {
            self.log(&format!("concat failed: {}", joined.stderr_tail(800)));
            return Err("Couldn't read the replay buffer".into());
        }

        let duration = probe_duration(&ffmpeg, &tmp_ts).unwrap_or(chosen.len() as f64);

        let mut wavs: Vec<(PathBuf, f64)> = Vec::new();
        for (name, pcm, rate, channels, bpf, volume) in &snaps {
            let need = (duration * f64::from(*rate)) as usize * bpf;
            let trimmed = if pcm.len() >= need {
                pcm[pcm.len() - need..].to_vec()
            } else {
                let mut padded = vec![0u8; need - pcm.len()];
                padded.extend_from_slice(pcm);
                padded
            };
            let wav = self.work_dir.join(format!("{name}.wav"));
            write_wav(&wav, &trimmed, *rate, *channels).map_err(|e| e.to_string())?;
            wavs.push((wav, *volume));
        }

        let _ = fs::create_dir_all(&self.output_dir);
        let out = unique_path(&self.output_dir, "Rewind_", "mp4");
        let build = |with_audio: bool| -> Vec<String> {
            let mut args = vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
                "-i".into(),
                tmp_ts.to_string_lossy().into_owned(),
            ];
            if with_audio && !wavs.is_empty() {
                for (wav, _) in &wavs {
                    args.extend(["-i".into(), wav.to_string_lossy().into_owned()]);
                }
                if wavs.len() == 1 {
                    args.extend([
                        "-map".into(),
                        "0:v".into(),
                        "-map".into(),
                        "1:a".into(),
                        "-filter:a".into(),
                        format!("volume={:.2}", wavs[0].1),
                    ]);
                } else {
                    let mut fc = String::new();
                    for (i, (_, volume)) in wavs.iter().enumerate() {
                        fc.push_str(&format!(
                            "[{}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume={volume:.2}[a{i}];",
                            i + 1
                        ));
                    }
                    for i in 0..wavs.len() {
                        fc.push_str(&format!("[a{i}]"));
                    }
                    fc.push_str(&format!(
                        "amix=inputs={}:duration=longest:normalize=0[aout]",
                        wavs.len()
                    ));
                    args.extend([
                        "-filter_complex".into(),
                        fc,
                        "-map".into(),
                        "0:v".into(),
                        "-map".into(),
                        "[aout]".into(),
                    ]);
                }
                args.extend([
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    "192k".into(),
                    "-ac".into(),
                    "2".into(),
                ]);
            } else {
                args.extend(["-map".into(), "0:v".into()]);
            }
            args.extend(["-c:v".into(), "copy".into()]);
            if let Some(tag) = self.codec.mp4_tag() {
                args.extend(["-tag:v".into(), tag.into()]);
            }
            args.extend(["-movflags".into(), "+faststart".into()]);
            args.push(out.to_string_lossy().into_owned());
            args
        };

        let mut result = ffmpeg::run(&ffmpeg, &build(true), Duration::from_secs(180), None);
        if !result.ok() && !wavs.is_empty() {
            self.log(&format!(
                "mux with audio failed: {}",
                result.stderr_tail(800)
            ));
            result = ffmpeg::run(&ffmpeg, &build(false), Duration::from_secs(180), None);
        }
        let _ = fs::remove_file(&tmp_ts);
        for (wav, _) in &wavs {
            let _ = fs::remove_file(wav);
        }
        if !result.ok() || !file_has_bytes(&out) {
            self.log(&format!("mux failed: {}", result.stderr_tail(800)));
            let _ = fs::remove_file(&out);
            return Err("Couldn't write the clip file".into());
        }

        let _ = update_meta(
            &crate::paths::file_key(&out),
            &[(
                "duration",
                serde_json::json!((duration * 100.0).round() / 100.0),
            )],
        );
        self.emit(Event::ClipSaved {
            path: out.clone(),
            duration,
        });
        if make_thumbnail(&ffmpeg, &out, Some(duration)).is_some() {
            self.emit(Event::ThumbnailReady(out));
        }
        Ok(())
    }
}

fn file_has_bytes(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false)
}

/// `(path, mtime)` for every non-empty segment in the buffer.
fn list_segments(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !(name.starts_with("seg") && name.ends_with(".ts")) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if meta.len() == 0 {
                return None;
            }
            let mtime = meta.modified().ok()?;
            Some((path, mtime))
        })
        .collect()
}

/// Mux captured video plus spooled audio tracks into the final MP4.
fn mux_recording(
    ffmpeg: &Path,
    video: &Path,
    output: &Path,
    tracks: &[AudioTrack],
    codec: Codec,
    duration: f64,
    timeout: Duration,
) -> ffmpeg::Output {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-i".into(),
        video.to_string_lossy().into_owned(),
    ];
    for track in tracks {
        // Audio capture starts a moment after FFmpeg opened the screen, and opening a
        // macOS display takes seconds. Shifting the track by that gap is what keeps
        // sound on its frame; without it the tail gets cut to match the shorter audio.
        if track.offset > 0.001 {
            args.extend(["-itsoffset".into(), format!("{:.3}", track.offset)]);
        }
        args.extend([
            "-f".into(),
            "s16le".into(),
            "-ar".into(),
            "48000".into(),
            "-ac".into(),
            "2".into(),
            "-i".into(),
            track.path.to_string_lossy().into_owned(),
        ]);
    }
    match tracks.len() {
        0 => args.extend(["-map".into(), "0:v".into(), "-an".into()]),
        1 => {
            let volume = tracks[0].volume;
            args.extend([
                "-map".into(),
                "0:v".into(),
                "-map".into(),
                "1:a".into(),
                "-filter:a".into(),
                format!("volume={volume:.2},atrim=0:{duration:.3}"),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-ac".into(),
                "2".into(),
            ]);
        }
        _ => {
            let mut fc = String::new();
            for (i, track) in tracks.iter().enumerate() {
                fc.push_str(&format!(
                    "[{}:a]aformat=sample_rates=48000:channel_layouts=stereo,volume={:.2},\
                     atrim=0:{duration:.3}[a{i}];",
                    i + 1,
                    track.volume
                ));
            }
            for i in 0..tracks.len() {
                fc.push_str(&format!("[a{i}]"));
            }
            fc.push_str(&format!(
                "amix=inputs={}:duration=longest:normalize=0[aout]",
                tracks.len()
            ));
            args.extend([
                "-filter_complex".into(),
                fc,
                "-map".into(),
                "0:v".into(),
                "-map".into(),
                "[aout]".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-ac".into(),
                "2".into(),
            ]);
        }
    }
    args.extend(["-c:v".into(), "copy".into()]);
    if let Some(tag) = codec.mp4_tag() {
        args.extend(["-tag:v".into(), tag.into()]);
    }
    args.extend(["-movflags".into(), "+faststart".into()]);
    args.push(output.to_string_lossy().into_owned());
    ffmpeg::run(ffmpeg, &args, timeout, None)
}
