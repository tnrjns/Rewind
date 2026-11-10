# PyInstaller spec for the macOS app bundle. Build with: ./build_macos.sh
import re

APP_VERSION = re.search(r'APP_VERSION = "([^"]+)"', open("app/settings.py").read()).group(1)

QT_EXCLUDES = ["PySide6.QtQml", "PySide6.QtQuick", "PySide6.QtQuickWidgets", "PySide6.QtWebEngineCore",
               "PySide6.QtWebEngineWidgets", "PySide6.QtWebChannel", "PySide6.QtPdf", "PySide6.QtCharts",
               "PySide6.QtMultimedia", "PySide6.Qt3DCore", "PySide6.QtDataVisualization", "PySide6.QtSql",
               "PySide6.QtTest", "PySide6.QtDesigner", "PySide6.QtOpenGL", "PySide6.QtSvg", "tkinter", "numpy"]

a = Analysis(
    ["main.py"],
    binaries=[("bin/rewind-audio", "bin")],
    datas=[("assets/rewind.icns", "assets")],
    excludes=QT_EXCLUDES,
)
DROP = ("QtQml", "QtQuick", "QtPdf", "QtVirtualKeyboard", "QtOpenGL", "QtSvg", "qpdf", "qsvg",
        "virtualkeyboard", "qtvirtualkeyboard", "/translations/")
a.binaries = [b for b in a.binaries if not any(d in b[0] for d in DROP)]
a.datas = [d for d in a.datas if not any(x in d[0] for x in DROP)]
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, [], exclude_binaries=True, name="Rewind", console=False, upx=False,
          target_arch="arm64")
coll = COLLECT(exe, a.binaries, a.datas, name="Rewind", upx=False)
app = BUNDLE(
    coll,
    name="Rewind.app",
    icon="assets/rewind.icns",
    bundle_identifier="com.rewind.instantreplay",
    version=APP_VERSION,
    info_plist={
        "CFBundleShortVersionString": APP_VERSION,
        "CFBundleVersion": APP_VERSION,
        "LSMinimumSystemVersion": "13.0",
        "LSApplicationCategoryType": "public.app-category.video",
        "NSHighResolutionCapable": True,
        "NSMicrophoneUsageDescription": "Rewind records your microphone into clips when you turn it on.",
        "NSHumanReadableCopyright": "Copyright 2026 Tanner Jensen and Colin Jaxon",
    },
)
