"""Main window."""
import datetime
import os

from PySide6.QtCore import QEasingCurve, QMimeData, QPropertyAnimation, QRect, QSize, Qt, QTimer, QUrl
from PySide6.QtGui import QDesktopServices, QFont, QGuiApplication, QIcon, QPixmap
from PySide6.QtWidgets import (QApplication, QFileDialog, QFrame, QGraphicsOpacityEffect,
                               QHBoxLayout, QInputDialog, QLabel, QListView, QListWidget, QListWidgetItem,
                               QMainWindow, QMenu, QMessageBox, QProgressBar, QPushButton, QScrollArea,
                               QSpinBox, QStackedWidget, QVBoxLayout, QWidget)

from . import sounds, theme, winapi
from .recorder import ENCODERS
from .settings import APP_VERSION, IS_MAC, data_dir
from .widgets import (Card, ChipGroup, ClipCard, HotkeyEdit, LiveDot, SliderControl, ToggleSwitch, make_combo,
                      rounded_pixmap, set_combo)

NAV = [("home", "Home", ""), ("replay", "Replay", ""), ("recording", "Recording", "●"),
       ("hotkeys", "Hotkeys", ""), ("sound", "Sound", ""), ("overlay", "Overlay", ""),
       ("clips", "Clips", ""), ("general", "General", "")]


def fmt_size(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit in ("B", "KB") else f"{n:.1f} {unit}" if n < 10 else f"{n:.0f} {unit}"
        n /= 1024


def fmt_dur(sec):
    if sec is None:
        return "—"
    sec = int(round(sec))
    return f"{sec // 60}:{sec % 60:02d}"


def load_thumb(path, w=480):
    """Fast downscaled load (big 4K screenshots would be slow to decode at full size)."""
    from PySide6.QtGui import QImageReader
    r = QImageReader(path)
    r.setAutoTransform(True)
    sz = r.size()
    if sz.isValid() and sz.width() > w:
        r.setScaledSize(QSize(w, max(1, int(sz.height() * w / sz.width()))))
    img = r.read()
    return QPixmap.fromImage(img) if not img.isNull() else None


def clip_meta_text(c):
    if c.get("kind") == "image":
        return f"Screenshot  ·  {fmt_size(c['size'])}  ·  " \
               f"{datetime.datetime.fromtimestamp(c['mtime']).strftime('%b %d, %I:%M %p')}"
    return f"{fmt_dur(c['duration'])}  ·  {fmt_size(c['size'])}  ·  " \
           f"{datetime.datetime.fromtimestamp(c['mtime']).strftime('%b %d, %I:%M %p')}"


class Page(QScrollArea):
    def __init__(self, title, subtitle):
        super().__init__()
        self.setWidgetResizable(True)
        self.setFrameShape(QFrame.NoFrame)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        body = QWidget()
        body.setObjectName("PageBody")
        self.setWidget(body)
        self.v = QVBoxLayout(body)
        self.v.setContentsMargins(36, 30, 36, 30)
        self.v.setSpacing(16)
        t = QLabel(title)
        t.setObjectName("PageTitle")
        self.v.addWidget(t)
        if subtitle:
            s = QLabel(subtitle)
            s.setObjectName("PageSubtitle")
            s.setWordWrap(True)
            self.v.addWidget(s)
        self.v.addSpacing(6)

    def finish(self):
        self.v.addStretch(1)

    def refresh(self):
        pass


# =================================================================== Home
class HomePage(Page):
    def __init__(self, ctrl):
        super().__init__("Home", None)
        self.ctrl = ctrl

        hero = QFrame()
        hero.setObjectName("Hero")
        hv = QVBoxLayout(hero)
        hv.setContentsMargins(28, 24, 28, 24)
        hv.setSpacing(18)
        top = QHBoxLayout()
        col = QVBoxLayout()
        col.setSpacing(4)
        cap = QLabel("INSTANT REPLAY")
        cap.setObjectName("CardTitle")
        col.addWidget(cap)
        row = QHBoxLayout()
        row.setSpacing(6)
        self.dot = LiveDot(size=12)
        row.addWidget(self.dot, 0, Qt.AlignVCenter)
        self.state_lbl = QLabel("Off")
        self.state_lbl.setObjectName("HeroState")
        row.addWidget(self.state_lbl)
        row.addStretch(1)
        col.addLayout(row)
        self.detail = QLabel("")
        self.detail.setObjectName("RowSub")
        col.addWidget(self.detail)
        top.addLayout(col, 1)
        self.toggle = ToggleSwitch(large=True)
        self.toggle.toggled.connect(ctrl.set_replay)
        top.addWidget(self.toggle, 0, Qt.AlignVCenter)
        hv.addLayout(top)

        buf = QHBoxLayout()
        buf.setSpacing(12)
        bl = QLabel("Buffer")
        bl.setObjectName("Muted")
        self.buffer_title = bl
        self.bar = QProgressBar()
        self.bar.setObjectName("Buffer")
        self.bar.setRange(0, 1000)
        self.bar.setTextVisible(False)
        self.buf_lbl = QLabel("0:00 / 0:30")
        self.buf_lbl.setObjectName("Value")
        self.buf_lbl.setMinimumWidth(90)
        self.buf_lbl.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        buf.addWidget(bl)
        buf.addWidget(self.bar, 1)
        buf.addWidget(self.buf_lbl)
        hv.addLayout(buf)

        tiles = QHBoxLayout()
        tiles.setSpacing(10)
        self.tiles = {}
        for key, label in (("len", "CLIP LENGTH"), ("rate", "BITRATE"), ("enc", "ENCODER"), ("save", "CLIP HOTKEY")):
            t = QFrame()
            t.setObjectName("Tile")
            tv = QVBoxLayout(t)
            tv.setContentsMargins(14, 11, 14, 12)
            tv.setSpacing(3)
            a = QLabel(label)
            a.setObjectName("TileLabel")
            b = QLabel("—")
            b.setObjectName("TileValue")
            tv.addWidget(a)
            tv.addWidget(b)
            tiles.addWidget(t, 1)
            self.tiles[key] = b
        hv.addLayout(tiles)
        self.v.addWidget(hero)

        self.banner = QLabel("")
        self.banner.setObjectName("ErrorBanner")
        self.banner.setWordWrap(True)
        self.banner.hide()
        self.v.addWidget(self.banner)

        btns = QHBoxLayout()
        btns.setSpacing(10)
        self.save_btn = QPushButton("Save clip")
        self.save_btn.setObjectName("Primary")
        self.save_btn.setCursor(Qt.PointingHandCursor)
        self.save_btn.clicked.connect(ctrl.save_clip)
        self.record_btn = QPushButton("Start screen recording")
        self.record_btn.setCursor(Qt.PointingHandCursor)
        self.record_btn.clicked.connect(ctrl.toggle_recording)
        self.shot_btn = QPushButton("Screenshot")
        self.shot_btn.setCursor(Qt.PointingHandCursor)
        self.shot_btn.clicked.connect(ctrl.take_screenshot)
        folder = QPushButton("Open clips folder")
        folder.setCursor(Qt.PointingHandCursor)
        folder.clicked.connect(ctrl.open_clips_folder)
        btns.addWidget(self.save_btn)
        btns.addWidget(self.record_btn)
        btns.addWidget(self.shot_btn)
        btns.addWidget(folder)
        btns.addStretch(1)
        self.v.addLayout(btns)

        quick = Card("Quick settings")
        self.len_chips = ChipGroup([(15, "15s"), (30, "30s"), (45, "45s"), (60, "60s")])
        self.len_chips.selected.connect(lambda v: ctrl.set("clip_seconds", v))
        quick.add_row("Clip length", "How far back each clip reaches", self.len_chips)
        self.rate_chips = ChipGroup([(10, "10"), (25, "25"), (50, "50"), (100, "100 Mbps")])
        self.rate_chips.selected.connect(lambda v: ctrl.set("bitrate_mbps", v))
        quick.add_row("Bitrate", "Fine-tune anywhere from 5–150 Mbps on the Replay page", self.rate_chips)
        self.v.addWidget(quick)

        rc = Card("Recent clips & screenshots")
        self.recent_wrap = QWidget()
        self.recent = QHBoxLayout(self.recent_wrap)
        self.recent.setContentsMargins(0, 10, 0, 8)
        self.recent.setSpacing(12)
        rc.add_widget(self.recent_wrap)
        self.v.addWidget(rc)
        self.finish()

        self._timer = QTimer(self, interval=100)
        self._timer.timeout.connect(self._tick_buffer)
        self._timer.start()
        ctrl.state_changed.connect(self.on_state)
        ctrl.message.connect(self.on_message)
        ctrl.clips_changed.connect(self.load_recent)
        ctrl.settings_changed.connect(lambda _k: self.refresh())
        ctrl.recorder.encoders_detected.connect(lambda _e: self.refresh())
        self.on_state(ctrl.recorder.state)
        self.refresh()
        self.load_recent()

    def refresh(self):
        s = self.ctrl.settings
        self.len_chips.set_value(s.clip_seconds)
        self.rate_chips.set_value(s.bitrate_mbps)
        self.tiles["len"].setText(f"{s.clip_seconds} seconds")
        self.tiles["rate"].setText(f"{s.bitrate_mbps} Mbps")
        r = self.ctrl.recorder
        key = r.active_encoder if r.state == "on" else r.resolve_encoder()
        self.tiles["enc"].setText(ENCODERS.get(key, ENCODERS["x264"])["label"] +
                                  ("" if r.encoders is not None or r.state == "on" else "…"))
        self.tiles["save"].setText(s.hotkey_clip.get("text") or "Not set")
        self.save_btn.setText(f"Save last {s.clip_seconds}s")
        shk = s.hotkey_screenshot.get("text")
        self.shot_btn.setText(f"Screenshot  ({shk})" if shk else "Screenshot")
        self._update_detail()

    def _update_detail(self):
        r = self.ctrl.recorder
        s = self.ctrl.settings
        if r.state == "on":
            self.detail.setText("Screen recording  ·  " + r.pipeline if r.mode == "recording" else r.pipeline)
        elif r.state == "saving":
            self.detail.setText("Finishing the recording and adding its audio…")
        elif r.state == "starting":
            self.detail.setText("Starting screen recording…" if r.mode == "recording" else
                                "Warming up the encoder…")
        else:
            hk = s.hotkey_toggle.get("text") or "the switch"
            self.detail.setText(f"Flip the switch or press {hk} to start recording in the background")

    def on_state(self, st):
        r = self.ctrl.recorder
        replay_on = st in ("on", "starting") and r.mode == "replay"
        recording = r.mode == "recording"
        self.toggle.set_silent(replay_on)
        self.toggle.setEnabled(not recording and st != "saving")
        self.dot.set_state(st)
        label = "Recording" if recording and st == "on" else \
                "Starting…" if recording and st == "starting" else \
                {"on": "Replay On", "starting": "Starting…", "saving": "Saving…"}.get(st, "Off")
        self.state_lbl.setText(label)
        self.save_btn.setEnabled(st == "on" and r.mode == "replay")
        self.record_btn.setEnabled(st != "saving")
        if st == "saving":
            self.record_btn.setText("Saving recording…")
        elif recording:
            self.record_btn.setText("Stop recording")
        else:
            self.record_btn.setText("Start screen recording")
        self._update_detail()
        self.refresh()

    def on_message(self, msg):
        self.banner.setText(msg)
        self.banner.setVisible(bool(msg))

    def _tick_buffer(self):
        s = self.ctrl.settings
        r = self.ctrl.recorder
        recording = r.mode == "recording" and r.state == "on"
        self.buffer_title.setText("Elapsed" if recording else "Buffer")
        self.bar.setVisible(not recording)
        if recording:
            self.buf_lbl.setText(fmt_dur(r.buffered_seconds()))
            return
        have = min(r.buffered_seconds(), s.clip_seconds)
        self.bar.setValue(int(1000 * have / max(1, s.clip_seconds)))
        self.buf_lbl.setText(f"{fmt_dur(have)} / {fmt_dur(s.clip_seconds)}")

    def load_recent(self):
        while self.recent.count():
            it = self.recent.takeAt(0)
            w = it.widget()
            if w is not None:
                w.hide()
                w.setParent(None)
                w.deleteLater()
        clips = self.ctrl.list_clips(limit=3)
        if not clips:
            hk = self.ctrl.settings.hotkey_clip.get("text") or "Save clip"
            e = QLabel(f"No clips yet. Press {hk} while Instant Replay is on and your clip will show up here.")
            e.setObjectName("Muted")
            e.setWordWrap(True)
            self.recent.addWidget(e)
            return
        self.ctrl.request_thumbs(clips)
        for c in clips:
            img = c.get("kind") == "image"
            pm = load_thumb(c["thumb"]) if c["thumb"] else None
            name = os.path.splitext(os.path.basename(c["path"]))[0].replace("Rewind_", "").replace("Screenshot_", "")
            meta = f"Screenshot · {fmt_size(c['size'])}" if img else f"{fmt_dur(c['duration'])} · {fmt_size(c['size'])}"
            card = ClipCard(c["path"], pm, name, meta, w=160, play=not img)
            card.clicked.connect(winapi.open_path)
            self.recent.addWidget(card)
        self.recent.addStretch(1)



class RecordingPage(Page):
    def __init__(self, ctrl):
        super().__init__("Recording", "Screen capture with a live monitor preview.")
        self.ctrl = ctrl
        head = QHBoxLayout()
        self.status = QLabel("READY")
        self.status.setObjectName("CardTitle")
        head.addWidget(self.status)
        head.addStretch(1)
        self.elapsed = QLabel("00:00")
        self.elapsed.setObjectName("Value")
        head.addWidget(self.elapsed)
        self.v.addLayout(head)

        self.preview = QLabel("Waiting for screen preview…")
        self.preview.setObjectName("Preview")
        self.preview.setAlignment(Qt.AlignCenter)
        self.preview.setMinimumSize(480, 270)
        self.preview.setMaximumHeight(440)
        self.v.addWidget(self.preview, 1)

        controls = QHBoxLayout()
        self.record_btn = QPushButton("Start recording")
        self.record_btn.setObjectName("Primary")
        self.record_btn.setCursor(Qt.PointingHandCursor)
        self.record_btn.clicked.connect(ctrl.toggle_recording)
        controls.addWidget(self.record_btn)
        self.detail = QLabel("")
        self.detail.setObjectName("Muted")
        controls.addWidget(self.detail, 1)
        self.v.addLayout(controls)
        self.finish()

        self._timer = QTimer(self, interval=250)
        self._timer.timeout.connect(self._refresh_preview)
        ctrl.state_changed.connect(self._on_state)
        self._on_state(ctrl.recorder.state)

    def showEvent(self, event):
        super().showEvent(event)
        self._refresh_preview()
        self._timer.start()

    def hideEvent(self, event):
        self._timer.stop()
        super().hideEvent(event)

    def _refresh_preview(self):
        self._update_elapsed()
        monitors = self.ctrl.recorder.monitors
        monitor_index = self.ctrl.settings.monitor
        screens = QGuiApplication.screens()
        if not screens or not monitors:
            self.preview.setText("No display is available for preview.")
            return
        index = min(max(monitor_index, 0), min(len(screens), len(monitors)) - 1)
        pixmap = screens[index].grabWindow(0)
        if pixmap.isNull():
            self.preview.setText("Screen preview is unavailable.")
            return
        self.preview.setPixmap(pixmap.scaled(self.preview.size(), Qt.KeepAspectRatio, Qt.SmoothTransformation))

    def _on_state(self, state):
        recorder = self.ctrl.recorder
        recording = recorder.mode == "recording"
        self.status.setText("RECORDING" if recording and state == "on" else
                            "STARTING" if recording and state == "starting" else
                            "SAVING" if state == "saving" else "READY")
        self.record_btn.setEnabled(state != "saving")
        self.record_btn.setText("Stop recording" if recording else "Start recording")
        self.detail.setText(recorder.pipeline if recording else "Preview follows the selected monitor in Replay settings.")

    def _update_elapsed(self):
        recorder = self.ctrl.recorder
        elapsed = max(0, int(recorder.buffered_seconds())) if recorder.mode == "recording" else 0
        self.elapsed.setText(f"{elapsed // 60:02d}:{elapsed % 60:02d}")


# =================================================================== Replay
class ReplayPage(Page):
    def __init__(self, ctrl):
        super().__init__("Replay", "How much gets kept and how good it looks. Changes apply instantly "
                                   "(the buffer restarts if the capture settings change).")
        self.ctrl = ctrl
        s = ctrl.settings
        self.capture_note = QLabel("Full recording is active. Encoder, quality, monitor and audio-source changes "
                                   "apply after it stops; the save folder applies when the file is saved.")
        self.capture_note.setObjectName("RowSub")
        self.capture_note.setWordWrap(True)
        self.v.addWidget(self.capture_note)
        ctrl.state_changed.connect(self._refresh_capture_note)
        self._refresh_capture_note(ctrl.recorder.state)
        c1 = Card("Clip")
        self.length = SliderControl(15, 60, 5, s.clip_seconds, lambda v: f"{v} seconds")
        self.length.valueChanged.connect(lambda v: ctrl.set("clip_seconds", v))
        c1.add_row("Replay length", "Save anywhere from the last 15 seconds up to a full minute", self.length)
        self.v.addWidget(c1)

        c2 = Card("Quality")
        self.rate = SliderControl(5, 150, 5, s.bitrate_mbps, lambda v: f"{v} Mbps")
        self.rate.valueChanged.connect(lambda v: ctrl.set("bitrate_mbps", v))
        self.rate_row = c2.add_row("Bitrate", "", self.rate)
        self.fps = make_combo([("30 fps", 30), ("60 fps", 60), ("120 fps", 120), ("144 fps", 144)], s.fps)
        self.fps.currentIndexChanged.connect(lambda _i: ctrl.set("fps", self.fps.currentData()))
        c2.add_row("Frame rate", "Higher frame rates need more bitrate to stay sharp", self.fps)
        self.res = make_combo([("Native", "native"), ("1440p", "1440"), ("1080p", "1080"), ("720p", "720")],
                              s.resolution)
        self.res.currentIndexChanged.connect(lambda _i: ctrl.set("resolution", self.res.currentData()))
        c2.add_row("Resolution", "Downscaling uses a little extra CPU", self.res)
        self.enc = make_combo([("Automatic (best available)", "auto")], s.encoder)
        self.enc.currentIndexChanged.connect(lambda _i: ctrl.set("encoder", self.enc.currentData()))
        self.enc_row = c2.add_row("Encoder", "GPU encoders have almost no impact on game performance", self.enc)
        self.codec = make_combo([("H.264 (plays everywhere)", "h264"), ("HEVC / H.265 (smaller files)", "hevc")],
                                s.codec)
        self.codec.currentIndexChanged.connect(lambda _i: ctrl.set("codec", self.codec.currentData()))
        c2.add_row("Codec", None, self.codec)
        self.v.addWidget(c2)

        c3 = Card("Capture")
        mons = ctrl.recorder.monitors
        self.mon = make_combo([(f"{m['name']}  ·  {m['w']}×{m['h']}" + ("  (main)" if m.get("primary") else ""), i)
                               for i, m in enumerate(mons)], s.monitor)
        self.mon.currentIndexChanged.connect(lambda _i: ctrl.set("monitor", self.mon.currentData()))
        c3.add_row("Monitor", None, self.mon)
        self.cursor = ToggleSwitch()
        self.cursor.set_silent(s.capture_cursor)
        self.cursor.toggled.connect(lambda v: ctrl.set("capture_cursor", v))
        c3.add_row("Show mouse cursor", None, self.cursor)
        self.v.addWidget(c3)

        c4 = Card("Audio")
        self.sys_audio = ToggleSwitch()
        self.sys_audio.set_silent(s.system_audio)
        self.sys_audio.toggled.connect(lambda v: ctrl.set("system_audio", v))
        c4.add_row("Desktop audio", "Game and everything else you hear", self.sys_audio)
        self.sys_vol = SliderControl(0, 200, 5, s.system_volume, lambda v: f"{v}%", width=180)
        self.sys_vol.valueChanged.connect(lambda v: ctrl.set("system_volume", v))
        c4.add_row("Desktop volume", None, self.sys_vol)
        self.mic = ToggleSwitch()
        self.mic.set_silent(s.mic_enabled)
        self.mic.toggled.connect(lambda v: ctrl.set("mic_enabled", v))
        c4.add_row("Microphone", "Mix your voice into clips", self.mic)
        mics = ctrl.microphones()
        self.mic_dev = make_combo([("Default microphone", "")] + [(m, m) for m in mics], s.mic_device)
        self.mic_dev.currentIndexChanged.connect(lambda _i: ctrl.set("mic_device", self.mic_dev.currentData()))
        c4.add_row("Mic device", None, self.mic_dev)
        self.mic_vol = SliderControl(0, 200, 5, s.mic_volume, lambda v: f"{v}%", width=180)
        self.mic_vol.valueChanged.connect(lambda v: ctrl.set("mic_volume", v))
        c4.add_row("Mic volume", None, self.mic_vol)
        self.offset = QSpinBox()
        self.offset.setRange(-1000, 1000)
        self.offset.setSingleStep(10)
        self.offset.setSuffix(" ms")
        self.offset.setValue(s.audio_offset_ms)
        self.offset.setFixedWidth(110)
        self.offset.valueChanged.connect(lambda v: ctrl.set("audio_offset_ms", v))
        c4.add_row("Audio sync offset", "Only touch this if sound is early (+) or late (−) in your clips",
                   self.offset)
        self.v.addWidget(c4)

        c5 = Card("Saving")
        w = QWidget()
        h = QHBoxLayout(w)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(8)
        change = QPushButton("Change…")
        change.clicked.connect(self._pick_folder)
        opn = QPushButton("Open")
        opn.clicked.connect(ctrl.open_clips_folder)
        h.addWidget(change)
        h.addWidget(opn)
        self.folder_row = c5.add_row("Clips folder", s.output_dir, w)
        self.shot_fmt = make_combo([("PNG (lossless)", "png"), ("JPG (smaller files)", "jpg")], s.screenshot_format)
        self.shot_fmt.currentIndexChanged.connect(lambda _i: ctrl.set("screenshot_format", self.shot_fmt.currentData()))
        c5.add_row("Screenshot format", "Screenshots are saved in the same folder as your clips", self.shot_fmt)
        self.v.addWidget(c5)
        self.finish()

        ctrl.recorder.encoders_detected.connect(self._fill_encoders)
        if ctrl.recorder.encoders is not None:
            self._fill_encoders(ctrl.recorder.encoders)
        ctrl.settings_changed.connect(lambda _k: self.refresh())
        self.refresh()

    def _fill_encoders(self, encs):
        cur = self.ctrl.settings.encoder
        self.enc.blockSignals(True)
        self.enc.clear()
        self.enc.addItem("Automatic (best available)", "auto")
        for k in encs:
            self.enc.addItem(ENCODERS[k]["label"], k)
        self.enc.blockSignals(False)
        set_combo(self.enc, cur if cur in encs or cur == "auto" else "auto")
        found = ", ".join(ENCODERS[k]["label"] for k in encs if k != "x264") or "none (using CPU)"
        self.enc_row.sub.setText(f"Hardware encoders found: {found}")

    def _pick_folder(self):
        d = QFileDialog.getExistingDirectory(self, "Choose where clips are saved", self.ctrl.settings.output_dir)
        if d:
            self.ctrl.set("output_dir", os.path.normpath(d))

    def refresh(self):
        s = self.ctrl.settings
        self.length.set_value(s.clip_seconds)
        self.rate.set_value(s.bitrate_mbps)
        mb = s.bitrate_mbps * s.clip_seconds / 8
        self.rate_row.set_sub(f"≈ {mb:.0f} MB per {s.clip_seconds}s clip. 25–50 Mbps looks great at 1080p60; "
                              f"use 80+ for 1440p/4K")
        self.folder_row.set_sub(s.output_dir)
        for w in (self.mic_dev, self.mic_vol):
            w.setEnabled(s.mic_enabled)
        self.sys_vol.setEnabled(s.system_audio)

    def _refresh_capture_note(self, _state):
        self.capture_note.setVisible(self.ctrl.recorder.mode == "recording")


# =================================================================== Hotkeys
class HotkeysPage(Page):
    def __init__(self, ctrl):
        super().__init__("Hotkeys", "These work everywhere, even while you're in a game. "
                                    "Click a box, then press the combo you want.")
        self.ctrl = ctrl
        s = ctrl.settings
        c = Card("Shortcuts")
        self.toggle = HotkeyEdit(s.hotkey_toggle)
        self.toggle.changed.connect(lambda hk: ctrl.set("hotkey_toggle", hk))
        self.toggle.capturing_changed.connect(lambda on: ctrl.register_hotkeys(paused=True) if on else
                                              ctrl.register_hotkeys())
        self.row_toggle = c.add_row("Turn Instant Replay on / off", None, self.toggle)
        self.clip = HotkeyEdit(s.hotkey_clip)
        self.clip.changed.connect(lambda hk: ctrl.set("hotkey_clip", hk))
        self.clip.capturing_changed.connect(lambda on: ctrl.register_hotkeys(paused=True) if on else
                                            ctrl.register_hotkeys())
        self.row_clip = c.add_row("Save clip", None, self.clip)
        self.record = HotkeyEdit(s.hotkey_record)
        self.record.changed.connect(lambda hk: ctrl.set("hotkey_record", hk))
        self.record.capturing_changed.connect(lambda on: ctrl.register_hotkeys(paused=True) if on else
                                              ctrl.register_hotkeys())
        self.row_record = c.add_row("Start / stop screen recording", None, self.record)
        self.shot = HotkeyEdit(s.hotkey_screenshot)
        self.shot.changed.connect(lambda hk: ctrl.set("hotkey_screenshot", hk))
        self.shot.capturing_changed.connect(lambda on: ctrl.register_hotkeys(paused=True) if on else
                                            ctrl.register_hotkeys())
        self.row_shot = c.add_row("Take screenshot", None, self.shot)
        self.open = HotkeyEdit(s.hotkey_open)
        self.open.changed.connect(lambda hk: ctrl.set("hotkey_open", hk))
        self.open.capturing_changed.connect(lambda on: ctrl.register_hotkeys(paused=True) if on else
                                            ctrl.register_hotkeys())
        self.row_open = c.add_row("Open Rewind", None, self.open)
        self.v.addWidget(c)
        tip = QLabel("Tip: if a combo shows “in use”, another app or macOS already owns it. Combos need ⌃ or ⌘; "
                     "macOS blocks ones that only use ⌥ and ⇧." if IS_MAC else
                     "Tip: if a combo shows “in use”, another app (often the NVIDIA overlay, Discord or OBS) "
                     "already owns it. Pick a different one, or turn it off in that app.")
        tip.setObjectName("Muted")
        tip.setWordWrap(True)
        self.v.addWidget(tip)
        self.finish()
        ctrl.hotkeys_changed.connect(self.on_status)
        self.on_status(ctrl.hotkey_status)

    def on_status(self, st):
        for key, row in (("toggle", self.row_toggle), ("clip", self.row_clip), ("record", self.row_record),
                         ("shot", self.row_shot),
                         ("open", self.row_open)):
            ok = st.get(key)
            if ok is None:
                txt = "Not set"
            elif ok:
                txt = "Active"
            elif key in self.ctrl.hotkey_dupes:
                txt = "⚠ Same combo as another Rewind hotkey — pick another"
            else:
                txt = "⚠ In use by another app — pick another combo"
            row.sub.setText(txt)
            row.sub.setObjectName("RowSub" if ok else "Warn")
            row.sub.style().unpolish(row.sub)
            row.sub.style().polish(row.sub)
            row.sub.show()


# =================================================================== Sound
class SoundPage(Page):
    def __init__(self, ctrl):
        super().__init__("Sound", "A little confirmation so you know the clip landed, without alt-tabbing.")
        self.ctrl = ctrl
        s = ctrl.settings
        c = Card("Clip sound")
        self.enabled = ToggleSwitch()
        self.enabled.set_silent(s.sound_enabled)
        self.enabled.toggled.connect(lambda v: ctrl.set("sound_enabled", v))
        c.add_row("Play a sound when a clip is saved", None, self.enabled)
        self.style = make_combo([(label, key) for key, label in sounds.STYLES], s.sound_style)
        self.style.currentIndexChanged.connect(self._style_changed)
        self.style_row = c.add_row("Sound", None, self.style)
        self.vol = SliderControl(5, 100, 5, s.sound_volume, lambda v: f"{v}%", width=200)
        self.vol.valueChanged.connect(lambda v: ctrl.set("sound_volume", v))
        self.vol.slider.sliderReleased.connect(ctrl.test_sound)
        c.add_row("Volume", None, self.vol)
        test = QPushButton("Play test sound")
        test.setCursor(Qt.PointingHandCursor)
        test.clicked.connect(ctrl.test_sound)
        c.add_row("Preview", None, test)
        self.v.addWidget(c)

        c2 = Card("Other sounds")
        self.toggle_snd = ToggleSwitch()
        self.toggle_snd.set_silent(s.toggle_sound)
        self.toggle_snd.toggled.connect(lambda v: ctrl.set("toggle_sound", v))
        c2.add_row("Play a sound when Instant Replay turns on or off", None, self.toggle_snd)
        self.shot_snd = ToggleSwitch()
        self.shot_snd.set_silent(s.screenshot_sound)
        self.shot_snd.toggled.connect(lambda v: ctrl.set("screenshot_sound", v))
        c2.add_row("Play a camera-shutter sound for screenshots", None, self.shot_snd)
        self.v.addWidget(c2)
        self.finish()
        self._update_sub()

    def _style_changed(self, _i):
        key = self.style.currentData()
        if key == "custom":
            path, _ = QFileDialog.getOpenFileName(self, "Pick a sound", "", "WAV audio (*.wav)")
            if path:
                self.ctrl.set("sound_custom", path)
            elif not self.ctrl.settings.sound_custom:
                set_combo(self.style, self.ctrl.settings.sound_style)
                return
        self.ctrl.set("sound_style", key)
        self._update_sub()
        self.ctrl.test_sound()

    def _update_sub(self):
        s = self.ctrl.settings
        if s.sound_style == "custom" and s.sound_custom:
            self.style_row.sub.setText(os.path.basename(s.sound_custom))
            self.style_row.sub.show()
        else:
            self.style_row.sub.hide()


# =================================================================== Overlay
class OverlayPage(Page):
    def __init__(self, ctrl):
        super().__init__("Overlay", "The status icon in the corner of your screen. It never shows up in your "
                                    "clips and clicks pass straight through it.")
        self.ctrl = ctrl
        s = ctrl.settings
        c = Card("Status icon")
        self.enabled = ToggleSwitch()
        self.enabled.set_silent(s.overlay_enabled)
        self.enabled.toggled.connect(lambda v: ctrl.set("overlay_enabled", v))
        c.add_row("Show status icon", "Shows whether Instant Replay is on or off", self.enabled)
        self.mode = make_combo([("Always visible", "always"), ("Only when something changes", "auto")],
                               s.overlay_mode)
        self.mode.currentIndexChanged.connect(lambda _i: ctrl.set("overlay_mode", self.mode.currentData()))
        c.add_row("Visibility", "“Only when something changes” pops up on toggle/clip, then fades away", self.mode)
        self.corner = make_combo([("Bottom right", "bottom-right"), ("Bottom left", "bottom-left"),
                                  ("Top right", "top-right"), ("Top left", "top-left")], s.overlay_corner)
        self.corner.currentIndexChanged.connect(lambda _i: ctrl.set("overlay_corner", self.corner.currentData()))
        c.add_row("Position", None, self.corner)
        self.scale = SliderControl(75, 150, 5, s.overlay_scale, lambda v: f"{v}%", width=200)
        self.scale.valueChanged.connect(lambda v: ctrl.set("overlay_scale", v))
        c.add_row("Size", None, self.scale)
        self.opacity = SliderControl(30, 100, 5, s.overlay_opacity, lambda v: f"{v}%", width=200)
        self.opacity.valueChanged.connect(lambda v: ctrl.set("overlay_opacity", v))
        c.add_row("Opacity", None, self.opacity)
        prev = QPushButton("Preview")
        prev.setCursor(Qt.PointingHandCursor)
        prev.clicked.connect(ctrl.preview_overlay)
        c.add_row("Test it", "Shows the “clip saved” animation", prev)
        self.v.addWidget(c)
        note = QLabel("Works over windowed and borderless-fullscreen games. Exclusive-fullscreen games draw over "
                      "everything, so switch those to borderless if you want to see the icon.")
        note.setObjectName("Muted")
        note.setWordWrap(True)
        self.v.addWidget(note)
        self.finish()


# =================================================================== Clips
class ClipsPage(Page):
    def __init__(self, ctrl):
        super().__init__("Clips", None)
        self.ctrl = ctrl
        self.filter = "all"
        bar = QHBoxLayout()
        self.filters = ChipGroup([("all", "All"), ("video", "Clips"), ("image", "Screenshots")])
        self.filters.set_value("all")
        self.filters.selected.connect(self._set_filter)
        bar.addWidget(self.filters)
        bar.addStretch(1)
        refresh = QPushButton("Refresh")
        refresh.clicked.connect(self.refresh)
        folder = QPushButton("Open folder")
        folder.clicked.connect(ctrl.open_clips_folder)
        bar.addWidget(refresh)
        bar.addWidget(folder)
        self.v.addLayout(bar)
        self.grid = QListWidget()
        self.grid.setObjectName("ClipGrid")
        self.grid.setViewMode(QListView.IconMode)
        self.grid.setIconSize(QSize(240, 135))
        self.grid.setGridSize(QSize(264, 205))
        self.grid.setResizeMode(QListView.Adjust)
        self.grid.setMovement(QListView.Static)
        self.grid.setWordWrap(True)
        self.grid.setUniformItemSizes(True)
        self.grid.setSelectionMode(QListWidget.ExtendedSelection)
        self.grid.setVerticalScrollMode(QListView.ScrollPerPixel)
        self.grid.setContextMenuPolicy(Qt.CustomContextMenu)
        self.grid.customContextMenuRequested.connect(self._menu)
        self.grid.itemDoubleClicked.connect(lambda it: winapi.open_path(it.data(Qt.UserRole)))
        self.grid.setMinimumHeight(420)
        self.v.addWidget(self.grid, 1)
        self.count = QLabel("")
        self.count.setObjectName("Muted")
        self.v.addWidget(self.count)
        hint = QLabel("Double-click to open · Right-click to copy (paste straight into Discord), rename or delete")
        hint.setObjectName("Muted")
        self.v.addWidget(hint)
        ctrl.clips_changed.connect(self.refresh)
        self._icons = {}
        self.refresh()

    def _set_filter(self, v):
        self.filter = v
        self.refresh()

    def _icon(self, c):
        key = (c["path"], c["thumb"], c["mtime"])
        if key not in self._icons:
            pm = load_thumb(c["thumb"]) if c["thumb"] else None
            self._icons[key] = QIcon(rounded_pixmap(pm, 240, 135, 10, play=c.get("kind") != "image"))
        return self._icons[key]

    def refresh(self):
        clips = self.ctrl.list_clips()
        self.ctrl.request_thumbs(clips)
        if self.filter != "all":
            clips = [c for c in clips if c.get("kind") == self.filter]
        self.grid.clear()
        for c in clips:
            name = os.path.splitext(os.path.basename(c["path"]))[0]
            it = QListWidgetItem(self._icon(c), f"{name}\n{clip_meta_text(c)}")
            it.setData(Qt.UserRole, c["path"])
            it.setToolTip(c["path"])
            self.grid.addItem(it)
        total = sum(c["size"] for c in clips)
        self.count.setText(f"{len(clips)} item{'s' if len(clips) != 1 else ''}  ·  {fmt_size(total)}  ·  "
                           f"{self.ctrl.settings.output_dir}")

    def _selected(self):
        return [it.data(Qt.UserRole) for it in self.grid.selectedItems()]

    def _menu(self, pos):
        it = self.grid.itemAt(pos)
        if it is None:
            return
        if not it.isSelected():
            self.grid.clearSelection()
            it.setSelected(True)
        paths = self._selected()
        m = QMenu(self)
        m.addAction("Open", lambda: winapi.open_path(paths[0]))
        m.addAction("Show in folder", lambda: winapi.reveal_in_explorer(paths[0]))
        m.addAction("Copy (paste into Discord etc.)", lambda: self._copy(paths))
        if len(paths) == 1:
            m.addAction("Rename…", lambda: self._rename(paths[0]))
        m.addSeparator()
        m.addAction(f"Delete{'' if len(paths) == 1 else f' {len(paths)} items'}", lambda: self._delete(paths))
        m.exec(self.grid.viewport().mapToGlobal(pos))

    def _copy(self, paths):
        md = QMimeData()
        md.setUrls([QUrl.fromLocalFile(p) for p in paths])
        QApplication.clipboard().setMimeData(md)

    def _rename(self, path):
        old = os.path.splitext(os.path.basename(path))
        name, ok = QInputDialog.getText(self, "Rename clip", "New name:", text=old[0])
        name = (name or "").strip().strip(".")
        if ok and name and name != old[0]:
            bad = '<>:"/\\|?*'
            name = "".join("_" if ch in bad else ch for ch in name)
            new = os.path.join(os.path.dirname(path), name + old[1])
            if os.path.exists(new):
                QMessageBox.warning(self, "Rename clip", "A clip with that name already exists.")
                return
            try:
                os.rename(path, new)
                from .recorder import load_meta, thumb_path, update_meta
                meta = load_meta().get(os.path.basename(path), {})
                if meta:
                    update_meta(os.path.basename(new), **meta)
                if os.path.exists(thumb_path(path)):
                    os.replace(thumb_path(path), thumb_path(new))
            except OSError as e:
                QMessageBox.warning(self, "Rename clip", str(e))
            self.ctrl.clips_changed.emit()

    def _delete(self, paths):
        from PySide6.QtCore import QFile
        what = "this file" if len(paths) == 1 else f"these {len(paths)} files"
        if QMessageBox.question(self, "Delete", f"Move {what} to the Recycle Bin?") != QMessageBox.Yes:
            return
        for p in paths:
            if not QFile.moveToTrash(p):
                try:
                    os.remove(p)
                except OSError:
                    pass
        self.ctrl.clips_changed.emit()


# =================================================================== General
class GeneralPage(Page):
    def _make_shortcut(self):
        ok = winapi.create_shortcuts()
        self.sc_row.set_sub("✓ Shortcut created on your Desktop and in the Start menu" if ok else
                            "Couldn't create the shortcut. Try running Create Shortcut.bat in the Rewind folder")

    def _update_clicked(self):
        u = self.ctrl.updates
        if u.status == "newer" and u.url:
            QDesktopServices.openUrl(QUrl(u.url))
            return
        self.upd_btn.setEnabled(False)
        self.upd_row.set_sub("Checking…")
        u.check()

    def _update_checked(self, status, version, _url):
        self.upd_btn.setEnabled(True)
        if status == "newer":
            self.upd_btn.setText(f"Download {version}")
            self.upd_row.set_sub(f"Rewind {version} is out. You have {APP_VERSION}.")
        elif status == "current":
            self.upd_btn.setText("Check for updates")
            self.upd_row.set_sub(f"You're on the newest version ({APP_VERSION}).")
        else:
            self.upd_btn.setText("Check for updates")
            self.upd_row.set_sub("Couldn't reach GitHub. Check your connection and try again.")

    def __init__(self, ctrl):
        super().__init__("General", None)
        self.ctrl = ctrl
        s = ctrl.settings
        c = Card("Startup")
        self.auto_on = ToggleSwitch()
        self.auto_on.set_silent(s.start_replay_on_launch)
        self.auto_on.toggled.connect(lambda v: ctrl.set("start_replay_on_launch", v))
        c.add_row("Turn on Instant Replay when Rewind opens", None, self.auto_on)
        self.boot = ToggleSwitch()
        self.boot.set_silent(s.launch_at_startup)
        self.boot.toggled.connect(lambda v: ctrl.set("launch_at_startup", v))
        c.add_row("Open at login" if IS_MAC else "Launch when Windows starts",
                  "Starts quietly in the menu bar" if IS_MAC else "Starts quietly in the system tray", self.boot)
        self.mini = ToggleSwitch()
        self.mini.set_silent(s.start_minimized)
        self.mini.toggled.connect(lambda v: ctrl.set("start_minimized", v))
        c.add_row("Always start minimized to the " + ("menu bar" if IS_MAC else "tray"), None, self.mini)
        self.v.addWidget(c)

        c2 = Card("About")
        ff = ctrl.recorder.ffmpeg
        c2.add_row(f"Rewind {APP_VERSION}", "Instant replay for macOS" if IS_MAC else "Instant replay for Windows 11", None)
        c2.add_row("FFmpeg", ff or ("Not found. Install it with: brew install ffmpeg" if IS_MAC
                                     else "Not found. Run install.bat"), None)
        self.upd_btn = QPushButton("Check for updates")
        self.upd_btn.setCursor(Qt.PointingHandCursor)
        self.upd_btn.clicked.connect(self._update_clicked)
        self.upd_row = c2.add_row("Updates", "Rewind checks for a new version when it opens", self.upd_btn)
        ctrl.updates.checked.connect(self._update_checked)
        if ctrl.updates.status:
            self._update_checked(ctrl.updates.status, ctrl.updates.version, ctrl.updates.url)
        logs = QPushButton("Open data folder")
        logs.clicked.connect(lambda: winapi.open_path(data_dir()))
        c2.add_row("Logs & settings", data_dir(), logs)
        if not IS_MAC:
            sc = QPushButton("Create shortcut")
            sc.setCursor(Qt.PointingHandCursor)
            sc.clicked.connect(self._make_shortcut)
            self.sc_row = c2.add_row("Desktop shortcut",
                                     "Adds Rewind to your Desktop and Start menu", sc)
        quit_btn = QPushButton("Quit Rewind")
        quit_btn.clicked.connect(ctrl.quit)
        c2.add_row("Close completely", "Closing the window keeps Rewind running in the " +
                   ("menu bar" if IS_MAC else "tray"), quit_btn)
        self.v.addWidget(c2)
        self.finish()


# =================================================================== Window
class MainWindow(QMainWindow):
    def __init__(self, ctrl):
        super().__init__()
        self.ctrl = ctrl
        self.setWindowTitle("Rewind")
        self.setWindowIcon(theme.app_icon())
        self.resize(1040, 720)
        self.setMinimumSize(900, 600)

        root = QWidget()
        root.setObjectName("Root")
        self.setCentralWidget(root)
        h = QHBoxLayout(root)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(0)

        side = QFrame()
        side.setObjectName("Sidebar")
        side.setFixedWidth(218)
        self.side = side
        sv = QVBoxLayout(side)
        sv.setContentsMargins(14, 20, 14, 16)
        sv.setSpacing(3)
        brand = QHBoxLayout()
        brand.setContentsMargins(6, 0, 0, 0)
        brand.setSpacing(10)
        logo = QLabel()
        logo_pm = theme.draw_logo(64)
        logo_pm.setDevicePixelRatio(2)
        logo.setPixmap(logo_pm)
        logo.setFixedSize(32, 32)
        brand.addWidget(logo)
        bcol = QVBoxLayout()
        bcol.setSpacing(0)
        bt = QLabel("Rewind")
        bt.setObjectName("Brand")
        bs = QLabel("Instant Replay")
        bs.setObjectName("BrandSub")
        bcol.addWidget(bt)
        bcol.addWidget(bs)
        brand.addLayout(bcol)
        brand.addStretch(1)
        sv.addLayout(brand)
        sv.addSpacing(22)

        icon_font = theme.icon_font_family()
        self.nav = []
        for i, (key, label, glyph) in enumerate(NAV):
            b = QPushButton(f"{glyph}   {label}" if icon_font else label)
            b.setObjectName("NavButton")
            b.setCheckable(True)
            b.setCursor(Qt.PointingHandCursor)
            if icon_font:
                f = QFont()
                f.setFamilies([icon_font] + theme.ui_font_families())
                f.setPixelSize(13)
                b.setFont(f)
            b.clicked.connect(lambda _=False, idx=i: self.go(idx))
            sv.addWidget(b)
            self.nav.append(b)
        sv.addStretch(1)

        st = QHBoxLayout()
        st.setContentsMargins(8, 0, 0, 0)
        st.setSpacing(4)
        self.side_dot = LiveDot(size=8)
        self.side_status = QLabel("Replay off")
        self.side_status.setObjectName("SideStatus")
        st.addWidget(self.side_dot)
        st.addWidget(self.side_status)
        st.addStretch(1)
        sv.addLayout(st)

        self.indicator = QFrame(side)
        self.indicator.setStyleSheet(f"background: qlineargradient(x1:0,y1:0,x2:0,y2:1, stop:0 {theme.ACCENT},"
                                     f" stop:1 {theme.ACCENT_2}); border-radius: 2px;")
        self.indicator.resize(3, 18)
        self._ind_anim = QPropertyAnimation(self.indicator, b"geometry", self)
        self._ind_anim.setDuration(260)
        self._ind_anim.setEasingCurve(QEasingCurve.OutCubic)

        h.addWidget(side)
        self.stack = QStackedWidget()
        h.addWidget(self.stack, 1)
        self.pages = [HomePage(ctrl), ReplayPage(ctrl), RecordingPage(ctrl), HotkeysPage(ctrl), SoundPage(ctrl),
                      OverlayPage(ctrl), ClipsPage(ctrl), GeneralPage(ctrl)]
        for p in self.pages:
            self.stack.addWidget(p)
        self.nav[0].setChecked(True)
        self._page_anim = None
        ctrl.state_changed.connect(self._on_state)
        self._on_state(ctrl.recorder.state)

    def _on_state(self, st):
        self.side_dot.set_state(st)
        r = self.ctrl.recorder
        if r.mode == "recording":
            label = "Starting recording…" if st == "starting" else "Recording"
        else:
            label = {"on": "Replay on", "starting": "Starting…", "saving": "Saving…"}.get(st, "Replay off")
        self.side_status.setText(label)

    def _indicator_rect(self, idx):
        g = self.nav[idx].geometry()
        return QRect(g.x() - 8, g.y() + (g.height() - 18) // 2, 3, 18)

    def go(self, idx):
        for i, b in enumerate(self.nav):
            b.setChecked(i == idx)
        self._ind_anim.stop()
        self._ind_anim.setStartValue(self.indicator.geometry())
        self._ind_anim.setEndValue(self._indicator_rect(idx))
        self._ind_anim.start()
        if idx == self.stack.currentIndex():
            return
        page = self.pages[idx]
        self.stack.setCurrentIndex(idx)
        eff = QGraphicsOpacityEffect(page)
        page.setGraphicsEffect(eff)
        anim = QPropertyAnimation(eff, b"opacity", self)
        anim.setDuration(200)
        anim.setStartValue(0.0)
        anim.setEndValue(1.0)
        anim.setEasingCurve(QEasingCurve.OutCubic)
        anim.finished.connect(lambda: page.setGraphicsEffect(None))
        anim.start()
        self._page_anim = anim

    def showEvent(self, e):
        super().showEvent(e)
        winapi.style_titlebar(int(self.winId()), (0x11, 0x11, 0x11))
        QTimer.singleShot(0, lambda: self.indicator.setGeometry(self._indicator_rect(self.stack.currentIndex())))

    def resizeEvent(self, e):
        super().resizeEvent(e)
        if self._ind_anim.state() != QPropertyAnimation.Running:
            self.indicator.setGeometry(self._indicator_rect(self.stack.currentIndex()))

    def closeEvent(self, e):
        e.ignore()
        self.hide()
        self.ctrl.window_closed()
