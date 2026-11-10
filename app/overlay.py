"""The little on-screen status pill (bottom-right by default).

Click-through, never steals focus, and is excluded from screen capture so it
never appears in clips or full recordings.
"""
from PySide6.QtCore import Property, QEasingCurve, QPropertyAnimation, QRectF, Qt, QTimer
from PySide6.QtGui import QColor, QFont, QFontMetricsF, QGuiApplication, QPainter, QPainterPath, QPen
from PySide6.QtWidgets import QWidget

from . import theme, winapi


class Overlay(QWidget):
    def __init__(self, settings):
        super().__init__(None, Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.Tool |
                         Qt.WindowTransparentForInput | Qt.WindowDoesNotAcceptFocus | Qt.NoDropShadowWindowHint)
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setAttribute(Qt.WA_ShowWithoutActivating)
        self.setWindowTitle("Rewind overlay")
        self.s = settings
        self.state = "off"
        self.flash = None          # (kind, text)
        self._pw = 120.0
        self._phase = 0.0
        self._bounce = 0.0
        self._target_visible = False

        self._w_anim = QPropertyAnimation(self, b"pillWidth", self)
        self._w_anim.setDuration(320)
        self._w_anim.setEasingCurve(QEasingCurve.OutCubic)
        self._fade = QPropertyAnimation(self, b"windowOpacity", self)
        self._fade.setDuration(240)
        self._fade.setEasingCurve(QEasingCurve.OutCubic)
        self._fade.finished.connect(self._fade_done)
        self._bounce_anim = QPropertyAnimation(self, b"bounce", self)
        self._bounce_anim.setDuration(520)
        self._bounce_anim.setEasingCurve(QEasingCurve.OutElastic)

        self._tick = QTimer(self)
        self._tick.setInterval(33)
        self._tick.timeout.connect(self._on_tick)
        self._hide_timer = QTimer(self, singleShot=True)
        self._hide_timer.timeout.connect(lambda: self._set_visible(False))
        self._flash_timer = QTimer(self, singleShot=True)
        self._flash_timer.timeout.connect(self._end_flash)
        self._top_timer = QTimer(self)
        self._top_timer.setInterval(2000)
        self._top_timer.timeout.connect(lambda: winapi.make_topmost(int(self.winId())))
        self.apply_settings()

    # ---- animated properties
    def _get_pw(self):
        return self._pw

    def _set_pw(self, v):
        self._pw = v
        self.update()

    pillWidth = Property(float, _get_pw, _set_pw)

    def _get_bounce(self):
        return self._bounce

    def _set_bounce(self, v):
        self._bounce = v
        self.update()

    bounce = Property(float, _get_bounce, _set_bounce)

    # ---- layout
    @property
    def k(self):
        return self.s.overlay_scale / 100.0

    def apply_settings(self):
        k = self.k
        w, h = int(240 * k), int(46 * k)
        self.setFixedSize(w, h)
        scr = QGuiApplication.primaryScreen()
        if scr is not None:
            g = scr.availableGeometry()
            m = int(4 * k)
            corner = self.s.overlay_corner
            x = g.right() - w - m + 1 if "right" in corner else g.left() + m
            y = g.bottom() - h - m + 1 if "bottom" in corner else g.top() + m
            self.move(x, y)
        self._retarget(animate=False)
        self._refresh_visibility(bump=False)

    def _content(self):
        if self.flash:
            kind, text = self.flash
            if kind == "saving":
                return theme.ACCENT, "spinner", "REWIND", "Saving clip…"
            if kind == "saved":
                return theme.GREEN, "check", "CLIP SAVED", text
            if kind == "recorded":
                return theme.GREEN, "check", "RECORDING SAVED", text
            if kind == "shot":
                return theme.ACCENT, "check", "SCREENSHOT", text
            return theme.WARN, "bang", getattr(self, "_fail_title", "CLIP FAILED"), text
        if self.state == "on":
            return theme.ACCENT, "rewind", "REWIND", f"ON  ·  {self.s.clip_seconds}s"
        if self.state == "recording":
            return theme.GREEN, "rewind", "RECORDING", "Screen capture on"
        if self.state == "saving":
            return theme.ACCENT, "spinner", "REWIND", "Saving recording…"
        if self.state == "starting":
            return "#777777", "spinner", "REWIND", "Starting…"
        return "#555555", "rewind", "REWIND", "Off"

    def _fonts(self):
        k = self.k
        f1 = QFont()
        f1.setFamilies(theme.ui_font_families())
        f1.setPixelSize(max(7, int(8 * k)))
        f1.setWeight(QFont.Bold)
        f1.setLetterSpacing(QFont.AbsoluteSpacing, 1.0 * k)
        f2 = QFont()
        f2.setFamilies(theme.ui_font_families())
        f2.setPixelSize(max(9, int(11 * k)))
        f2.setWeight(QFont.Bold)
        return f1, f2

    def _target_width(self):
        k = self.k
        _, _, l1, l2 = self._content()
        f1, f2 = self._fonts()
        tw = max(QFontMetricsF(f1).horizontalAdvance(l1), QFontMetricsF(f2).horizontalAdvance(l2))
        return min(self.width() - 4, 5 * k + 24 * k + 8 * k + tw + 13 * k)

    def _retarget(self, animate=True):
        target = self._target_width()
        self._w_anim.stop()
        if animate and self.isVisible():
            self._w_anim.setStartValue(self._pw)
            self._w_anim.setEndValue(target)
            self._w_anim.start()
        else:
            self._pw = target
        self._update_tick()
        self.update()

    def _update_tick(self):
        _, glyph, _, _ = self._content()
        need = self.isVisible() and (glyph == "spinner" or (self.state in ("on", "recording") and not self.flash))
        if need and not self._tick.isActive():
            self._tick.start()
        elif not need:
            self._tick.stop()

    def _on_tick(self):
        self._phase = (self._phase + 0.033) % 1000
        self.update()

    # ---- visibility
    def _refresh_visibility(self, bump):
        if not self.s.overlay_enabled:
            self._hide_timer.stop()
            self._set_visible(False)
            return
        if self.s.overlay_mode == "always" or self.flash:
            self._hide_timer.stop()
            self._set_visible(True)
        elif bump:
            self._set_visible(True)
            self._hide_timer.start(2600)

    def _set_visible(self, v):
        if v == self._target_visible and (self.isVisible() == v):
            return
        self._target_visible = v
        target = self.s.overlay_opacity / 100.0 if v else 0.0
        if v and not self.isVisible():
            self.setWindowOpacity(0.0)
            self.show()
            hwnd = int(self.winId())
            winapi.exclude_from_capture(hwnd)
            winapi.make_topmost(hwnd)
            self._top_timer.start()
        self._fade.stop()
        self._fade.setStartValue(self.windowOpacity())
        self._fade.setEndValue(target)
        self._fade.start()
        self._update_tick()

    def _fade_done(self):
        if not self._target_visible:
            self.hide()
            self._top_timer.stop()
            self._tick.stop()

    # ---- events from the app
    def set_state(self, state):
        if state == self.state:
            return
        self.state = state
        self._retarget()
        self._refresh_visibility(bump=True)
        self._pop()

    def show_saving(self):
        self.flash = ("saving", "")
        self._flash_timer.start(20000)
        self._retarget()
        self._refresh_visibility(bump=True)

    def show_saved(self, seconds):
        self.flash = ("saved", f"Last {int(round(seconds))}s")
        self._flash_timer.start(2400)
        self._retarget()
        self._refresh_visibility(bump=True)
        self._pop()

    def show_recording_saved(self, seconds):
        self.flash = ("recorded", f"{int(round(seconds))} seconds")
        self._flash_timer.start(2400)
        self._retarget()
        self._refresh_visibility(bump=True)
        self._pop()

    def show_screenshot(self, name=""):
        self.flash = ("shot", "Saved to folder")
        self._flash_timer.start(1800)
        self._retarget()
        self._refresh_visibility(bump=True)
        self._pop()

    def show_failed(self, msg, title="CLIP FAILED"):
        self._fail_title = title
        self.flash = ("failed", (msg or "Error")[:28])
        self._flash_timer.start(3000)
        self._retarget()
        self._refresh_visibility(bump=True)
        self._pop()

    def preview(self):
        self._refresh_visibility(bump=True)
        self._pop()

    def _end_flash(self):
        self.flash = None
        self._retarget()
        if self.s.overlay_mode != "always":
            self._hide_timer.start(900)

    def _pop(self):
        self._bounce_anim.stop()
        self._bounce_anim.setStartValue(1.0)
        self._bounce_anim.setEndValue(0.0)
        self._bounce_anim.start()

    # ---- painting
    def paintEvent(self, e):
        k = self.k
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        color, glyph, l1, l2 = self._content()
        color = QColor(color)
        ph = 34 * k
        pw = self._pw
        y = (self.height() - ph) / 2
        x = self.width() - pw - 1 if "right" in self.s.overlay_corner else 2.0
        rect = QRectF(x, y, pw, ph)

        # shadow + body
        for i, a in ((6, 18), (3, 30)):
            p.setPen(Qt.NoPen)
            p.setBrush(QColor(0, 0, 0, a))
            p.drawRoundedRect(rect.adjusted(-i * 0.5, -i * 0.3, i * 0.5, i * 0.7), ph / 2 + i, ph / 2 + i)
        p.setBrush(QColor(22, 22, 22, 238))
        p.setPen(QPen(QColor(255, 255, 255, 26), 1))
        p.drawRoundedRect(rect, ph / 2, ph / 2)

        path = QPainterPath()
        path.addRoundedRect(rect, ph / 2, ph / 2)
        p.setClipPath(path)

        # icon disc
        d = 24 * k * (1 + 0.12 * self._bounce)
        cx = x + 5 * k + 12 * k
        cy = y + ph / 2
        if self.state in ("on", "recording") and not self.flash:
            ph_ = (self._phase * 0.9) % 1.0
            halo = QColor(color)
            halo.setAlphaF(0.40 * (1 - ph_))
            rr = d / 2 + 5 * k * ph_
            p.setPen(Qt.NoPen)
            p.setBrush(halo)
            p.drawEllipse(QRectF(cx - rr, cy - rr, 2 * rr, 2 * rr))
        p.setPen(Qt.NoPen)
        p.setBrush(color)
        p.drawEllipse(QRectF(cx - d / 2, cy - d / 2, d, d))

        white = QColor("white")
        if glyph == "rewind":
            glyph_color = QColor("#111111") if QColor(color).lightness() > 150 else white
            theme.draw_rewind_glyph(p, cx, cy, d * 0.42, glyph_color)
        elif glyph == "check":
            pen = QPen(white, 2.0 * k)
            pen.setCapStyle(Qt.RoundCap)
            pen.setJoinStyle(Qt.RoundJoin)
            p.setPen(pen)
            s = d * 0.2
            chk = QPainterPath()
            chk.moveTo(cx - s * 1.05, cy + s * 0.05)
            chk.lineTo(cx - s * 0.25, cy + s * 0.85)
            chk.lineTo(cx + s * 1.15, cy - s * 0.75)
            p.drawPath(chk)
        elif glyph == "spinner":
            pen = QPen(white, 1.8 * k)
            pen.setCapStyle(Qt.RoundCap)
            p.setPen(pen)
            r = d * 0.26
            start = int(-(self._phase * 360 * 1.6) % 360 * 16)
            p.drawArc(QRectF(cx - r, cy - r, 2 * r, 2 * r), start, 260 * 16)
        else:  # bang
            p.setPen(Qt.NoPen)
            p.setBrush(QColor("#1A1305"))
            p.drawRoundedRect(QRectF(cx - 1.1 * k, cy - 5.2 * k, 2.2 * k, 6.8 * k), 1.1 * k, 1.1 * k)
            p.drawEllipse(QRectF(cx - 1.35 * k, cy + 2.6 * k, 2.7 * k, 2.7 * k))

        # text
        f1, f2 = self._fonts()
        tx = x + 5 * k + 24 * k + 8 * k
        p.setFont(f1)
        p.setPen(QColor(theme.MUTED) if not self.flash else color)
        p.drawText(QRectF(tx, y + 4 * k, pw, 11 * k), Qt.AlignLeft | Qt.AlignVCenter, l1)
        p.setFont(f2)
        p.setPen(QColor(theme.TEXT))
        p.drawText(QRectF(tx, y + 14 * k, pw, 16 * k), Qt.AlignLeft | Qt.AlignVCenter, l2)
