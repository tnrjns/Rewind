"""Rolling in-memory audio buffers (desktop audio + microphone).

Windows captures through PyAudioWPatch (WASAPI loopback for desktop audio).
Other platforms capture the microphone through sounddevice (PortAudio).

WASAPI loopback delivers *no data* while nothing is playing, so every chunk is
time-stamped and gaps are filled with silence. That keeps audio locked to the
video no matter how long the desktop was quiet.
"""
import math
import os
import queue
import threading
import time
from collections import deque

try:
    import pyaudiowpatch as pyaudio  # Windows only
except Exception:  # pragma: no cover
    pyaudio = None

try:
    import sounddevice as sd
except Exception:  # pragma: no cover
    sd = None

MAX_SECONDS = 75


class PcmRing:
    def __init__(self, rate: int, channels: int, max_seconds: int = MAX_SECONDS):
        self.rate = rate
        self.channels = channels
        self.bpf = 2 * channels  # 16-bit samples
        self.max_bytes = int(rate * max_seconds) * self.bpf
        self._chunks = deque()
        self._size = 0
        self._end = None  # monotonic time of the last buffered sample
        self._lock = threading.Lock()

    def _append(self, b: bytes):
        self._chunks.append(b)
        self._size += len(b)
        while self._chunks and self._size - len(self._chunks[0]) >= self.max_bytes:
            self._size -= len(self._chunks.popleft())

    def _silence(self, seconds: float) -> bytes:
        return bytes(int(max(0.0, seconds) * self.rate) * self.bpf)

    def push(self, data: bytes, now: float):
        frames = len(data) // self.bpf
        start = now - frames / self.rate
        with self._lock:
            if self._end is not None:
                gap = start - self._end
                if gap > 0.03:
                    self._append(self._silence(min(gap, MAX_SECONDS)))
            self._append(data)
            self._end = now

    def snapshot(self, seconds: float, end_time: float) -> bytes:
        """Exactly `seconds` of audio ending at monotonic `end_time`."""
        need = int(seconds * self.rate) * self.bpf
        with self._lock:
            data = b"".join(self._chunks)
            last = self._end
        if last is None:
            return bytes(need)
        tail = end_time - last
        if tail > 0:
            data += self._silence(min(tail, seconds))
        elif tail < 0:
            cut = int(-tail * self.rate) * self.bpf
            data = data[:-cut] if cut < len(data) else b""
        if len(data) >= need:
            return data[len(data) - need:]
        return bytes(need - len(data)) + data


_pa = None
_pa_lock = threading.Lock()
_active = 0


def _get_pa(fresh=False):
    global _pa
    with _pa_lock:
        if fresh and _pa is not None and _active == 0:
            try:
                _pa.terminate()
            except Exception:
                pass
            _pa = None
        if _pa is None:
            _pa = pyaudio.PyAudio()
        return _pa


def available() -> bool:
    return pyaudio is not None or sd is not None


def list_microphones() -> list:
    if pyaudio is None:
        if sd is None:
            return []
        try:
            return [d["name"] for d in sd.query_devices() if d.get("max_input_channels", 0) > 0]
        except Exception:
            return []
    try:
        pa = _get_pa()
        wasapi = pa.get_host_api_info_by_type(pyaudio.paWASAPI)
        names = []
        for i in range(pa.get_device_count()):
            d = pa.get_device_info_by_index(i)
            if d.get("hostApi") == wasapi["index"] and d.get("maxInputChannels", 0) > 0 \
                    and not d.get("isLoopbackDevice", False):
                names.append(d["name"])
        return names
    except Exception:
        return []


class AudioCapture:
    """kind = 'loopback' (what you hear) or 'mic'."""

    def __init__(self, kind: str, device_name: str = ""):
        self.kind = kind
        self.device_name = device_name
        self.ring = None
        self.stream = None
        self.error = ""
        self.record_error = ""
        self.started_at = 0.0
        self._record_queue = None
        self._record_thread = None
        self._record_file = None
        self._record_end = None

    def start(self, record_path=None) -> bool:
        global _active
        if pyaudio is None:
            self.error = "PyAudioWPatch is not installed"
            return False
        try:
            pa = _get_pa(fresh=True)
            wasapi = pa.get_host_api_info_by_type(pyaudio.paWASAPI)
            dev = None
            if self.kind == "loopback":
                spk = pa.get_device_info_by_index(wasapi["defaultOutputDevice"])
                if spk.get("isLoopbackDevice"):
                    dev = spk
                else:
                    for lb in pa.get_loopback_device_info_generator():
                        if spk["name"] in lb["name"]:
                            dev = lb
                            break
            else:
                if self.device_name:
                    for i in range(pa.get_device_count()):
                        d = pa.get_device_info_by_index(i)
                        if d.get("hostApi") == wasapi["index"] and d["name"] == self.device_name \
                                and d.get("maxInputChannels", 0) > 0 and not d.get("isLoopbackDevice", False):
                            dev = d
                            break
                if dev is None and wasapi.get("defaultInputDevice", -1) >= 0:
                    dev = pa.get_device_info_by_index(wasapi["defaultInputDevice"])
            if dev is None:
                self.error = "No audio device found"
                return False
            rate = int(dev["defaultSampleRate"])
            ch = max(1, min(8, int(dev["maxInputChannels"])))
            self.ring = PcmRing(rate, ch)
            self._open_recording(record_path)
            self.stream = pa.open(format=pyaudio.paInt16, channels=ch, rate=rate, input=True,
                                  input_device_index=dev["index"], frames_per_buffer=max(256, rate // 50),
                                  stream_callback=self._cb)
            self.started_at = time.monotonic()
            self._record_end = self.started_at
            self.stream.start_stream()
            with _pa_lock:
                _active += 1
            return True
        except Exception as e:  # device busy, unplugged, etc.
            self.error = str(e)
            if self.stream is not None:
                try:
                    self.stream.close()
                except Exception:
                    pass
            self.stream = None
            self._finish_recording()
            return False

    def _cb(self, data, frames, info, status):
        self._push(data, frames)
        return (None, pyaudio.paContinue)

    def _open_recording(self, record_path):
        if not record_path:
            return
        os.makedirs(os.path.dirname(record_path), exist_ok=True)
        self._record_file = open(record_path, "wb")
        self.record_error = ""
        self._record_queue = queue.Queue()
        self._record_thread = threading.Thread(target=self._write_recording, daemon=True)
        self._record_thread.start()

    def _push(self, data, frames):
        now = time.monotonic()
        self.ring.push(data, now)
        if self._record_queue is not None:
            start = now - frames / self.ring.rate
            gap = max(0, int((start - self._record_end) * self.ring.rate))
            if gap > self.ring.rate // 40:
                self._record_queue.put(("silence", gap))
            self._record_queue.put(("data", data))
            self._record_end = max(now, self._record_end)

    def _write_recording(self):
        silence = bytes(self.ring.rate * self.ring.bpf)
        try:
            while True:
                item = self._record_queue.get()
                if item is None:
                    break
                kind, payload = item
                if kind == "data":
                    self._record_file.write(payload)
                else:
                    while payload:
                        frames = min(payload, self.ring.rate)
                        self._record_file.write(silence[:frames * self.ring.bpf])
                        payload -= frames
        except Exception as e:
            self.record_error = str(e)
            while self._record_queue.get() is not None:
                pass

    def _finish_recording(self):
        if self._record_queue is not None:
            if self._record_end:
                tail = max(0, int((time.monotonic() - self._record_end) * self.ring.rate))
                if tail > self.ring.rate // 40:
                    self._record_queue.put(("silence", tail))
            self._record_queue.put(None)
            if self._record_thread is not None:
                self._record_thread.join()
            if self._record_file is not None:
                try:
                    self._record_file.close()
                except Exception as e:
                    self.record_error = self.record_error or str(e)
        self._record_queue = None
        self._record_thread = None
        self._record_file = None
        self._record_end = None

    def stop(self):
        global _active
        if self.stream is not None:
            try:
                self.stream.stop_stream()
                self.stream.close()
            except Exception:
                pass
            self.stream = None
            with _pa_lock:
                _active = max(0, _active - 1)
        self._finish_recording()


class SoundDeviceCapture(AudioCapture):
    """Microphone capture through PortAudio. Desktop audio needs ScreenCaptureKit and isn't wired up yet."""

    def start(self, record_path=None) -> bool:
        if self.kind == "loopback":
            self.error = "desktop audio isn't supported on this system yet"
            return False
        if sd is None:
            self.error = "sounddevice is not installed"
            return False
        try:
            index = None
            if self.device_name:
                for i, d in enumerate(sd.query_devices()):
                    if d["name"] == self.device_name and d.get("max_input_channels", 0) > 0:
                        index = i
                        break
            dev = sd.query_devices(index, "input")
            rate = int(dev["default_samplerate"])
            ch = max(1, min(2, int(dev["max_input_channels"])))
            self.ring = PcmRing(rate, ch)
            self._open_recording(record_path)
            self.stream = sd.RawInputStream(samplerate=rate, channels=ch, dtype="int16", device=index,
                                            blocksize=max(256, rate // 50), callback=self._sd_cb)
            self.started_at = time.monotonic()
            self._record_end = self.started_at
            self.stream.start()
            return True
        except Exception as e:
            self.error = str(e)
            if self.stream is not None:
                try:
                    self.stream.close()
                except Exception:
                    pass
            self.stream = None
            self._finish_recording()
            return False

    def _sd_cb(self, indata, frames, time_info, status):
        self._push(bytes(indata), frames)

    def stop(self):
        if self.stream is not None:
            try:
                self.stream.stop()
                self.stream.close()
            except Exception:
                pass
            self.stream = None
        self._finish_recording()


class FakeCapture:
    """Test-only source: a tone that keeps going silent (like idle loopback)."""

    def __init__(self, kind="loopback", device_name=""):
        self.kind = kind
        self.ring = PcmRing(48000, 2)
        self._stop = threading.Event()
        self.error = ""
        self.record_error = ""
        self.started_at = 0.0
        self._record_path = ""
        self._thread = None

    def start(self, record_path=None):
        self._record_path = record_path or ""
        self.started_at = time.monotonic()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return True

    def _run(self):
        import array
        rate, n = 48000, 960
        i = 0
        t0 = time.monotonic()
        out = open(self._record_path, "wb") if self._record_path else None
        while not self._stop.is_set():
            now = time.monotonic()
            buf = array.array("h")
            if int(now - t0) % 4 != 3:  # silent (no callbacks) 1 s out of every 4
                for k in range(n):
                    v = int(8000 * math.sin(2 * math.pi * 440 * (i + k) / rate))
                    buf.extend((v, v))
                data = buf.tobytes()
            else:
                data = bytes(n * 4)
            self.ring.push(data, now)
            if out:
                out.write(data)
            i += n
            time.sleep(n / rate)
        if out:
            out.close()

    def stop(self):
        self._stop.set()
        if self._thread is not None:
            self._thread.join()
            self._thread = None


def make_capture(kind, device_name=""):
    if os.environ.get("REWIND_TEST_SOURCE"):
        return FakeCapture(kind, device_name)
    if pyaudio is None and sd is not None:
        return SoundDeviceCapture(kind, device_name)
    return AudioCapture(kind, device_name)
