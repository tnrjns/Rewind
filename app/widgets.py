"""Custom animated widgets."""
from PySide6.QtCore import (QEvent, QObject, Property, QEasingCurve, QPropertyAnimation, QRectF, QSize, Qt, QTimer,
                            Signal)
from PySide6.QtGui import QColor, QKeySequence, QPainter, QPainterPath, QPixmap
from PySide6.QtWidgets import (QAbstractButton, QAbstractScrollArea, QAbstractSlider, QAbstractSpinBox,
                               QApplication, QComboBox, QScrollBar, QFrame, QHBoxLayout, QLabel, QListView,
                               QPushButton, QSizePolicy, QSlider, QVBoxLayout, QWidget)

from . import theme
from .hotkeys import IS_MAC, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN


class WheelGuard(QObject):
    """Scrolling over a dropdown, slider or number box scrolls the page instead of changing the setting."""

    def eventFilter(self, obj, e):
        if e.type() == QEvent.Wheel and isinstance(obj, (QComboBox, QAbstractSlider, QAbstractSpinBox)) \
                and not isinstance(obj, QScrollBar):
            if isinstance(obj, QComboBox) and obj.view().isVisible():
                return False
            p = obj.parentWidget()
            while p is not None and not isinstance(p, QAbstractScrollArea):
                p = p.parentWidget()
            if p is not None:
                QApplication.sendEvent(p.verticalScrollBar(), e)
            return True
        return False


def steady_width(btn, texts):
    """Size a button for its longest label so it never resizes, and nothing next to it shifts."""
    btn.ensurePolished()
    current = btn.text()
    widest = 0
    for t in texts:
        btn.setText(t)
        widest = max(widest, btn.sizeHint().width())
    btn.setText(current)
    btn.setMinimumWidth(widest)
    btn.setMinimumHeight(btn.sizeHint().height())


class ToggleSwitch(QAbstractButton):
    """iOS/Win11-style switch with an eased knob."""

    def __init__(self, parent=None, large=False):
        super().__init__(parent)
        self.setCheckable(True)
        self.setCursor(Qt.PointingHandCursor)
        self._w, self._h = (62, 34) if large else (44, 24)
        self.setFixedSize(self._w, self._h)
        self._knob = 0.0
        self._hover = 0.0
        self._anim = QPropertyAnimation(self, b"knob", self)
        self._anim.setDuration(220)
        self._anim.setEasingCurve(QEasingCurve.OutCubic)
        self.toggled.connect(self._animate)

    def sizeHint(self):
        return QSize(self._w, self._h)

    def _get_knob(self):
        return self._knob

    def _set_knob(self, v):
        self._knob = v
        self.update()

    knob = Property(float, _get_knob, _set_knob)

    def _animate(self, on):
        self._anim.stop()
        self._anim.setStartValue(self._knob)
        self._anim.setEndValue(1.0 if on else 0.0)
        self._anim.start()

    def set_silent(self, on: bool):
        self.blockSignals(True)
        self.setChecked(on)
        self.blockSignals(False)
        if not self._anim.state() == QPropertyAnimation.Running or self._anim.endValue() != (1.0 if on else 0.0):
            self._animate(on)

    def enterEvent(self, e):
        self._hover = 1.0
        self.update()

    def leaveEvent(self, e):
        self._hover = 0.0
        self.update()

    def paintEvent(self, e):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        w, h = self.width(), self.height()
        t = self._knob
        off = QColor("#4A4A4A") if not self._hover else QColor("#5A5A5A")
        track = theme.mix(off, QColor(theme.ACCENT), t)
        if not self.isEnabled():
            track.setAlpha(110)
        p.setPen(Qt.NoPen)
        p.setBrush(track)
        p.drawRoundedRect(QRectF(0, 0, w, h), h / 2, h / 2)
        m = 3
        d = h - 2 * m
        x = m + t * (w - 2 * m - d)
        p.setBrush(QColor(0, 0, 0, 40))
        p.drawEllipse(QRectF(x, m + 1, d, d))
        p.setBrush(QColor("white") if self.isEnabled() else QColor("#8A8A8A"))
        p.drawEllipse(QRectF(x, m, d, d))


class LiveDot(QWidget):
    """Pulsing record dot."""

    def __init__(self, parent=None, size=12):
        super().__init__(parent)
        self._size = size
        self.setFixedSize(size * 2, size * 2)
        self._on = False
        self._phase = 0.0
        self._timer = QTimer(self)
        self._timer.setInterval(33)
        self._timer.timeout.connect(self._tick)
        self.color = QColor(theme.ACCENT)

    def set_state(self, state):
        self._on = state == "on"
        self.color = QColor(theme.ACCENT if state == "on" else (theme.WARN if state == "starting" else "#555555"))
        if state in ("on", "starting"):
            self._timer.start()
        else:
            self._timer.stop()
        self.update()

    def _tick(self):
        self._phase = (self._phase + 0.045) % 1.0
        self.update()

    def paintEvent(self, e):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        c = self.width() / 2
        r = self._size / 2
        if self._timer.isActive():
            halo = QColor(self.color)
            halo.setAlphaF(0.45 * (1 - self._phase))
            rr = r + r * 1.0 * self._phase
            p.setPen(Qt.NoPen)
            p.setBrush(halo)
            p.drawEllipse(QRectF(c - rr, c - rr, 2 * rr, 2 * rr))
        p.setPen(Qt.NoPen)
        p.setBrush(self.color)
        p.drawEllipse(QRectF(c - r, c - r, 2 * r, 2 * r))


def _vk_name(vk, fallback):
    if 0x30 <= vk <= 0x39 or 0x41 <= vk <= 0x5A:
        return chr(vk)
    if 0x70 <= vk <= 0x87:
        return f"F{vk - 0x6F}"
    if 0x60 <= vk <= 0x69:
        return f"Num {vk - 0x60}"
    names = {0x20: "Space", 0x2C: "PrtSc", 0x2D: "Insert", 0x2E: "Delete", 0x24: "Home", 0x23: "End",
             0x21: "PgUp", 0x22: "PgDn", 0x13: "Pause", 0x91: "ScrLk", 0xC0: "`", 0xBD: "-", 0xBB: "=",
             0xDB: "[", 0xDD: "]", 0xDC: "\\", 0xBA: ";", 0xDE: "'", 0xBC: ",", 0xBE: ".", 0xBF: "/",
             0x25: "Left", 0x26: "Up", 0x27: "Right", 0x28: "Down", 0x6A: "Num *", 0x6B: "Num +",
             0x6D: "Num -", 0x6F: "Num /", 0x6E: "Num ."}
    return names.get(vk, fallback)


class HotkeyEdit(QPushButton):
    """Click, then press a key combo."""
    changed = Signal(dict)
    capturing_changed = Signal(bool)

    MODIFIER_KEYS = {Qt.Key_Control, Qt.Key_Shift, Qt.Key_Alt, Qt.Key_Meta, Qt.Key_AltGr}

    def __init__(self, hk: dict, parent=None):
        super().__init__(parent)
        self.setObjectName("HotkeyEdit")
        self.setCursor(Qt.PointingHandCursor)
        self.hk = dict(hk)
        self._capturing = False
        self.clicked.connect(self.begin)
        self._refresh()

    def set_hotkey(self, hk):
        self.hk = dict(hk)
        self._refresh()

    def _refresh(self):
        if self._capturing:
            return
        self.setText(self.hk.get("text") or "Not set")

    def _set_capturing(self, v):
        self._capturing = v
        self.setProperty("capturing", "true" if v else "false")
        self.style().unpolish(self)
        self.style().polish(self)
        self.capturing_changed.emit(v)

    def begin(self):
        if self._capturing:
            return
        self._set_capturing(True)
        self.setText("Press keys…  (Esc cancels)")
        self.grabKeyboard()

    def _end(self):
        self.releaseKeyboard()
        self._set_capturing(False)
        self._refresh()

    @staticmethod
    def _mods(qmods):
        if IS_MAC:  # Qt swaps them on macOS: ControlModifier is Command, MetaModifier is Control
            pairs = ((Qt.MetaModifier, MOD_CONTROL, "⌃"), (Qt.AltModifier, MOD_ALT, "⌥"),
                     (Qt.ShiftModifier, MOD_SHIFT, "⇧"), (Qt.ControlModifier, MOD_WIN, "⌘"))
            mods, parts = 0, []
            for qm, flag, sym in pairs:
                if qmods & qm:
                    mods |= flag
                    parts.append(sym)
            return mods, parts
        mods, parts = 0, []
        if qmods & Qt.ControlModifier:
            mods |= MOD_CONTROL
            parts.append("Ctrl")
        if qmods & Qt.AltModifier:
            mods |= MOD_ALT
            parts.append("Alt")
        if qmods & Qt.ShiftModifier:
            mods |= MOD_SHIFT
            parts.append("Shift")
        if qmods & Qt.MetaModifier:
            mods |= MOD_WIN
            parts.append("Win")
        return mods, parts

    def keyPressEvent(self, e):
        if not self._capturing:
            return super().keyPressEvent(e)
        key = e.key()
        if key == Qt.Key_Escape:
            self._end()
            return
        mods, parts = self._mods(e.modifiers())
        sep = "" if IS_MAC else " + "
        if key in self.MODIFIER_KEYS:
            self.setText(sep.join(parts + ["…"]))
            return
        vk = e.nativeVirtualKey() if IS_MAC else (e.nativeVirtualKey() or key)
        fallback = QKeySequence(key).toString() or f"Key {vk}"
        name = fallback if IS_MAC else _vk_name(vk, fallback)
        self.hk = {"mods": mods, "vk": int(vk), "text": sep.join(parts + [name])}
        self._end()
        self.changed.emit(dict(self.hk))

    def keyReleaseEvent(self, e):
        if self._capturing:
            return
        super().keyReleaseEvent(e)

    def focusOutEvent(self, e):
        if self._capturing:
            self._end()
        super().focusOutEvent(e)


class Card(QFrame):
    def __init__(self, title=None, parent=None):
        super().__init__(parent)
        self.setObjectName("Card")
        self.v = QVBoxLayout(self)
        self.v.setContentsMargins(22, 16, 22, 10)
        self.v.setSpacing(0)
        if title:
            lbl = QLabel(title.upper())
            lbl.setObjectName("CardTitle")
            self.v.addWidget(lbl)
            self.v.addSpacing(4)
        self._rows = 0

    def add_row(self, title, subtitle, control):
        if self._rows:
            sep = QFrame()
            sep.setObjectName("Sep")
            sep.setFixedHeight(1)
            self.v.addWidget(sep)
        row = SettingRow(title, subtitle, control)
        self.v.addWidget(row)
        self._rows += 1
        return row

    def add_widget(self, w):
        self.v.addWidget(w)


class SettingRow(QWidget):
    def __init__(self, title, subtitle, control, parent=None):
        super().__init__(parent)
        h = QHBoxLayout(self)
        h.setContentsMargins(0, 12, 0, 12)
        h.setSpacing(18)
        col = QVBoxLayout()
        col.setSpacing(3)
        self.title = QLabel(title)
        self.title.setObjectName("RowTitle")
        col.addWidget(self.title)
        self.sub = QLabel(subtitle or "")
        self.sub.setObjectName("RowSub")
        self.sub.setWordWrap(True)
        self.sub.setVisible(bool(subtitle))
        col.addWidget(self.sub)
        h.addLayout(col, 1)
        if control is not None:
            h.addWidget(control, 0, Qt.AlignVCenter | Qt.AlignRight)

    def set_sub(self, text):
        self.sub.setText(text or "")
        self.sub.setVisible(bool(text))


class SliderControl(QWidget):
    valueChanged = Signal(int)

    def __init__(self, lo, hi, step, value, fmt, width=230, parent=None):
        super().__init__(parent)
        self.lo, self.hi, self.step, self.fmt = lo, hi, step, fmt
        h = QHBoxLayout(self)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(14)
        self.slider = QSlider(Qt.Horizontal)
        self.slider.setRange(lo, hi)
        self.slider.setSingleStep(step)
        self.slider.setPageStep(step)
        self.slider.setFixedWidth(width)
        self.slider.setCursor(Qt.PointingHandCursor)
        self.label = QLabel()
        self.label.setObjectName("Value")
        self.label.setMinimumWidth(78)
        self.label.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        h.addWidget(self.slider)
        h.addWidget(self.label)
        self.slider.valueChanged.connect(self._changed)
        self.set_value(value)

    def _snap(self, v):
        return max(self.lo, min(self.hi, self.lo + round((v - self.lo) / self.step) * self.step))

    def _changed(self, v):
        s = self._snap(v)
        if s != v:
            self.slider.blockSignals(True)
            self.slider.setValue(s)
            self.slider.blockSignals(False)
        self.label.setText(self.fmt(s))
        self.valueChanged.emit(s)

    def set_value(self, v):
        self.slider.blockSignals(True)
        self.slider.setValue(self._snap(v))
        self.slider.blockSignals(False)
        self.label.setText(self.fmt(self._snap(v)))

    def value(self):
        return self.slider.value()


class ChipGroup(QWidget):
    selected = Signal(object)

    def __init__(self, options, parent=None):
        super().__init__(parent)
        h = QHBoxLayout(self)
        h.setContentsMargins(0, 0, 0, 0)
        h.setSpacing(8)
        self.buttons = {}
        for value, label in options:
            b = QPushButton(label)
            b.setObjectName("Chip")
            b.setCheckable(True)
            b.setCursor(Qt.PointingHandCursor)
            b.clicked.connect(lambda _=False, v=value: self._click(v))
            h.addWidget(b)
            self.buttons[value] = b
        h.addStretch(1)

    def _click(self, v):
        self.set_value(v)
        self.selected.emit(v)

    def set_value(self, v):
        for val, b in self.buttons.items():
            b.setChecked(val == v)


def make_combo(items, current=None):
    cb = QComboBox()
    cb.setView(QListView())
    cb.setCursor(Qt.PointingHandCursor)
    for label, value in items:
        cb.addItem(label, value)
    set_combo(cb, current)
    return cb


def set_combo(cb, value):
    cb.blockSignals(True)
    for i in range(cb.count()):
        if cb.itemData(i) == value:
            cb.setCurrentIndex(i)
            break
    cb.blockSignals(False)


def rounded_pixmap(src: QPixmap, w: int, h: int, radius: float, play: bool = True) -> QPixmap:
    dpr = 2.0
    out = QPixmap(int(w * dpr), int(h * dpr))
    out.setDevicePixelRatio(dpr)
    out.fill(Qt.transparent)
    p = QPainter(out)
    p.setRenderHint(QPainter.Antialiasing)
    p.setRenderHint(QPainter.SmoothPixmapTransform)
    path = QPainterPath()
    path.addRoundedRect(QRectF(0, 0, w, h), radius, radius)
    p.setClipPath(path)
    if src is not None and not src.isNull():
        scaled = src.scaled(int(w * dpr), int(h * dpr), Qt.KeepAspectRatioByExpanding, Qt.SmoothTransformation)
        scaled.setDevicePixelRatio(dpr)
        sx = (scaled.width() / dpr - w) / 2
        sy = (scaled.height() / dpr - h) / 2
        p.drawPixmap(QRectF(-sx, -sy, scaled.width() / dpr, scaled.height() / dpr), scaled,
                     QRectF(0, 0, scaled.width(), scaled.height()))
    else:
        p.fillRect(QRectF(0, 0, w, h), QColor("#242424"))
        theme.draw_rewind_glyph(p, w / 2, h / 2, min(w, h) * 0.18, QColor("#666666"))
    if not play:
        p.end()
        return out
    # play badge
    p.setClipping(False)
    r = min(w, h) * 0.13
    cx, cy = w / 2, h / 2
    p.setPen(Qt.NoPen)
    p.setBrush(QColor(0, 0, 0, 110))
    p.drawEllipse(QRectF(cx - r, cy - r, 2 * r, 2 * r))
    tri = QPainterPath()
    tri.moveTo(cx - r * 0.32, cy - r * 0.45)
    tri.lineTo(cx + r * 0.5, cy)
    tri.lineTo(cx - r * 0.32, cy + r * 0.45)
    tri.closeSubpath()
    p.setBrush(QColor(255, 255, 255, 230))
    p.drawPath(tri)
    p.end()
    return out


class ClipCard(QFrame):
    clicked = Signal(str)

    def __init__(self, path, pixmap, name, meta, w=196, play=True, parent=None):
        super().__init__(parent)
        self.setObjectName("ClipCard")
        self.path = path
        self.setCursor(Qt.PointingHandCursor)
        v = QVBoxLayout(self)
        v.setContentsMargins(8, 8, 8, 10)
        v.setSpacing(6)
        img = QLabel()
        th = int(w * 9 / 16)
        img.setFixedSize(w, th)
        img.setPixmap(rounded_pixmap(pixmap, w, th, 8, play))
        v.addWidget(img)
        n = QLabel(name)
        n.setObjectName("ClipName")
        m = QLabel(meta)
        m.setObjectName("ClipMeta")
        v.addWidget(n)
        v.addWidget(m)
        self.setSizePolicy(QSizePolicy.Fixed, QSizePolicy.Fixed)

    def mouseReleaseEvent(self, e):
        if e.button() == Qt.LeftButton:
            self.clicked.emit(self.path)
