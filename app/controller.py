"""Glue between the engine, hotkeys, overlay, sounds, tray and the window."""
import datetime
import os
import queue
import threading

from PySide6.QtCore import QObject, QTimer, Signal
from PySide6.QtGui import QAction
from PySide6.QtGui import QGuiApplication
from PySide6.QtWidgets import QApplication, QMenu, QSystemTrayIcon

from . import audio, sounds, theme, winapi
from .hotkeys import HotkeyManager
from .overlay import Overlay
from .recorder import (Recorder, load_meta, make_thumbnail, probe_duration, take_screenshot, thumb_path,
                       update_meta)
from .settings import Settings
from .updates import UpdateChecker

RESTART_KEYS = {"bitrate_mbps", "fps", "resolution", "monitor", "encoder", "codec", "capture_cursor",
                "system_audio", "mic_enabled", "mic_device"}
RECORD_DEFER_KEYS = RESTART_KEYS | {"system_volume", "mic_volume"}

IMAGE_EXTS = (".png", ".jpg", ".jpeg")
VIDEO_EXTS = (".mp4", ".mkv", ".mov")


class Controller(QObject):
    state_changed = Signal(str)
    settings_changed = Signal(str)
    clips_changed = Signal()
    hotkeys_changed = Signal(dict)
    message = Signal(str)          # error / warning banner text ("" clears)
    _shot_done = Signal(str, bool)

    def __init__(self, app: QApplication, autostart=False):
        super().__init__()
        self.app = app
        self.settings = Settings.load()
        self.recorder = Recorder(self.settings)
        self.hotkeys = HotkeyManager()
        self.updates = UpdateChecker()
        self.updates.checked.connect(self._on_update_checked)
        self.overlay = Overlay(self.settings)
        self.hotkey_status = {}
        self.hotkey_dupes = set()
        self._shooting = False
        self.last_error = ""
        self._user_toggled = False
        self._resume_replay_after_recording = False
        self._thumb_q = queue.Queue()
        self._thumb_pending = set()
        threading.Thread(target=self._thumb_worker, daemon=True).start()

        self._save_timer = QTimer(self, singleShot=True, interval=400)
        self._save_timer.timeout.connect(self.settings.save)
        self._restart_timer = QTimer(self, singleShot=True, interval=900)
        self._restart_timer.timeout.connect(self.recorder.restart)

        r = self.recorder
        r.state_changed.connect(self._on_state)
        r.error.connect(self._on_error)
        r.clip_started.connect(self.overlay.show_saving)
        r.clip_saved.connect(self._on_saved)
        r.clip_failed.connect(self._on_failed)
        r.recording_saved.connect(self._on_recording_saved)
        r.recording_failed.connect(self._on_recording_failed)
        r.thumbnail_ready.connect(lambda _p: self.clips_changed.emit())
        self.hotkeys.triggered.connect(self._on_hotkey)
        self._shot_done.connect(self._finish_screenshot)

        from .ui import MainWindow  # local import to avoid a cycle
        self.window = MainWindow(self)
        self._make_tray()
        self.register_hotkeys()
        threading.Thread(target=self.recorder.detect_encoders, daemon=True).start()

        if self.settings.start_replay_on_launch:
            self.recorder.start()
        if self.settings.launch_at_startup:
            winapi.set_run_at_startup(True)  # keep the path fresh if the app moved
        if not (autostart or self.settings.start_minimized) or not QSystemTrayIcon.isSystemTrayAvailable():
            self.window.show()
        self.overlay.apply_settings()
        QTimer.singleShot(8000, self.updates.check)

    def _on_update_checked(self, status, version, _url):
        if status == "newer" and self.tray is not None and not self.window.isVisible():
            self.tray.showMessage("Rewind", f"Rewind {version} is out. Open Rewind → General to update.",
                                  theme.app_icon(), 6000)

    # ------------------------------------------------------------ settings
    def set(self, key, value):
        if getattr(self.settings, key) == value:
            return
        if self.recorder.mode == "recording" and key in RECORD_DEFER_KEYS:
            setattr(self.settings, key, value)
            self._save_timer.start()
            if key.startswith("overlay_") or key == "clip_seconds":
                self.overlay.apply_settings()
            self.settings_changed.emit(key)
            self.message.emit("Capture setting saved. It will apply after this recording.")
            return
        setattr(self.settings, key, value)
        self._save_timer.start()
        if key in RESTART_KEYS and self.recorder.state in ("on", "starting"):
            self._restart_timer.start()
        if key.startswith("hotkey_"):
            self.register_hotkeys()
        if key.startswith("overlay_") or key == "clip_seconds":
            self.overlay.apply_settings()
            self.overlay.preview() if key.startswith("overlay_") else self.overlay.update()
        if key == "launch_at_startup":
            winapi.set_run_at_startup(bool(value))
        if key == "output_dir":
            self.clips_changed.emit()
        self.settings_changed.emit(key)

    # ------------------------------------------------------------ replay
    def set_replay(self, on: bool):
        if self.recorder.mode == "recording":
            self.message.emit("Instant Replay is paused while a full recording is in progress.")
            return
        self._user_toggled = True
        if on:
            self.recorder.start()
        else:
            self.recorder.stop()

    def toggle_replay(self):
        self.set_replay(self.recorder.state == "off")

    def toggle_recording(self):
        if self.recorder.mode == "recording":
            self.recorder.stop_recording()
            return
        if self.recorder.state == "saving":
            return
        self._resume_replay_after_recording = (self.recorder.state in ("on", "starting") and
                                               self.recorder.mode == "replay")
        self.recorder.start_recording()

    def save_clip(self):
        self.recorder.save_clip(self.settings.clip_seconds)

    def _on_state(self, st):
        if st == "on":
            self.last_error = ""
            note = self.recorder.audio_note
            self.message.emit(f"Recording without some audio: {note}" if note else "")
        if self._user_toggled and st in ("on", "off") and self.settings.toggle_sound:
            sounds.play("toggle_on" if st == "on" else "toggle_off", self.settings.sound_volume)
        if st in ("on", "off"):
            self._user_toggled = False
        overlay_state = "recording" if st == "on" and self.recorder.mode == "recording" else st
        self.overlay.set_state(overlay_state)
        self._update_tray(st)
        self.state_changed.emit(st)

    def _on_error(self, msg):
        self.last_error = msg
        self.message.emit(msg)
        if not self.window.isVisible() and self.tray is not None:
            self.tray.showMessage("Rewind", msg, theme.app_icon(), 6000)

    def _on_saved(self, path, dur):
        s = self.settings
        if s.sound_enabled:
            sounds.play(s.sound_style, s.sound_volume, s.sound_custom)
        self.overlay.show_saved(dur)
        self.clips_changed.emit()

    def _on_failed(self, msg):
        self.overlay.show_failed(msg)

    def _on_recording_saved(self, path, dur):
        s = self.settings
        if s.sound_enabled:
            sounds.play(s.sound_style, s.sound_volume, s.sound_custom)
        self.overlay.show_recording_saved(dur)
        self.clips_changed.emit()
        if self._resume_replay_after_recording:
            self._resume_replay_after_recording = False
            self.recorder.start()

    def _on_recording_failed(self, msg):
        self.message.emit(msg)
        self.overlay.show_failed(msg, "RECORDING FAILED")
        if self._resume_replay_after_recording:
            self._resume_replay_after_recording = False
            self.recorder.start()

    # ------------------------------------------------------------ hotkeys
    def register_hotkeys(self, paused=False):
        s = self.settings
        if paused:
            self.hotkeys.stop()
            return
        mapping, self.hotkey_dupes = {}, set()
        for name, hk in (("toggle", s.hotkey_toggle), ("clip", s.hotkey_clip), ("record", s.hotkey_record),
                         ("shot", s.hotkey_screenshot), ("open", s.hotkey_open)):
            combo = (hk.get("mods", 0), hk.get("vk"))
            if combo[1] is not None and combo in mapping.values():
                self.hotkey_dupes.add(name)
                continue
            mapping[name] = combo
        self.hotkey_status = self.hotkeys.set_hotkeys(mapping)
        for name in self.hotkey_dupes:
            self.hotkey_status[name] = False
        self.hotkeys_changed.emit(dict(self.hotkey_status))

    def _on_hotkey(self, name):
        if name == "toggle":
            self.toggle_replay()
        elif name == "clip":
            self.save_clip()
        elif name == "record":
            self.toggle_recording()
        elif name == "shot":
            self.take_screenshot()
        elif name == "open":
            self.show_window()

    # ------------------------------------------------------------ screenshots
    def take_screenshot(self):
        if self._shooting:
            return
        s = self.settings
        try:
            os.makedirs(s.output_dir, exist_ok=True)
        except OSError as e:
            self.overlay.show_failed(str(e), "SCREENSHOT FAILED")
            return
        ext = "jpg" if s.screenshot_format == "jpg" else "png"
        stamp = datetime.datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
        path = os.path.join(s.output_dir, f"Rewind_Screenshot_{stamp}.{ext}")
        n = 2
        while os.path.exists(path):
            path = os.path.join(s.output_dir, f"Rewind_Screenshot_{stamp}_{n}.{ext}")
            n += 1
        self._shooting = True
        ff = self.recorder.ffmpeg
        mon = s.monitor if 0 <= s.monitor < len(self.recorder.monitors) else 0

        def work():
            ok = bool(ff) and take_screenshot(ff, mon, path, s.capture_cursor)
            self._shot_done.emit(path, ok)

        threading.Thread(target=work, daemon=True).start()

    def _finish_screenshot(self, path, ok):
        self._shooting = False
        if not ok:  # fallback: plain Qt grab of the same screen
            screens = QGuiApplication.screens()
            idx = self.settings.monitor
            scr = screens[idx] if 0 <= idx < len(screens) else QGuiApplication.primaryScreen()
            try:
                ok = scr is not None and scr.grabWindow(0).save(path, None, 92)
            except Exception:
                ok = False
        if ok:
            s = self.settings
            if s.sound_enabled and s.screenshot_sound:
                sounds.play("shutter", s.sound_volume)
            self.overlay.show_screenshot(os.path.basename(path))
            self.clips_changed.emit()
        else:
            self.overlay.show_failed("Couldn't capture screen", "SCREENSHOT FAILED")

    # ------------------------------------------------------------ clips
    def list_clips(self, limit=400):
        d = self.settings.output_dir
        items = []
        try:
            for e in os.scandir(d):
                if e.is_file() and e.name.lower().endswith(VIDEO_EXTS + IMAGE_EXTS):
                    st = e.stat()
                    items.append((e.path, st.st_mtime, st.st_size))
        except OSError:
            return []
        items.sort(key=lambda t: -t[1])
        meta = load_meta()
        out = []
        for path, mtime, size in items[:limit]:
            if path.lower().endswith(IMAGE_EXTS):
                out.append({"path": path, "mtime": mtime, "size": size, "duration": None,
                            "thumb": path, "kind": "image"})
                continue
            out.append({"path": path, "mtime": mtime, "size": size, "kind": "video",
                        "duration": meta.get(os.path.basename(path), {}).get("duration"),
                        "thumb": thumb_path(path) if os.path.exists(thumb_path(path)) else None})
        return out

    def request_thumbs(self, clips):
        for c in clips:
            if c.get("kind") == "video" and not c["thumb"] and c["path"] not in self._thumb_pending:
                self._thumb_pending.add(c["path"])
                self._thumb_q.put(c["path"])

    def _thumb_worker(self):
        while True:
            path = self._thumb_q.get()
            ff = self.recorder.ffmpeg
            if ff and os.path.exists(path):
                dur = probe_duration(ff, path)
                if dur:
                    try:
                        update_meta(os.path.basename(path), duration=round(dur, 2))
                    except Exception:
                        pass
                if make_thumbnail(ff, path, dur) and self._thumb_q.empty():
                    self.recorder.thumbnail_ready.emit(path)
            self._thumb_pending.discard(path)

    def open_clips_folder(self):
        os.makedirs(self.settings.output_dir, exist_ok=True)
        winapi.open_path(self.settings.output_dir)

    # ------------------------------------------------------------ misc
    def test_sound(self):
        s = self.settings
        sounds.play(s.sound_style, s.sound_volume, s.sound_custom)

    def preview_overlay(self):
        self.overlay.show_saved(self.settings.clip_seconds)

    def microphones(self):
        return audio.list_microphones()

    # ------------------------------------------------------------ tray
    def _make_tray(self):
        self.tray = None
        if not QSystemTrayIcon.isSystemTrayAvailable():
            return
        self.tray = QSystemTrayIcon(theme.app_icon(dim=True), self)
        menu = QMenu()
        a_open = QAction("Open Rewind", menu)
        a_open.triggered.connect(self.show_window)
        self.a_toggle = QAction("Instant Replay", menu, checkable=True)
        self.a_toggle.triggered.connect(lambda on: self.set_replay(on))
        self.a_record = QAction("Start screen recording", menu)
        self.a_record.triggered.connect(self.toggle_recording)
        self.a_clip = QAction("Save clip", menu)
        self.a_clip.triggered.connect(self.save_clip)
        a_shot = QAction("Take screenshot", menu)
        a_shot.triggered.connect(self.take_screenshot)
        a_folder = QAction("Open clips folder", menu)
        a_folder.triggered.connect(self.open_clips_folder)
        a_quit = QAction("Quit Rewind", menu)
        a_quit.triggered.connect(self.quit)
        for a in (a_open, None, self.a_toggle, self.a_record, self.a_clip, a_shot, a_folder, None, a_quit):
            menu.addSeparator() if a is None else menu.addAction(a)
        self.tray.setContextMenu(menu)
        self._tray_menu = menu
        self.tray.activated.connect(self._tray_activated)
        self.tray.setToolTip("Rewind — Instant Replay off")
        self.tray.show()

    def _tray_activated(self, reason):
        if reason in (QSystemTrayIcon.Trigger, QSystemTrayIcon.DoubleClick):
            self.show_window()

    def _update_tray(self, st):
        if self.tray is None:
            return
        recording = self.recorder.mode == "recording"
        self.tray.setIcon(theme.app_icon(dim=(st != "on")))
        if recording and st == "starting":
            label = "Starting screen recording…"
        elif recording:
            label = "Recording"
        else:
            label = {"on": "Instant Replay on", "starting": "Starting Instant Replay…",
                     "saving": "Saving recording…"}.get(st, "Instant Replay off")
        self.tray.setToolTip(f"Rewind — {label}")
        self.a_toggle.setChecked(st in ("on", "starting") and not recording)
        self.a_toggle.setEnabled(not recording and st != "saving")
        self.a_record.setText("Stop screen recording" if recording else "Start screen recording")
        self.a_record.setEnabled(st != "saving")
        self.a_clip.setEnabled(st == "on" and not recording)

    def show_window(self):
        self.window.showNormal()
        self.window.raise_()
        self.window.activateWindow()
        winapi.bring_to_front(int(self.window.winId()))

    def window_closed(self):
        """Main window closed → keep running in the tray."""
        if self.tray is None:
            self.quit()
            return
        if not self.settings.tray_notice_shown:
            activity = "Screen recording keeps working in the background." if self.recorder.mode == "recording" \
                       else "Instant Replay keeps working in the background."
            self.tray.showMessage("Rewind is still running",
                                  activity + " "
                                  "Right-click the tray icon to quit.", theme.app_icon(), 5000)
            self.set("tray_notice_shown", True)

    def quit(self):
        self.settings.save()
        self._resume_replay_after_recording = False
        self.hotkeys.stop()
        self.recorder.shutdown()
        if self.tray is not None:
            self.tray.hide()
        self.app.quit()
