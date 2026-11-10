"""Screen capture for instant replay and full-length recordings.

FFmpeg captures the desktop (DXGI Desktop Duplication -> GPU encoder) into a
ring of 1-second MPEG-TS segments. Saving a clip stitches the newest N
segments together (stream copy, no re-encode), then muxes in the last N
seconds of desktop/mic audio held in RAM. Full recordings capture the same
video stream to a temporary Matroska file and spool audio until it is muxed to
the final MP4 on stop.
"""
import datetime
import glob
import json
import os
import queue
import re
import shutil
import subprocess
import threading
import time
import uuid
import wave

from PySide6.QtCore import QObject, Signal

from . import winapi
from .audio import make_capture
from .settings import app_base_dir, data_dir, sub_dir, IS_MAC, IS_WIN

TEST_MODE = bool(os.environ.get("REWIND_TEST_SOURCE"))
SEGMENT_WRAP = 75  # > 60 s max clip + headroom

ENCODERS = {
    "nvenc": {"h264": "h264_nvenc", "hevc": "hevc_nvenc", "label": "NVIDIA NVENC"},
    "amf": {"h264": "h264_amf", "hevc": "hevc_amf", "label": "AMD AMF"},
    "qsv": {"h264": "h264_qsv", "hevc": "hevc_qsv", "label": "Intel Quick Sync"},
    "vt": {"h264": "h264_videotoolbox", "hevc": "hevc_videotoolbox", "label": "Apple VideoToolbox"},
    "x264": {"h264": "libx264", "hevc": "libx265", "label": "CPU (x264)"},
}

_meta_lock = threading.Lock()


# ---------------------------------------------------------------- helpers
def find_ffmpeg():
    exe = "ffmpeg.exe" if IS_WIN else "ffmpeg"
    base = app_base_dir()
    for c in (os.path.join(base, "ffmpeg", "bin", exe), os.path.join(base, "ffmpeg", exe), os.path.join(base, exe)):
        if os.path.isfile(c):
            return c
    w = shutil.which("ffmpeg")
    if w:
        return w
    if IS_MAC:  # apps opened from Finder don't get Homebrew's PATH
        for c in ("/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"):
            if os.path.isfile(c):
                return c
    if IS_WIN:
        local = os.environ.get("LOCALAPPDATA", "")
        hits = glob.glob(os.path.join(local, "Microsoft", "WinGet", "Packages", "*FFmpeg*", "*", "bin", "ffmpeg.exe"))
        if hits:
            return hits[0]
    return None


def run(cmd, timeout=60, cwd=None):
    try:
        return subprocess.run(cmd, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=timeout, **winapi.popen_kwargs())
    except subprocess.TimeoutExpired as e:
        return subprocess.CompletedProcess(cmd, 124, b"", (e.stderr or b"") + b"\ntimeout")
    except OSError as e:
        return subprocess.CompletedProcess(cmd, 127, b"", str(e).encode())


def probe_duration(ffmpeg, path):
    r = run([ffmpeg, "-hide_banner", "-i", path], timeout=20)
    m = re.search(rb"Duration:\s*(\d+):(\d+):(\d+(?:\.\d+)?)", r.stderr or b"")
    if not m:
        return None
    return int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))


def meta_path():
    return os.path.join(data_dir(), "clips.json")


def load_meta() -> dict:
    with _meta_lock:
        try:
            with open(meta_path(), "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return {}


def update_meta(name, **values):
    with _meta_lock:
        try:
            with open(meta_path(), "r", encoding="utf-8") as f:
                meta = json.load(f)
        except Exception:
            meta = {}
        meta.setdefault(name, {}).update(values)
        tmp = meta_path() + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(meta, f)
        os.replace(tmp, meta_path())


def thumb_path(video_path):
    return os.path.join(sub_dir("thumbs"), os.path.splitext(os.path.basename(video_path))[0] + ".jpg")


def make_thumbnail(ffmpeg, video, duration=None):
    out = thumb_path(video)
    for ss in (min(1.0, (duration or 2) / 2), 0):
        r = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-ss", f"{ss:.2f}", "-i", video,
                 "-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "3", out], timeout=30)
        if r.returncode == 0 and os.path.exists(out):
            return out
    return None


_avf_screens: dict = {}


def avf_screen_input(ffmpeg, monitor_idx):
    """AVFoundation input name for a display ("<device index>:none"), cached per FFmpeg binary."""
    if ffmpeg not in _avf_screens:
        r = run([ffmpeg, "-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""], timeout=15)
        _avf_screens[ffmpeg] = [int(m) for m in re.findall(rb"\[(\d+)\] Capture screen \d+", r.stderr or b"")]
    screens = _avf_screens[ffmpeg]
    dev = screens[monitor_idx] if 0 <= monitor_idx < len(screens) else (screens[0] if screens else 0)
    return f"{dev}:none"


def take_screenshot(ffmpeg, monitor_idx, out_path, cursor=True):
    """Grab one frame of a monitor with DXGI desktop duplication (works over games)."""
    jpg = out_path.lower().endswith((".jpg", ".jpeg"))
    fmt = ["-q:v", "2"] if jpg else ["-compression_level", "3"]
    pix = "yuvj444p" if jpg else "rgb24"
    if TEST_MODE:
        src = ["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30"]
        vf = f"format={pix}"
    elif IS_MAC:
        src = ["-f", "avfoundation", "-capture_cursor", "1" if cursor else "0", "-framerate", "30",
               "-pixel_format", "bgr0", "-i", avf_screen_input(ffmpeg, monitor_idx)]
        vf = f"format={pix}"
    else:
        src = ["-f", "lavfi", "-i", f"ddagrab=output_idx={monitor_idx}:framerate=30:draw_mouse={1 if cursor else 0}"]
        vf = f"hwdownload,format=bgra,format={pix}"
    r = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *src, "-frames:v", "1", "-vf", vf, *fmt,
             out_path], timeout=15)
    return r.returncode == 0 and os.path.exists(out_path) and os.path.getsize(out_path) > 0


def encoder_args(key, codec, mbps, fps):
    name = ENCODERS[key][codec]
    rate = f"{mbps}M"
    common = ["-b:v", rate, "-maxrate", rate, "-bufsize", f"{mbps * 2}M", "-g", str(fps),
              "-force_key_frames", "expr:gte(t,n_forced*1)"]
    if key == "nvenc":
        return ["-c:v", name, "-preset", "p4", "-rc", "cbr", "-forced-idr", "1", "-bf", "0"] + common
    if key == "amf":
        return ["-c:v", name, "-usage", "transcoding", "-quality", "balanced", "-rc", "cbr"] + common
    if key == "qsv":
        return ["-c:v", name, "-preset", "veryfast"] + common
    if key == "vt":
        return ["-c:v", name, "-realtime", "1", "-bf", "0"] + common
    if codec == "hevc":
        return ["-c:v", name, "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-x265-params", "log-level=error"] + common
    return ["-c:v", name, "-preset", "veryfast", "-pix_fmt", "yuv420p"] + common


# ---------------------------------------------------------------- recorder
class Recorder(QObject):
    state_changed = Signal(str)          # off | starting | on | saving
    error = Signal(str)
    clip_started = Signal()
    clip_saved = Signal(str, float)
    clip_failed = Signal(str)
    recording_saved = Signal(str, float)
    recording_failed = Signal(str)
    thumbnail_ready = Signal(str)
    encoders_detected = Signal(list)

    def __init__(self, settings):
        super().__init__()
        self.s = settings
        self.state = "off"
        self.mode = "off"                 # off | replay | recording
        self.proc = None
        self.pipeline = ""
        self.active_encoder = ""
        self.started_at = 0.0
        self.encoders = None
        self.monitors = winapi.list_monitors()
        self.ffmpeg = find_ffmpeg()
        self.buf_dir = sub_dir("buffer")
        self.work_dir = sub_dir("work")
        self.log_path = os.path.join(data_dir(), "ffmpeg.log")
        self.audio_note = ""
        self._audio = {}
        self._record_audio = []
        self._record_video = ""
        self._record_output = ""
        self._record_started_at = 0.0
        self._record_codec = settings.codec
        self._record_volumes = {}
        self._saving = False
        self._enc_lock = threading.Lock()
        self._q = queue.Queue()
        self._revives = []
        self._kill_stale_ffmpeg()
        threading.Thread(target=self._worker, daemon=True).start()
        threading.Thread(target=self._watchdog, daemon=True).start()

    # ---- public API (thread-safe, non-blocking)
    def start(self):
        self._q.put("start")

    def stop(self):
        self._q.put("stop")

    def restart(self):
        self._q.put("restart")

    def start_recording(self):
        self._q.put("record_start")

    def stop_recording(self):
        self._q.put("record_stop")

    def shutdown(self):
        self._do_stop(emit=False)

    def buffered_seconds(self) -> float:
        if self.state != "on":
            return 0.0
        return max(0.0, time.monotonic() - self.started_at)

    def detect_encoders(self):
        with self._enc_lock:
            if self.encoders is not None:
                return self.encoders
            found = []
            if self.ffmpeg and not TEST_MODE:
                for key in (("vt",) if IS_MAC else ("nvenc", "amf", "qsv")):
                    r = run([self.ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "lavfi",
                             "-i", "color=black:s=640x360:r=30", "-frames:v", "3",
                             "-c:v", ENCODERS[key]["h264"], "-f", "null", "-"], timeout=20)
                    if r.returncode == 0:
                        found.append(key)
            found.append("x264")
            self.encoders = found
        self.encoders_detected.emit(found)
        return found

    def resolve_encoder(self, encs=None):
        encs = encs or self.encoders or ["x264"]
        if self.s.encoder != "auto" and self.s.encoder in encs:
            return self.s.encoder
        return encs[0]

    # ---- worker
    def _worker(self):
        while True:
            cmd = self._q.get()
            try:
                if cmd == "start":
                    self._do_start("replay")
                elif cmd == "stop":
                    self._do_stop()
                elif cmd == "record_start":
                    while self._saving:
                        time.sleep(0.05)
                    if self.state != "on" or self.mode != "recording":
                        if self.state != "off":
                            self._do_stop(emit=False)
                        self._do_start("recording")
                elif cmd == "record_stop":
                    if self.mode == "recording":
                        self._do_stop()
                elif cmd == "restart":
                    if self.mode == "recording":
                        continue
                    if self.state != "off" or self.proc is not None:
                        self._do_stop(emit=False)
                        self._do_start("replay")
                elif cmd == "revive":
                    self._do_stop(emit=False)
                    self._do_start("replay")
                elif cmd == "record_crash":
                    if self.mode == "recording":
                        self.error.emit("Screen capture stopped unexpectedly. Saving the recording so far.")
                        self._do_stop()
            except Exception as e:  # never let the worker die
                self._log(f"worker error: {e!r}")
                had_recording = self.mode == "recording" or cmd in ("record_start", "record_stop", "record_crash")
                p, self.proc = self.proc, None
                if p is not None and p.poll() is None:
                    try:
                        p.kill()
                        p.wait(2)
                    except Exception:
                        pass
                self._stop_audio()
                if had_recording:
                    self._cleanup_recording_files(self._record_video,
                                                  [path for _name, path, _cap in self._record_audio])
                    self._record_video = self._record_output = ""
                    self._record_audio = []
                self.mode = "off"
                self._set_state("off")
                if had_recording:
                    self.recording_failed.emit(str(e))
                self.error.emit(f"Something went wrong: {e}")

    def _set_state(self, st):
        if st != self.state:
            self.state = st
            self.state_changed.emit(st)

    def _log(self, text):
        try:
            with open(self.log_path, "a", encoding="utf-8") as f:
                f.write(f"[{datetime.datetime.now():%H:%M:%S}] {text}\n")
        except Exception:
            pass

    def _monitor(self):
        self.monitors = winapi.list_monitors()
        idx = self.s.monitor if 0 <= self.s.monitor < len(self.monitors) else 0
        return idx, self.monitors[idx]

    def _attempts(self, encs):
        s = self.s
        key = self.resolve_encoder(encs)
        fps = int(s.fps)
        cur = "1" if s.capture_cursor else "0"
        idx, mon = self._monitor()
        target_h = None if s.resolution == "native" else int(s.resolution)
        if target_h and target_h >= mon["h"]:
            target_h = None
        scale = f",scale=-2:{target_h}:flags=bicubic" if target_h else ""
        out_w = mon["w"] if not target_h else int(round(mon["w"] * target_h / mon["h"] / 2) * 2)
        out_h = target_h or mon["h"]
        size = f"{out_w}×{out_h}"
        attempts = []
        if TEST_MODE:
            src = ["-re", "-f", "lavfi", "-i", f"testsrc2=size=1280x720:rate={fps}"]
            attempts.append(("test", src, ["-vf", "format=yuv420p"], "x264", "1280×720"))
            return attempts
        if IS_MAC:
            avf = ["-f", "avfoundation", "-capture_cursor", cur, "-framerate", str(fps), "-pixel_format", "nv12",
                   "-i", avf_screen_input(self.ffmpeg, idx)]
            pix = "yuv420p" if key == "x264" else "nv12"
            cfr = ["-fps_mode", "cfr", "-r", str(fps)]
            attempts.append(("avfoundation", avf, cfr + ["-vf", f"{scale.lstrip(',') or 'null'},format={pix}"], key, size))
            if key != "x264":
                attempts.append(("avfoundation-cpu", avf, cfr + ["-vf", f"{scale.lstrip(',') or 'null'},format=yuv420p"],
                                 "x264", size))
            return attempts
        dda = ["-f", "lavfi", "-i", f"ddagrab=output_idx={idx}:framerate={fps}:draw_mouse={cur}"]
        pix = "yuv420p" if key == "x264" else "nv12"
        if key in ("nvenc", "amf") and not target_h:
            attempts.append(("ddagrab-gpu", dda, [], key, size))
        attempts.append(("ddagrab", dda, ["-vf", f"hwdownload,format=bgra{scale},format={pix}"], key, size))
        gdi = ["-f", "gdigrab", "-framerate", str(fps), "-draw_mouse", cur,
               "-offset_x", str(mon["x"]), "-offset_y", str(mon["y"]),
               "-video_size", f"{mon['w']}x{mon['h']}", "-i", "desktop"]
        attempts.append(("gdigrab", gdi, ["-vf", f"{scale.lstrip(',') or 'null'},format={pix}"], key, size))
        if key != "x264":
            attempts.append(("gdigrab-cpu", gdi, ["-vf", f"{scale.lstrip(',') or 'null'},format=yuv420p"], "x264", size))
        return attempts

    def _launch(self, att, output_path=None) -> bool:
        name, src, vf, key, size = att
        s = self.s
        if output_path:
            out_args = ["-f", "matroska", output_path]
        else:
            out = os.path.join(self.buf_dir, "seg%03d.ts")
            out_args = ["-f", "segment", "-segment_time", "1", "-segment_wrap", str(SEGMENT_WRAP),
                        "-segment_format", "mpegts", out]
        cmd = [self.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostats", "-y", *src, *vf,
               "-map", "0:v", "-an", *encoder_args(key, s.codec, int(s.bitrate_mbps), int(s.fps)), *out_args]
        self._log(f"try {name}: " + " ".join(cmd))
        try:
            log = open(self.log_path, "ab")
            proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=log,
                                    **winapi.popen_kwargs())
        except OSError as e:
            self._log(f"launch failed: {e}")
            return False
        self._write_pid(proc.pid)
        deadline = time.monotonic() + (12 if IS_MAC else 7)  # AVFoundation takes a few seconds to open a display
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                self._log(f"{name} exited with {proc.returncode}")
                return False
            has_output = (os.path.isfile(output_path) and os.path.getsize(output_path) > 2048) if output_path else \
                         any(os.path.getsize(f) > 0 for f in glob.glob(os.path.join(self.buf_dir, "seg*.ts")))
            if has_output:
                self.proc = proc
                self.active_encoder = key
                label = ENCODERS[key]["label"]
                self.pipeline = f"{label} · {s.codec.upper().replace('HEVC', 'HEVC')} · {size} @ {s.fps} fps"
                return True
            time.sleep(0.1)
        self._log(f"{name} produced no output")
        try:
            proc.kill()
            proc.wait(2)
        except Exception:
            pass
        return False

    def _clear_buffer(self):
        for f in glob.glob(os.path.join(self.buf_dir, "seg*.ts")):
            try:
                os.remove(f)
            except OSError:
                pass

    def _do_start(self, mode="replay"):
        if self.proc is not None and self.proc.poll() is None:
            if self.mode == "recording" and mode == "replay":
                return
            if self.mode != mode:
                return
            self._set_state("on")
            return
        self.mode = mode
        self._set_state("starting")
        if not self.ffmpeg:
            self.ffmpeg = find_ffmpeg()
        if not self.ffmpeg:
            self.mode = "off"
            self._set_state("off")
            msg = ("FFmpeg wasn't found. Install it with Homebrew: brew install ffmpeg" if IS_MAC else
                   "FFmpeg wasn't found. Run install.bat, or put ffmpeg.exe in Rewind's ffmpeg\\bin folder.")
            if mode == "recording":
                self.recording_failed.emit(msg)
            self.error.emit(msg)
            return
        encs = self.detect_encoders()
        record_video = ""
        if mode == "recording":
            token = uuid.uuid4().hex
            self._record_video = os.path.join(self.work_dir, f"recording_{token}.mkv")
            self._record_output = ""
            self._record_started_at = time.monotonic()
            self._record_codec = self.s.codec
            self._record_volumes = {"system": self.s.system_volume / 100.0,
                                    "mic": self.s.mic_volume / 100.0}
            record_video = self._record_video
        else:
            self._clear_buffer()
        ok = False
        for att in self._attempts(encs):
            if self._launch(att, output_path=record_video):
                ok = True
                break
            if record_video:
                try:
                    os.remove(record_video)
                except OSError:
                    pass
            else:
                self._clear_buffer()
        if not ok:
            self.mode = "off"
            self._record_video = self._record_output = ""
            self._set_state("off")
            msg = "Couldn't start screen capture. Try another encoder on the Replay page " \
                  "(details are in ffmpeg.log)."
            if mode == "recording":
                self.recording_failed.emit(msg)
            self.error.emit(msg)
            return
        self._start_audio(recording=(mode == "recording"))
        self.started_at = time.monotonic()
        self._set_state("on")

    def _new_recording_path(self):
        os.makedirs(self.s.output_dir, exist_ok=True)
        stamp = datetime.datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
        path = os.path.join(self.s.output_dir, f"Rewind_Recording_{stamp}.mp4")
        n = 2
        while os.path.exists(path):
            path = os.path.join(self.s.output_dir, f"Rewind_Recording_{stamp}_{n}.mp4")
            n += 1
        return path

    def _start_audio(self, recording=False):
        self._stop_audio()
        notes = []
        self._record_audio = []
        token = uuid.uuid4().hex
        if self.s.system_audio:
            cap = make_capture("loopback")
            audio_path = os.path.join(self.work_dir, f"recording_system_{token}.pcm") if recording else None
            if cap.start(audio_path):
                self._audio["system"] = cap
                if recording:
                    self._record_audio.append(("system", audio_path, cap))
            else:
                if audio_path:
                    try:
                        os.remove(audio_path)
                    except OSError:
                        pass
                notes.append(f"desktop audio unavailable ({cap.error})")
        if self.s.mic_enabled:
            cap = make_capture("mic", self.s.mic_device)
            audio_path = os.path.join(self.work_dir, f"recording_mic_{token}.pcm") if recording else None
            if cap.start(audio_path):
                self._audio["mic"] = cap
                if recording:
                    self._record_audio.append(("mic", audio_path, cap))
            else:
                if audio_path:
                    try:
                        os.remove(audio_path)
                    except OSError:
                        pass
                notes.append(f"microphone unavailable ({cap.error})")
        self.audio_note = "; ".join(notes)
        if notes:
            self._log("audio: " + self.audio_note)

    def _stop_audio(self):
        for cap in self._audio.values():
            cap.stop()
        self._audio = {}

    def _pid_path(self):
        return os.path.join(data_dir(), "ffmpeg.pid")

    def _write_pid(self, pid):
        try:
            with open(self._pid_path(), "w") as f:
                f.write(str(pid))
        except OSError:
            pass

    def _kill_stale_ffmpeg(self):
        """An FFmpeg left over from a Rewind that was force-quit still holds the screen; stop it."""
        try:
            with open(self._pid_path()) as f:
                pid = int(f.read().strip())
            os.remove(self._pid_path())
        except (OSError, ValueError):
            return
        if IS_WIN:
            return
        r = run(["ps", "-p", str(pid), "-o", "command="], timeout=5)
        if self.buf_dir.encode() in (r.stdout or b"") or self.work_dir.encode() in (r.stdout or b""):
            try:
                os.kill(pid, 9)
            except OSError:
                pass

    def _do_stop(self, emit=True):
        previous_mode = self.mode
        p, self.proc = self.proc, None
        if p is not None:
            try:
                p.stdin.write(b"q")
                p.stdin.flush()
            except Exception:
                pass
            try:
                p.wait(4)
            except Exception:
                try:
                    p.kill()
                    p.wait(2)
                except Exception:
                    pass
        self._stop_audio()
        self.mode = "off"
        if previous_mode == "recording" and self._record_video:
            self._set_state("saving" if emit else "off")
            self._finish_recording()
        elif emit:
            self._set_state("off")
        else:
            self.state = "off"

    def _finish_recording(self):
        video = self._record_video
        out = self._new_recording_path()
        audio_paths = [path for _name, path, _cap in self._record_audio]
        audio_tracks = []
        for name, path, cap in self._record_audio:
            if cap.record_error:
                self.error.emit(f"{name.title()} audio could not be saved: {cap.record_error}")
                continue
            if os.path.isfile(path) and os.path.getsize(path) > 0:
                offset = max(0.0, cap.started_at - self._record_started_at)
                audio_tracks.append((path, self._record_volumes.get(name, 1.0), offset,
                                     cap.ring.rate, cap.ring.channels))
        if not video or not os.path.isfile(video) or os.path.getsize(video) == 0:
            self._cleanup_recording_files(video, audio_paths)
            self._record_audio = []
            self._record_video = self._record_output = ""
            self._set_state("off")
            self.recording_failed.emit("No video was captured")
            return
        duration = probe_duration(self.ffmpeg, video) or max(0.0, time.monotonic() - self._record_started_at)
        timeout = max(180, int(duration * 2))
        result = self._mux_video(video, out, audio_tracks, self._record_codec, timeout)
        if result.returncode != 0 and audio_tracks:
            self._log("recording audio mux failed: " + (result.stderr or b"").decode("utf-8", "replace")[-800:])
            result = self._mux_video(video, out, [], self._record_codec, timeout)
            if result.returncode == 0:
                self.error.emit("Recording saved without audio because audio muxing failed.")
        if result.returncode != 0 or not os.path.isfile(out) or os.path.getsize(out) == 0:
            self._log("recording mux failed: " + (result.stderr or b"").decode("utf-8", "replace")[-800:])
            self._cleanup_recording_files(video, audio_paths)
            try:
                if os.path.exists(out):
                    os.remove(out)
            except OSError:
                pass
            self._record_audio = []
            self._record_video = self._record_output = ""
            self._set_state("off")
            self.recording_failed.emit("Couldn't save the recording")
            return
        self._cleanup_recording_files(video, audio_paths)
        self._record_audio = []
        self._record_video = self._record_output = ""
        try:
            update_meta(os.path.basename(out), duration=round(duration, 2))
        except Exception:
            pass
        self._set_state("off")
        self.recording_saved.emit(out, duration)
        if make_thumbnail(self.ffmpeg, out, duration):
            self.thumbnail_ready.emit(out)

    def _cleanup_recording_files(self, video, audio_paths):
        for path in [video, *audio_paths]:
            if path:
                try:
                    os.remove(path)
                except OSError:
                    pass

    def _watchdog(self):
        while True:
            time.sleep(1.0)
            p = self.proc
            if self.state == "on" and p is not None and p.poll() is not None and p is self.proc:
                now = time.monotonic()
                self._revives = [t for t in self._revives if now - t < 60]
                self._log(f"capture died (code {p.returncode})")
                if self.mode == "recording":
                    self._q.put("record_crash")
                elif len(self._revives) < 3:
                    self._revives.append(now)
                    self._q.put("revive")
                else:
                    self.proc = None
                    self._stop_audio()
                    self._set_state("off")
                    self.error.emit("Screen capture keeps stopping. Check ffmpeg.log or try another encoder.")

    def _mux_video(self, video, output, audio_tracks, codec, timeout=180):
        cmd = [self.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", video]
        for path, _volume, offset, rate, channels in audio_tracks:
            if offset > 0:
                cmd += ["-itsoffset", f"{offset:.3f}"]
            cmd += ["-f", "s16le", "-ar", str(rate), "-ac", str(channels), "-i", path]
        if len(audio_tracks) == 1:
            cmd += ["-map", "0:v", "-map", "1:a", "-filter:a",
                    f"volume={audio_tracks[0][1]:.2f}", "-c:a", "aac", "-b:a", "192k", "-ac", "2", "-shortest"]
        elif len(audio_tracks) > 1:
            fc = ";".join(f"[{i + 1}:a]aformat=sample_rates=48000:channel_layouts=stereo,"
                           f"volume={volume:.2f}[a{i}]"
                           for i, (_path, volume, _offset, _rate, _channels) in enumerate(audio_tracks))
            fc += ";" + "".join(f"[a{i}]" for i in range(len(audio_tracks))) + \
                  f"amix=inputs={len(audio_tracks)}:duration=longest:normalize=0[aout]"
            cmd += ["-filter_complex", fc, "-map", "0:v", "-map", "[aout]",
                    "-c:a", "aac", "-b:a", "192k", "-ac", "2", "-shortest"]
        else:
            cmd += ["-map", "0:v", "-an"]
        cmd += ["-c:v", "copy"]
        if codec == "hevc":
            cmd += ["-tag:v", "hvc1"]
        cmd += ["-movflags", "+faststart", output]
        return run(cmd, timeout=timeout)

    # ---- clipping
    def save_clip(self, seconds=None):
        if self.state != "on" or self.mode != "replay":
            self.clip_failed.emit("Instant Replay is off")
            return False
        if self._saving:
            return False
        self._saving = True
        self.clip_started.emit()
        threading.Thread(target=self._do_clip, args=(int(seconds or self.s.clip_seconds),), daemon=True).start()
        return True

    def _do_clip(self, seconds):
        try:
            self._clip(seconds)
        except Exception as e:
            self._log(f"clip error: {e!r}")
            self.clip_failed.emit(str(e))
        finally:
            self._saving = False

    def _clip(self, seconds):
        s = self.s
        time.sleep(0.35)  # let the encoder flush the last frames
        t_end = time.monotonic() - s.audio_offset_ms / 1000.0
        snaps = {}
        for name, cap in list(self._audio.items()):
            ring = cap.ring
            snaps[name] = (ring.snapshot(seconds + 4, t_end), ring.rate, ring.channels, ring.bpf)

        segs = [f for f in glob.glob(os.path.join(self.buf_dir, "seg*.ts")) if os.path.getsize(f) > 0]
        if not segs:
            self.clip_failed.emit("Nothing recorded yet")
            return
        segs.sort(key=os.path.getmtime)
        chosen = [os.path.basename(f) for f in segs[-(seconds + 1):]]
        tmp_ts = os.path.join(self.work_dir, "clip.ts")
        r = run([self.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", "concat:" + "|".join(chosen),
                 "-map", "0:v", "-c", "copy", "-f", "mpegts", tmp_ts], cwd=self.buf_dir, timeout=90)
        if r.returncode != 0 or not os.path.exists(tmp_ts):
            self._log("concat failed: " + (r.stderr or b"").decode("utf-8", "replace")[-800:])
            self.clip_failed.emit("Couldn't read the replay buffer")
            return
        dur = probe_duration(self.ffmpeg, tmp_ts) or float(len(chosen))

        os.makedirs(s.output_dir, exist_ok=True)
        stamp = datetime.datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
        out = os.path.join(s.output_dir, f"Rewind_{stamp}.mp4")
        n = 2
        while os.path.exists(out):
            out = os.path.join(s.output_dir, f"Rewind_{stamp}_{n}.mp4")
            n += 1

        inputs, vols = [], []
        for name, (pcm, rate, ch, bpf) in snaps.items():
            need = int(dur * rate) * bpf
            pcm = pcm[-need:] if len(pcm) >= need else bytes(need - len(pcm)) + pcm
            wav = os.path.join(self.work_dir, f"{name}.wav")
            with wave.open(wav, "wb") as w:
                w.setnchannels(ch)
                w.setsampwidth(2)
                w.setframerate(rate)
                w.writeframes(pcm)
            inputs.append(wav)
            vols.append((s.system_volume if name == "system" else s.mic_volume) / 100.0)

        def build(with_audio):
            cmd = [self.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", tmp_ts]
            if with_audio and inputs:
                for wav in inputs:
                    cmd += ["-i", wav]
                if len(inputs) == 1:
                    cmd += ["-map", "0:v", "-map", "1:a", "-filter:a", f"volume={vols[0]:.2f}"]
                else:
                    fc = ";".join(f"[{i + 1}:a]aformat=sample_rates=48000:channel_layouts=stereo,"
                                  f"volume={v:.2f}[a{i}]" for i, v in enumerate(vols))
                    fc += ";" + "".join(f"[a{i}]" for i in range(len(vols))) + \
                          f"amix=inputs={len(vols)}:duration=longest:normalize=0[aout]"
                    cmd += ["-filter_complex", fc, "-map", "0:v", "-map", "[aout]"]
                cmd += ["-c:a", "aac", "-b:a", "192k", "-ac", "2"]
            else:
                cmd += ["-map", "0:v"]
            cmd += ["-c:v", "copy"]
            if s.codec == "hevc":
                cmd += ["-tag:v", "hvc1"]
            cmd += ["-movflags", "+faststart", out]
            return cmd

        r = run(build(True), timeout=180)
        if r.returncode != 0 and inputs:
            self._log("mux with audio failed: " + (r.stderr or b"").decode("utf-8", "replace")[-800:])
            r = run(build(False), timeout=180)
        if r.returncode != 0 or not os.path.exists(out):
            self._log("mux failed: " + (r.stderr or b"").decode("utf-8", "replace")[-800:])
            self.clip_failed.emit("Couldn't write the clip file")
            return
        try:
            update_meta(os.path.basename(out), duration=round(dur, 2))
        except Exception:
            pass
        self.clip_saved.emit(out, dur)
        if make_thumbnail(self.ffmpeg, out, dur):
            self.thumbnail_ready.emit(out)
