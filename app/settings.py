"""Persistent settings for Rewind."""
import json
import os
import sys
import threading
from dataclasses import dataclass, field, asdict, fields

APP_NAME = "Rewind"
APP_VERSION = "1.0.0"
IS_WIN = sys.platform == "win32"


def data_dir() -> str:
    if IS_WIN:
        base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
    else:
        base = os.path.join(os.path.expanduser("~"), ".local", "share")
    d = os.path.join(base, APP_NAME)
    os.makedirs(d, exist_ok=True)
    return d


def sub_dir(name: str) -> str:
    d = os.path.join(data_dir(), name)
    os.makedirs(d, exist_ok=True)
    return d


def default_clips_dir() -> str:
    videos = os.path.join(os.path.expanduser("~"), "Videos")
    if IS_WIN:
        try:
            import ctypes
            import uuid
            # FOLDERID_Videos {18989B1D-99B5-455B-841C-AB7C74E4DDFC}
            guid = uuid.UUID("{18989B1D-99B5-455B-841C-AB7C74E4DDFC}")

            class GUID(ctypes.Structure):
                _fields_ = [("data", ctypes.c_byte * 16)]

            g = GUID()
            ctypes.memmove(g.data, guid.bytes_le, 16)
            path_ptr = ctypes.c_wchar_p()
            shell32 = ctypes.WinDLL("shell32")
            if shell32.SHGetKnownFolderPath(ctypes.byref(g), 0, None, ctypes.byref(path_ptr)) == 0:
                videos = path_ptr.value
                ctypes.WinDLL("ole32").CoTaskMemFree(path_ptr)
        except Exception:
            pass
    return os.path.join(videos, "Rewind")


def app_base_dir() -> str:
    """Folder the app runs from (next to Rewind.exe or main.py)."""
    if getattr(sys, "frozen", False):
        return os.path.dirname(sys.executable)
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


@dataclass
class Settings:
    # Replay
    clip_seconds: int = 30
    bitrate_mbps: int = 50
    fps: int = 60
    resolution: str = "native"      # native | 1440 | 1080 | 720
    monitor: int = 0
    encoder: str = "auto"           # auto | nvenc | amf | qsv | x264
    codec: str = "h264"             # h264 | hevc
    capture_cursor: bool = True
    # Audio
    system_audio: bool = True
    system_volume: int = 100
    mic_enabled: bool = False
    mic_device: str = ""
    mic_volume: int = 100
    audio_offset_ms: int = 0
    # Saving
    output_dir: str = field(default_factory=default_clips_dir)
    # Hotkeys  (mods = Win32 MOD_* flags, vk = virtual-key code)
    hotkey_toggle: dict = field(default_factory=lambda: {"mods": 1, "vk": 0x78, "text": "Alt + F9"})
    hotkey_clip: dict = field(default_factory=lambda: {"mods": 1, "vk": 0x79, "text": "Alt + F10"})
    hotkey_record: dict = field(default_factory=lambda: {"mods": 1, "vk": 0x7A, "text": "Alt + F11"})
    hotkey_screenshot: dict = field(default_factory=lambda: {"mods": 1, "vk": 0x7B, "text": "Alt + F12"})
    hotkey_open: dict = field(default_factory=lambda: {"mods": 1, "vk": 0x30, "text": "Alt + 0"})
    # Screenshots
    screenshot_format: str = "png"  # png | jpg
    screenshot_sound: bool = True
    # Sound
    sound_enabled: bool = True
    sound_style: str = "chime"      # chime | pop | blip | custom
    sound_custom: str = ""
    sound_volume: int = 70
    toggle_sound: bool = True
    # Overlay
    overlay_enabled: bool = True
    overlay_mode: str = "always"    # always | auto
    overlay_corner: str = "bottom-right"
    overlay_scale: int = 100
    overlay_opacity: int = 92
    # General
    start_replay_on_launch: bool = True
    start_minimized: bool = False
    launch_at_startup: bool = False
    tray_notice_shown: bool = False

    # ------------------------------------------------------------------
    _lock = threading.Lock()

    @staticmethod
    def path() -> str:
        return os.path.join(data_dir(), "settings.json")

    @classmethod
    def load(cls) -> "Settings":
        s = cls()
        try:
            with open(cls.path(), "r", encoding="utf-8") as f:
                raw = json.load(f)
            for fdef in fields(cls):
                if fdef.name in raw:
                    val = raw[fdef.name]
                    cur = getattr(s, fdef.name)
                    if isinstance(cur, bool):
                        if isinstance(val, bool):
                            setattr(s, fdef.name, val)
                    elif isinstance(cur, int):
                        if isinstance(val, (int, float)) and not isinstance(val, bool):
                            setattr(s, fdef.name, int(val))
                    elif isinstance(cur, type(val)):
                        setattr(s, fdef.name, val)
        except FileNotFoundError:
            pass
        except Exception:
            pass
        s.clip_seconds = max(15, min(60, s.clip_seconds))
        s.bitrate_mbps = max(2, min(200, s.bitrate_mbps))
        # the previous version defaulted to Ctrl + 0; move anyone still on that to Alt + 0
        if s.hotkey_open.get("mods") == 2 and s.hotkey_open.get("vk") == 0x30:
            s.hotkey_open = {"mods": 1, "vk": 0x30, "text": "Alt + 0"}
        return s

    def save(self) -> None:
        with self._lock:
            tmp = self.path() + ".tmp"
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump(asdict(self), f, indent=2)
            os.replace(tmp, self.path())
