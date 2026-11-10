"""Synthesised feedback sounds (no audio files needed) + playback."""
import array
import math
import os
import sys
import wave

from .settings import sub_dir

RATE = 44100
IS_WIN = sys.platform == "win32"

STYLES = [("chime", "Chime"), ("pop", "Pop"), ("blip", "Double blip"), ("custom", "Custom .wav file…")]


def _tone(out, start, freq, amp, decay, dur=None, harmonics=(1.0, 0.18, 0.06), attack=0.004):
    s0 = int(start * RATE)
    n = len(out) - s0 if dur is None else min(len(out) - s0, int(dur * RATE))
    w = 2 * math.pi * freq / RATE
    for i in range(n):
        t = i / RATE
        env = min(1.0, t / attack) * math.exp(-t * decay)
        v = 0.0
        for k, h in enumerate(harmonics, start=1):
            v += h * math.sin(w * k * i)
        out[s0 + i] += amp * env * v


def synth(style: str) -> list:
    if style == "pop":
        n = int(RATE * 0.16)
        out = [0.0] * n
        phase = 0.0
        for i in range(n):
            t = i / RATE
            f = 260 + 900 * math.exp(-t * 28)
            phase += 2 * math.pi * f / RATE
            env = min(1.0, t / 0.002) * math.exp(-t * 30)
            out[i] = env * math.sin(phase)
        return out
    if style == "shutter":
        import random
        rnd = random.Random(7)
        out = [0.0] * int(RATE * 0.2)
        for start, amp in ((0.0, 1.0), (0.065, 0.75)):
            s0 = int(start * RATE)
            prev = 0.0
            for i in range(int(RATE * 0.05)):
                t = i / RATE
                prev = 0.55 * prev + 0.45 * rnd.uniform(-1, 1)   # soft low-pass on noise
                thump = 0.6 * math.sin(2 * math.pi * 190 * t) * math.exp(-t * 60)
                out[s0 + i] += amp * (prev * math.exp(-t * 110) + thump)
        return out
    if style == "blip":
        out = [0.0] * int(RATE * 0.22)
        for start in (0.0, 0.085):
            _tone(out, start, 1318.5, 0.8, 38, dur=0.07, harmonics=(1.0, 0.0, 0.25), attack=0.003)
        return out
    if style == "toggle_on":
        out = [0.0] * int(RATE * 0.35)
        _tone(out, 0.0, 783.99, 0.6, 16)
        _tone(out, 0.07, 1174.66, 0.7, 14)
        return out
    if style == "toggle_off":
        out = [0.0] * int(RATE * 0.35)
        _tone(out, 0.0, 1174.66, 0.6, 16)
        _tone(out, 0.07, 783.99, 0.7, 14)
        return out
    # chime (default)
    out = [0.0] * int(RATE * 0.8)
    _tone(out, 0.0, 1046.5, 0.8, 7.5)
    _tone(out, 0.09, 1567.98, 0.9, 6.5)
    return out


def _write_wav(samples, path, volume):
    peak = max(1e-6, max(abs(x) for x in samples))
    gain = 0.9 / peak * max(0, min(100, volume)) / 100.0
    data = array.array("h", (int(max(-1.0, min(1.0, x * gain)) * 32767) for x in samples))
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(data.tobytes())


def _scaled_custom(path, volume):
    """Return a volume-scaled copy of a custom 16-bit wav (or the file itself)."""
    try:
        st = os.stat(path)
        out = os.path.join(sub_dir("sounds"), f"custom_{int(st.st_mtime)}_{st.st_size}_{volume}.wav")
        if os.path.exists(out):
            return out
        with wave.open(path, "rb") as r:
            params = r.getparams()
            if params.sampwidth != 2:
                return path
            frames = array.array("h")
            frames.frombytes(r.readframes(params.nframes))
        if sys.byteorder == "big":
            frames.byteswap()
        g = volume / 100.0
        scaled = array.array("h", (int(max(-32768, min(32767, v * g))) for v in frames))
        with wave.open(out, "wb") as w:
            w.setparams(params)
            w.writeframes(scaled.tobytes())
        return out
    except Exception:
        return path if os.path.exists(path) else None


def sound_file(style: str, volume: int, custom: str = ""):
    if style == "custom":
        if custom and os.path.exists(custom):
            return _scaled_custom(custom, volume)
        style = "chime"
    path = os.path.join(sub_dir("sounds"), f"{style}_{volume}.wav")
    if not os.path.exists(path):
        _write_wav(synth(style), path, volume)
    return path


def play_file(path) -> None:
    if not path:
        return
    if IS_WIN:
        try:
            import winsound
            winsound.PlaySound(path, winsound.SND_FILENAME | winsound.SND_ASYNC | winsound.SND_NODEFAULT)
        except Exception:
            pass


def play(style: str, volume: int, custom: str = "") -> None:
    play_file(sound_file(style, volume, custom))
