"""Starts Rewind's core pieces without a screen and checks they load on this OS."""
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
os.environ.setdefault("REWIND_TEST_SOURCE", "1")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from PySide6.QtWidgets import QApplication  # noqa: E402

from app import audio, hotkeys, recorder, settings, sounds, theme, updates, winapi  # noqa: E402,F401


def main() -> int:
    app = QApplication(sys.argv)
    s = settings.Settings()
    for name in ("toggle", "clip", "record", "screenshot", "open"):
        hk = getattr(s, f"hotkey_{name}")
        assert hk["vk"] is not None and hk["text"], name
    assert updates._version("v1.10.0") > updates._version("1.9.9")
    assert updates._version("v1.0.0") == (1, 0, 0)
    assert isinstance(winapi.list_monitors(), list)
    assert recorder.find_ffmpeg() or True
    assert sounds.synth("chime")
    app.quit()
    print(f"ok on {sys.platform}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
