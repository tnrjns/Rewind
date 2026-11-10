"""First-run tutorial: a short, hands-on walkthrough shown over the main window."""
from PySide6.QtCore import QEasingCurve, QPropertyAnimation, Qt, QTimer
from PySide6.QtGui import QColor, QPainter
from PySide6.QtWidgets import (QGraphicsOpacityEffect, QHBoxLayout, QLabel, QPushButton, QVBoxLayout,
                               QWidget, QFrame)

from . import theme, winapi
from .settings import IS_MAC

TRAY = "menu bar" if IS_MAC else "tray"


class Tutorial(QWidget):
    """Dimmed layer over the window with one card per step. Try-it steps finish when the real action happens."""

    def __init__(self, window, ctrl):
        super().__init__(window.centralWidget())
        self.ctrl = ctrl
        self.window = window
        self.i = 0
        self.done_steps: set = set()
        s = ctrl.settings
        hk = lambda name: getattr(s, f"hotkey_{name}").get("text") or "not set"  # noqa: E731
        self.steps = []
        if IS_MAC and not winapi.screen_capture_allowed():
            self.steps.append({
                "key": "permission",
                "title": "First, let Rewind see your screen",
                "body": "macOS asks every app before it can record the screen. Turn on Rewind in System Settings → "
                        "Privacy & Security → Screen & System Audio Recording. Then quit and reopen Rewind, and "
                        "this tutorial picks up where you left off.",
                "action": ("Open Screen Recording settings", winapi.open_privacy_settings),
                "wait": "Waiting for permission…",
            })
        self.steps += [
            {"key": "welcome", "title": "Rewind is already recording",
             "body": f"It keeps the last {s.clip_seconds} seconds of your screen in memory, all the time. "
                     "Nothing is saved until you ask for it, so it costs you nothing to leave it on."},
            {"key": "clip", "title": "Save the last few seconds",
             "body": "Press your clip key now. Rewind saves what just happened as a video in your clips folder.",
             "combo": hk("clip"), "action": ("Or click here to save one", ctrl.save_clip),
             "wait": "Press the key and watch for the pill in the corner…", "ok": "Clip saved. That's all it takes."},
            {"key": "record", "title": "Record for longer",
             "body": "When you want more than a replay, record. Press the record key once to start and again to stop.",
             "combo": hk("record"), "action": ("Or click here to start / stop", ctrl.toggle_recording),
             "wait": "Start a recording, then stop it…", "ok": "Recording saved."},
            {"key": "shot", "title": "Take a screenshot",
             "body": "One key, and the picture lands right next to your clips.",
             "combo": hk("screenshot"), "action": ("Or click here to take one", ctrl.take_screenshot),
             "wait": "Press the key…", "ok": "Screenshot saved."},
            {"key": "folder", "title": "Where everything goes",
             "body": f"Clips, recordings and screenshots are saved to {s.output_dir}. You can also browse them on "
                     "the Clips page, and change every key on the Hotkeys page.",
             "action": ("Open clips folder", ctrl.open_clips_folder)},
            {"key": "end", "title": "You're set",
             "body": f"Closing this window keeps Rewind running in the {TRAY}, and the pill in the corner of your "
                     f"screen shows it's on. You can watch this again any time from the General page."},
        ]

        self.card = QFrame(self)
        self.card.setObjectName("Card")
        self.card.setFixedWidth(480)
        v = QVBoxLayout(self.card)
        v.setContentsMargins(28, 24, 28, 22)
        v.setSpacing(12)
        self.count = QLabel()
        self.count.setObjectName("CardTitle")
        self.title = QLabel()
        self.title.setStyleSheet("font-size: 21px; font-weight: 700;")
        self.title.setWordWrap(True)
        self.body = QLabel()
        self.body.setObjectName("Muted")
        self.body.setWordWrap(True)
        self.body.setStyleSheet("font-size: 13.5px; line-height: 140%;")
        self.combo = QLabel()
        self.combo.setObjectName("Tile")
        self.combo.setAlignment(Qt.AlignCenter)
        self.combo.setStyleSheet("font-size: 20px; font-weight: 700; padding: 14px 18px; letter-spacing: 1px;")
        self.action = QPushButton()
        self.action.setCursor(Qt.PointingHandCursor)
        self.action.clicked.connect(self._run_action)
        self.status = QLabel()
        self.status.setObjectName("RowSub")
        self.status.setWordWrap(True)
        for w in (self.count, self.title, self.body, self.combo):
            v.addWidget(w)
        ar = QHBoxLayout()
        ar.setSpacing(12)
        ar.addWidget(self.action)
        ar.addWidget(self.status, 1)
        v.addLayout(ar)
        v.addSpacing(6)
        nav = QHBoxLayout()
        self.skip = QPushButton("Skip tutorial")
        self.skip.setObjectName("Link")
        self.skip.setCursor(Qt.PointingHandCursor)
        self.skip.clicked.connect(self.finish)
        self.back = QPushButton("Back")
        self.back.setCursor(Qt.PointingHandCursor)
        self.back.clicked.connect(lambda: self.show_step(self.i - 1))
        self.next = QPushButton("Next")
        self.next.setObjectName("Primary")
        self.next.setCursor(Qt.PointingHandCursor)
        self.next.clicked.connect(self._next)
        nav.addWidget(self.skip)
        nav.addStretch(1)
        nav.addWidget(self.back)
        nav.addWidget(self.next)
        v.addLayout(nav)

        r = ctrl.recorder
        r.clip_saved.connect(lambda *_: self._did("clip"))
        r.recording_saved.connect(lambda *_: self._did("record"))
        ctrl._shot_done.connect(lambda _p, ok: ok and self._did("shot"))
        self._perm_timer = QTimer(self)
        self._perm_timer.setInterval(1000)
        self._perm_timer.timeout.connect(self._check_permission)
        self.hide()

    # ------------------------------------------------------------ flow
    def start(self):
        self.done_steps.clear()
        self.setGeometry(self.parentWidget().rect())
        self.raise_()
        self.show()
        self.show_step(0)
        eff = QGraphicsOpacityEffect(self)
        self.setGraphicsEffect(eff)
        a = QPropertyAnimation(eff, b"opacity", self)
        a.setDuration(220)
        a.setStartValue(0.0)
        a.setEndValue(1.0)
        a.setEasingCurve(QEasingCurve.OutCubic)
        a.finished.connect(lambda: self.setGraphicsEffect(None))
        a.start()
        self._fade = a

    def show_step(self, i):
        self.i = max(0, min(i, len(self.steps) - 1))
        st = self.steps[self.i]
        self.count.setText(f"{self.i + 1} OF {len(self.steps)}")
        self.title.setText(st["title"])
        self.body.setText(st["body"])
        self.combo.setVisible("combo" in st)
        self.combo.setText(st.get("combo", ""))
        act = st.get("action")
        self.action.setVisible(bool(act))
        if act:
            self.action.setText(act[0])
        self.back.setVisible(self.i > 0)
        last = self.i == len(self.steps) - 1
        self.next.setText("Done" if last else "Next")
        self._refresh_status()
        if st["key"] == "permission":
            self._perm_timer.start()
        else:
            self._perm_timer.stop()
        self._place()

    def _refresh_status(self):
        st = self.steps[self.i]
        key = st["key"]
        waiting = "wait" in st and key not in self.done_steps
        if key in self.done_steps and "ok" in st:
            self.status.setText("✓  " + st["ok"])
            self.status.setStyleSheet(f"color: {theme.GREEN}; font-weight: 600;")
        elif key == "clip" and self.ctrl.recorder.state != "on":
            self.status.setText("Instant Replay is off. Turn it on with the switch on Home first.")
            self.status.setStyleSheet(f"color: {theme.WARN};")
        else:
            self.status.setText(st.get("wait", "") if waiting else "")
            self.status.setStyleSheet("")
        self.next.setText(("Done" if self.i == len(self.steps) - 1 else "Next") if not waiting else "Skip this step")
        self.next.setObjectName("" if waiting else "Primary")
        self.next.style().unpolish(self.next)
        self.next.style().polish(self.next)

    def _did(self, key):
        if not self.isVisible():
            return
        self.done_steps.add(key)
        if self.steps[self.i]["key"] == key:
            self._refresh_status()
            self._place()

    def _check_permission(self):
        if winapi.screen_capture_allowed():
            self._did("permission")
            self.status.setText("✓  Allowed. Quit and reopen Rewind so it can start recording.")
            self.status.setStyleSheet(f"color: {theme.GREEN}; font-weight: 600;")
            self._perm_timer.stop()

    def _run_action(self):
        act = self.steps[self.i].get("action")
        if act:
            act[1]()

    def _next(self):
        if self.i >= len(self.steps) - 1:
            self.finish()
        else:
            self.show_step(self.i + 1)

    def finish(self):
        self._perm_timer.stop()
        permission_pending = any(s["key"] == "permission" for s in self.steps) and "permission" not in self.done_steps \
            and not winapi.screen_capture_allowed()
        if not permission_pending:
            self.ctrl.set("tutorial_done", True)
        self.hide()

    # ------------------------------------------------------------ layout
    def _place(self):
        self.card.adjustSize()
        self.card.move((self.width() - self.card.width()) // 2, (self.height() - self.card.height()) // 2)

    def resizeEvent(self, e):
        super().resizeEvent(e)
        self._place()

    def paintEvent(self, e):
        p = QPainter(self)
        p.fillRect(self.rect(), QColor(0, 0, 0, 165))

    def mousePressEvent(self, e):
        e.accept()  # the dim layer blocks clicks to the window behind it
