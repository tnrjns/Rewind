"""Trim a clip: pick a start and end, save the cut as a new file next to the original."""
import os
import tempfile
import threading

from PySide6.QtCore import Qt, QTimer, Signal
from PySide6.QtGui import QPixmap
from PySide6.QtWidgets import (QDialog, QHBoxLayout, QLabel, QMessageBox, QPushButton, QSlider, QVBoxLayout,
                               QWidget)

from .recorder import probe_duration, run


def _clock(sec):
    sec = max(0.0, sec)
    return f"{int(sec // 60)}:{sec % 60:04.1f}"


class _Frame(QWidget):
    """A still of the clip at one moment, plus its label."""
    loaded = Signal(str, str)

    def __init__(self, title):
        super().__init__()
        v = QVBoxLayout(self)
        v.setContentsMargins(0, 0, 0, 0)
        v.setSpacing(6)
        self.pic = QLabel()
        self.pic.setFixedSize(320, 180)
        self.pic.setAlignment(Qt.AlignCenter)
        self.pic.setStyleSheet("background:#080808; border:1px solid #353535; border-radius:8px; color:#8C8C8C;")
        self.cap = QLabel(title)
        self.cap.setObjectName("RowSub")
        v.addWidget(self.pic)
        v.addWidget(self.cap)
        self.loaded.connect(self._show)
        self._want = ""

    def load(self, ffmpeg, video, t, tmp):
        out = os.path.join(tmp, f"f_{int(t * 10)}.jpg")
        self._want = out
        if os.path.exists(out):
            self._show(out, out)
            return

        def work():
            run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-ss", f"{t:.2f}", "-i", video,
                 "-frames:v", "1", "-vf", "scale=640:-2", out], timeout=20)
            self.loaded.emit(out, out)

        threading.Thread(target=work, daemon=True).start()

    def _show(self, path, _):
        if path != self._want:
            return
        pm = QPixmap(path)
        if not pm.isNull():
            self.pic.setPixmap(pm.scaled(320, 180, Qt.KeepAspectRatio, Qt.SmoothTransformation))


class TrimDialog(QDialog):
    saved = Signal(str)

    def __init__(self, ffmpeg, video, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Trim clip")
        self.ffmpeg, self.video = ffmpeg, video
        self.dur = probe_duration(ffmpeg, video) or 0.0
        self.tmp = tempfile.mkdtemp(prefix="rewind-trim-")
        self.setStyleSheet("QDialog { background: #111111; }")
        v = QVBoxLayout(self)
        v.setContentsMargins(24, 22, 24, 20)
        v.setSpacing(14)
        title = QLabel(os.path.basename(video))
        title.setStyleSheet("font-size: 16px; font-weight: 700;")
        v.addWidget(title)
        frames = QHBoxLayout()
        frames.setSpacing(14)
        self.f_start, self.f_end = _Frame("Start"), _Frame("End")
        frames.addWidget(self.f_start)
        frames.addWidget(self.f_end)
        v.addLayout(frames)
        steps = max(1, int(self.dur * 10))
        self.s_start, self.s_end = QSlider(Qt.Horizontal), QSlider(Qt.Horizontal)
        for s, val in ((self.s_start, 0), (self.s_end, steps)):
            s.setRange(0, steps)
            s.setValue(val)
            s.valueChanged.connect(self._moved)
            s.sliderReleased.connect(self._frames)
        for label, s in (("Start", self.s_start), ("End", self.s_end)):
            row = QHBoxLayout()
            lab = QLabel(label)
            lab.setFixedWidth(44)
            lab.setObjectName("Muted")
            row.addWidget(lab)
            row.addWidget(s, 1)
            v.addLayout(row)
        self.info = QLabel()
        self.info.setObjectName("Muted")
        v.addWidget(self.info)
        btns = QHBoxLayout()
        btns.addStretch(1)
        cancel = QPushButton("Cancel")
        cancel.clicked.connect(self.reject)
        self.save = QPushButton("Save as new clip")
        self.save.setObjectName("Primary")
        self.save.clicked.connect(self._save)
        btns.addWidget(cancel)
        btns.addWidget(self.save)
        v.addLayout(btns)
        self._debounce = QTimer(self)
        self._debounce.setSingleShot(True)
        self._debounce.setInterval(180)
        self._debounce.timeout.connect(self._frames)
        self._moved()
        self._frames()

    def _span(self):
        a, b = self.s_start.value() / 10, self.s_end.value() / 10
        return min(a, b), max(a, b)

    def _moved(self):
        a, b = self._span()
        self.info.setText(f"{_clock(a)} to {_clock(b)}  ·  {b - a:.1f} s of {self.dur:.1f} s. "
                          "Cuts land on the nearest second.")
        self.save.setEnabled(b - a >= 0.5)
        self._debounce.start()

    def _frames(self):
        a, b = self._span()
        self.f_start.load(self.ffmpeg, self.video, a, self.tmp)
        self.f_end.load(self.ffmpeg, self.video, max(0.0, b - 0.05), self.tmp)

    def _target(self):
        base, ext = os.path.splitext(self.video)
        out, n = f"{base}_trim{ext}", 2
        while os.path.exists(out):
            out, n = f"{base}_trim{n}{ext}", n + 1
        return out

    def _save(self):
        a, b = self._span()
        out = self._target()
        self.save.setEnabled(False)
        self.save.setText("Saving…")
        r = run([self.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-ss", f"{a:.2f}", "-to", f"{b:.2f}",
                 "-i", self.video, "-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero",
                 "-movflags", "+faststart", out], timeout=120)
        if r.returncode != 0 or not os.path.exists(out) or os.path.getsize(out) == 0:
            if os.path.exists(out):
                os.remove(out)
            QMessageBox.warning(self, "Trim clip", "Couldn't save the trimmed clip.")
            self.save.setText("Save as new clip")
            self.save.setEnabled(True)
            return
        self.saved.emit(out)
        self.accept()

    def done(self, r):
        try:
            for f in os.listdir(self.tmp):
                os.remove(os.path.join(self.tmp, f))
            os.rmdir(self.tmp)
        except OSError:
            pass
        super().done(r)
