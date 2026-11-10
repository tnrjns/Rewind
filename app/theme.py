"""Colours, stylesheet and the drawn logo."""
import math
import os

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import (QColor, QFont, QFontDatabase, QIcon, QPainter, QPainterPath,
                           QPen, QPixmap)

ACCENT = "#D8D8D8"
ACCENT_2 = "#FFFFFF"
BG = "#111111"
SIDEBAR = "#171717"
CARD = "#1D1D1D"
CARD_2 = "#242424"
FIELD = "#292929"
BORDER = "#353535"
TEXT = "#F2F2F2"
MUTED = "#A3A3A3"
SUBTLE = "#747474"
GREEN = "#65C98A"
WARN = "#E5B968"


def ui_font_families():
    fams = set(QFontDatabase.families())
    out = [f for f in ("Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI", "Inter") if f in fams]
    return out or [QFontDatabase.systemFont(QFontDatabase.GeneralFont).family()]


def icon_font_family():
    fams = set(QFontDatabase.families())
    for f in ("Segoe Fluent Icons", "Segoe MDL2 Assets"):
        if f in fams:
            return f
    return None


def mix(a: QColor, b: QColor, t: float) -> QColor:
    return QColor(int(a.red() + (b.red() - a.red()) * t), int(a.green() + (b.green() - a.green()) * t),
                  int(a.blue() + (b.blue() - a.blue()) * t), int(a.alpha() + (b.alpha() - a.alpha()) * t))


def draw_rewind_glyph(p: QPainter, cx: float, cy: float, size: float, color=Qt.white):
    """Two left-pointing triangles (◀◀) centred on cx, cy."""
    p.save()
    p.setPen(Qt.NoPen)
    p.setBrush(QColor(color))
    h = size
    w = size * 0.62
    x0 = cx - w + w * 0.08
    for dx in (0, w * 0.92):
        path = QPainterPath()
        path.moveTo(x0 + dx, cy)
        path.lineTo(x0 + dx + w, cy - h / 2)
        path.lineTo(x0 + dx + w, cy + h / 2)
        path.closeSubpath()
        p.drawPath(path)
    p.restore()


def draw_logo(size: int, dim: bool = False) -> QPixmap:
    pm = QPixmap(size, size)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    c = size / 2
    base = "#303030" if dim else "#171717"
    mark = "#E5E5E5" if dim else "#FFFFFF"
    p.setPen(Qt.NoPen)
    p.setBrush(QColor(base))
    r = size * 0.24
    p.drawRoundedRect(QRectF(0, 0, size, size), r, r)
    p.setPen(QPen(QColor("#777777"), max(1.0, size * 0.025)))
    p.setBrush(Qt.NoBrush)
    p.drawRoundedRect(QRectF(0.5, 0.5, size - 1, size - 1), r, r)

    rad = size * 0.31
    pen = QPen(QColor(mark), max(1.5, size * 0.085))
    pen.setCapStyle(Qt.RoundCap)
    p.setPen(pen)
    start, span = 125, 290
    p.drawArc(QRectF(c - rad, c - rad, 2 * rad, 2 * rad), int(start * 16), int(span * 16))
    end = math.radians(start + span)
    ex, ey = c + rad * math.cos(end), c - rad * math.sin(end)
    tx, ty = -math.sin(end), -math.cos(end)
    nx, ny = -ty, tx
    L, W = size * 0.13, size * 0.11
    head = QPainterPath()
    head.moveTo(ex + tx * L, ey + ty * L)
    head.lineTo(ex + nx * W, ey + ny * W)
    head.lineTo(ex - nx * W, ey - ny * W)
    head.closeSubpath()
    p.setPen(Qt.NoPen)
    p.setBrush(QColor(mark))
    p.drawPath(head)
    draw_rewind_glyph(p, c, c, size * 0.30, QColor(mark))
    p.end()
    return pm


def app_icon(dim=False) -> QIcon:
    ic = QIcon()
    for s in (16, 20, 24, 32, 40, 48, 64, 128, 256):
        ic.addPixmap(draw_logo(s, dim))
    return ic


def chevron_file(folder: str) -> str:
    path = os.path.join(folder, "chevron.png")
    if not os.path.exists(path):
        pm = QPixmap(24, 24)
        pm.fill(Qt.transparent)
        p = QPainter(pm)
        p.setRenderHint(QPainter.Antialiasing)
        pen = QPen(QColor(MUTED), 2.2)
        pen.setCapStyle(Qt.RoundCap)
        pen.setJoinStyle(Qt.RoundJoin)
        p.setPen(pen)
        p.drawPolyline([QPointF(7, 10), QPointF(12, 15), QPointF(17, 10)])
        p.end()
        pm.save(path)
    return path.replace("\\", "/")


def stylesheet(chevron: str) -> str:
    return f"""
* {{ outline: 0; }}
QWidget {{ color: {TEXT}; font-size: 13px; }}
QMainWindow, #Root {{ background: {BG}; }}
QToolTip {{ background: {CARD_2}; color: {TEXT}; border: 1px solid {BORDER}; padding: 6px 8px; border-radius: 6px; }}

#Sidebar {{ background: {SIDEBAR}; border-right: 1px solid {BORDER}; }}
#Brand {{ font-size: 18px; font-weight: 650; letter-spacing: 0.1px; }}
#BrandSub {{ color: {MUTED}; font-size: 11px; }}
#NavButton {{ text-align: left; padding: 9px 12px; border: none; border-radius: 6px; color: {MUTED};
              background: transparent; font-size: 13px; }}
#NavButton:hover {{ background: #242424; color: {TEXT}; }}
#NavButton:checked {{ background: #303030; color: {TEXT}; font-weight: 600; }}
#SideStatus {{ color: {MUTED}; font-size: 12px; }}

#PageTitle {{ font-size: 26px; font-weight: 700; }}
#PageSubtitle {{ color: {MUTED}; font-size: 13px; }}
#Card {{ background: {CARD}; border: 1px solid {BORDER}; border-radius: 14px; }}
#CardTitle {{ color: {SUBTLE}; font-size: 11px; font-weight: 700; letter-spacing: 1.2px; }}
#Sep {{ background: {BORDER}; border: none; }}
#RowTitle {{ font-size: 13.5px; font-weight: 600; }}
#RowSub {{ color: {MUTED}; font-size: 12px; }}
#Value {{ color: {TEXT}; font-weight: 600; }}
#Muted {{ color: {MUTED}; }}
#Warn {{ color: {WARN}; }}
#ErrorBanner {{ background: rgba(255,181,71,0.08); border: 1px solid rgba(255,181,71,0.35); border-radius: 10px;
                color: {WARN}; padding: 10px 12px; }}

#Hero {{ border: 1px solid {BORDER}; border-radius: 10px; background: {CARD}; }}
#HeroState {{ font-size: 30px; font-weight: 650; }}
#Tile {{ background: #242424; border: 1px solid {BORDER}; border-radius: 8px; }}
#TileLabel {{ color: {MUTED}; font-size: 11px; font-weight: 650; letter-spacing: 0.7px; }}
#TileValue {{ font-size: 15px; font-weight: 600; }}

QPushButton {{ background: {FIELD}; border: 1px solid {BORDER}; border-radius: 6px; padding: 9px 16px; }}
QPushButton:hover {{ background: #383838; border-color: #555555; }}
QPushButton:pressed {{ background: #202020; }}
QPushButton:disabled {{ color: {SUBTLE}; }}
QPushButton#Primary {{ border: none; color: #111111; font-weight: 700; padding: 12px 20px;
    background: {ACCENT}; }}
QPushButton#Primary:hover {{ background: #FFFFFF; }}
QPushButton#Primary:pressed {{ background: #BDBDBD; }}
QPushButton#Primary:disabled {{ background: #3A3A3A; color: {SUBTLE}; }}
QPushButton#Chip {{ background: {FIELD}; border: 1px solid {BORDER}; border-radius: 15px; padding: 6px 14px;
                    color: {MUTED}; font-weight: 600; }}
QPushButton#Chip:hover {{ color: {TEXT}; border-color: #666666; }}
QPushButton#Chip:checked {{ background: #3A3A3A; border: 1px solid #8A8A8A; color: {TEXT}; }}
QPushButton#HotkeyEdit {{ min-width: 190px; font-weight: 700; letter-spacing: 0.4px; padding: 9px 14px; }}
QPushButton#HotkeyEdit[capturing="true"] {{ border: 1px solid {ACCENT}; color: {ACCENT}; background: #303030; }}
QPushButton#Link {{ background: transparent; border: none; color: {MUTED}; padding: 4px 6px; }}
QPushButton#Link:hover {{ color: {TEXT}; }}

QSlider {{ min-height: 22px; }}
QSlider::groove:horizontal {{ height: 4px; background: #383838; border-radius: 2px; }}
QSlider::sub-page:horizontal {{ height: 4px; border-radius: 2px; background: {ACCENT}; }}
QSlider::handle:horizontal {{ background: white; width: 16px; height: 16px; margin: -6px 0; border-radius: 8px; }}
QSlider::handle:horizontal:hover {{ background: #FFFFFF; }}
QSlider::sub-page:horizontal:disabled {{ background: #555555; }}

QComboBox {{ background: {FIELD}; border: 1px solid {BORDER}; border-radius: 6px; padding: 7px 12px; min-width: 190px; }}
QComboBox:hover {{ border-color: #666666; }}
QComboBox::drop-down {{ border: none; width: 28px; }}
QComboBox::down-arrow {{ image: url({chevron}); width: 14px; height: 14px; }}
QComboBox QAbstractItemView {{ background: {CARD_2}; border: 1px solid {BORDER}; border-radius: 6px; padding: 4px;
                               selection-background-color: #3A3A3A; selection-color: {TEXT}; }}
QComboBox QAbstractItemView::item {{ min-height: 28px; padding: 0 8px; border-radius: 4px; }}
QSpinBox, QLineEdit {{ background: {FIELD}; border: 1px solid {BORDER}; border-radius: 6px; padding: 7px 10px; }}
QSpinBox::up-button, QSpinBox::down-button {{ width: 0; border: none; }}

QScrollArea, #PageBody {{ background: transparent; border: none; }}
QScrollBar:vertical {{ background: transparent; width: 10px; margin: 6px 2px; }}
QScrollBar::handle:vertical {{ background: #3A3A3A; border-radius: 3px; min-height: 40px; }}
QScrollBar::handle:vertical:hover {{ background: #555555; }}
QScrollBar::add-line, QScrollBar::sub-line, QScrollBar::add-page, QScrollBar::sub-page {{ height: 0; background: none; }}

QProgressBar#Buffer {{ background: #383838; border: none; border-radius: 3px; max-height: 6px; min-height: 6px; }}
QProgressBar#Buffer::chunk {{ border-radius: 3px; background: {ACCENT}; }}

QMenu {{ background: {CARD_2}; border: 1px solid {BORDER}; padding: 6px; border-radius: 6px; }}
QMenu::item {{ padding: 7px 22px 7px 14px; border-radius: 4px; }}
QMenu::item:selected {{ background: #3A3A3A; }}
QMenu::separator {{ height: 1px; background: {BORDER}; margin: 5px 8px; }}

QListWidget#ClipGrid {{ background: transparent; border: none; }}
QListWidget#ClipGrid::item {{ border-radius: 8px; padding: 8px; color: {TEXT}; }}
QListWidget#ClipGrid::item:hover {{ background: #252525; }}
QListWidget#ClipGrid::item:selected {{ background: #353535; }}
#ClipCard {{ background: {CARD}; border: 1px solid {BORDER}; border-radius: 8px; }}
#ClipCard:hover {{ border-color: #777777; }}
#ClipName {{ font-size: 12px; font-weight: 600; }}
#ClipMeta {{ font-size: 11px; color: {MUTED}; }}
#Preview {{ background: #080808; border: 1px solid {BORDER}; border-radius: 4px; color: {MUTED}; }}
"""


def ui_font() -> QFont:
    f = QFont()
    f.setFamilies(ui_font_families())
    f.setPixelSize(13)
    f.setHintingPreference(QFont.PreferNoHinting)
    return f
