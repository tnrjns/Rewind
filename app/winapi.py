"""Small OS helpers (Win32, plus macOS where it has an equivalent). Safe no-ops elsewhere."""
import os
import subprocess
import sys

IS_WIN = sys.platform == "win32"
IS_MAC = sys.platform == "darwin"
CREATE_NO_WINDOW = 0x08000000
LAUNCH_AGENT = os.path.join(os.path.expanduser("~"), "Library", "LaunchAgents", "com.rewind.instantreplay.plist")

if IS_WIN:
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.WinDLL("user32", use_last_error=True)
    dwmapi = ctypes.WinDLL("dwmapi")

if IS_MAC:
    import ctypes

    _objc = ctypes.CDLL("/usr/lib/libobjc.A.dylib")
    _objc.objc_getClass.restype = ctypes.c_void_p
    _objc.sel_registerName.restype = ctypes.c_void_p
    ctypes.CDLL("/System/Library/Frameworks/AppKit.framework/AppKit")

    def _msg(obj, sel, *args, restype=ctypes.c_void_p, argtypes=()):
        f = ctypes.CFUNCTYPE(restype, ctypes.c_void_p, ctypes.c_void_p, *argtypes)(("objc_msgSend", _objc))
        return f(obj, _objc.sel_registerName(sel.encode()), *args)

    def _ns_window(view: int):
        return _msg(ctypes.c_void_p(view), "window") if view else None


def popen_kwargs() -> dict:
    return {"creationflags": CREATE_NO_WINDOW} if IS_WIN else {}


def set_app_id(app_id: str = "Rewind.InstantReplay") -> None:
    if not IS_WIN:
        return
    try:
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID(app_id)
    except Exception:
        pass


def style_titlebar(hwnd: int, caption_rgb=None) -> None:
    """Dark title bar + matching caption colour on Windows 11."""
    if not IS_WIN:
        return
    try:
        on = ctypes.c_int(1)
        for attr in (20, 19):  # DWMWA_USE_IMMERSIVE_DARK_MODE (new / old id)
            if dwmapi.DwmSetWindowAttribute(wintypes.HWND(hwnd), attr, ctypes.byref(on), ctypes.sizeof(on)) == 0:
                break
        if caption_rgb:
            r, g, b = caption_rgb
            color = ctypes.c_uint(r | (g << 8) | (b << 16))
            dwmapi.DwmSetWindowAttribute(wintypes.HWND(hwnd), 35, ctypes.byref(color), 4)  # DWMWA_CAPTION_COLOR
            border = ctypes.c_uint(0x00442925)
            dwmapi.DwmSetWindowAttribute(wintypes.HWND(hwnd), 34, ctypes.byref(border), 4)  # DWMWA_BORDER_COLOR
    except Exception:
        pass


def screen_capture_allowed(ask: bool = False) -> bool:
    """macOS Screen Recording permission. With ask=True, macOS shows its prompt the first time."""
    if not IS_MAC:
        return True
    try:
        cg = ctypes.CDLL("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
        cg.CGPreflightScreenCaptureAccess.restype = ctypes.c_bool
        cg.CGRequestScreenCaptureAccess.restype = ctypes.c_bool
        if cg.CGPreflightScreenCaptureAccess():
            return True
        return bool(cg.CGRequestScreenCaptureAccess()) if ask else False
    except Exception:
        return True


def open_privacy_settings() -> None:
    if IS_MAC:
        subprocess.Popen(["open", "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"])


def exclude_from_capture(hwnd: int) -> None:
    """Hide a window from screen capture (so the overlay never shows up in clips)."""
    if IS_MAC:
        try:
            win = _ns_window(hwnd)
            if win:
                _msg(win, "setSharingType:", 0, restype=None, argtypes=(ctypes.c_ulong,))  # NSWindowSharingNone
                # show on every Space and over full-screen apps
                _msg(win, "setCollectionBehavior:", (1 << 0) | (1 << 8), restype=None, argtypes=(ctypes.c_ulong,))
        except Exception:
            pass
        return
    if not IS_WIN:
        return
    try:
        user32.SetWindowDisplayAffinity(wintypes.HWND(hwnd), 0x11)  # WDA_EXCLUDEFROMCAPTURE
    except Exception:
        pass


def make_topmost(hwnd: int) -> None:
    if not IS_WIN:
        return
    try:
        # HWND_TOPMOST, SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
        user32.SetWindowPos(wintypes.HWND(hwnd), wintypes.HWND(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010)
    except Exception:
        pass


def list_monitors() -> list:
    """Monitors in physical pixels, in the same order the screen grabber numbers them."""
    if IS_MAC:
        mons = []
        try:
            from PySide6.QtGui import QGuiApplication
            for i, scr in enumerate(QGuiApplication.screens()):
                g, dpr = scr.geometry(), scr.devicePixelRatio()
                mons.append({"name": f"Display {i + 1}", "x": int(g.x() * dpr), "y": int(g.y() * dpr),
                             "w": int(g.width() * dpr) // 2 * 2, "h": int(g.height() * dpr) // 2 * 2,
                             "primary": i == 0})
        except Exception:
            pass
        return mons or [{"name": "Display 1", "x": 0, "y": 0, "w": 1920, "h": 1080, "primary": True}]
    if not IS_WIN:
        return [{"name": "Display 1", "x": 0, "y": 0, "w": 1920, "h": 1080, "primary": True}]
    mons = []
    try:
        class RECT(ctypes.Structure):
            _fields_ = [("left", ctypes.c_long), ("top", ctypes.c_long),
                        ("right", ctypes.c_long), ("bottom", ctypes.c_long)]

        class MONITORINFOEXW(ctypes.Structure):
            _fields_ = [("cbSize", wintypes.DWORD), ("rcMonitor", RECT), ("rcWork", RECT),
                        ("dwFlags", wintypes.DWORD), ("szDevice", wintypes.WCHAR * 32)]

        proc_t = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HMONITOR, wintypes.HDC,
                                    ctypes.POINTER(RECT), wintypes.LPARAM)

        def cb(hmon, hdc, lprc, lparam):
            info = MONITORINFOEXW()
            info.cbSize = ctypes.sizeof(MONITORINFOEXW)
            if user32.GetMonitorInfoW(hmon, ctypes.byref(info)):
                r = info.rcMonitor
                mons.append({"device": info.szDevice, "x": r.left, "y": r.top,
                             "w": r.right - r.left, "h": r.bottom - r.top,
                             "primary": bool(info.dwFlags & 1)})
            return True

        user32.EnumDisplayMonitors(None, None, proc_t(cb), 0)
        mons.sort(key=lambda m: m.get("device", ""))
        for i, m in enumerate(mons):
            m["name"] = f"Display {i + 1}"
    except Exception:
        pass
    if not mons:
        mons = [{"name": "Display 1", "x": 0, "y": 0, "w": 1920, "h": 1080, "primary": True}]
    return mons


RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def startup_command() -> str:
    if getattr(sys, "frozen", False):
        return f'"{sys.executable}" --autostart'
    exe = sys.executable
    pyw = os.path.join(os.path.dirname(exe), "pythonw.exe")
    if os.path.exists(pyw):
        exe = pyw
    main = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "main.py")
    return f'"{exe}" "{main}" --autostart'


OPEN_HOTKEY = ""


def bring_to_front(hwnd: int) -> None:
    """Restore + focus a window even when another app (e.g. a game) is in front."""
    if IS_MAC:
        try:
            app = _msg(_objc.objc_getClass(b"NSApplication"), "sharedApplication")
            _msg(app, "activateIgnoringOtherApps:", True, restype=None, argtypes=(ctypes.c_bool,))
        except Exception:
            pass
        return
    if not IS_WIN:
        return
    try:
        user32.ShowWindow(wintypes.HWND(hwnd), 9)  # SW_RESTORE
        # a tiny Alt tap lifts Windows' focus-stealing lock so SetForegroundWindow works
        user32.keybd_event(0x12, 0, 0, 0)
        user32.keybd_event(0x12, 0, 2, 0)
        user32.SetForegroundWindow(wintypes.HWND(hwnd))
    except Exception:
        pass


def create_shortcuts(hotkey: str = OPEN_HOTKEY) -> bool:
    """Desktop + Start-menu shortcuts. The desktop one also gets a Windows shortcut key,
    so the key opens Rewind even when it isn't running."""
    if not IS_WIN:
        return False
    base = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    if getattr(sys, "frozen", False):
        target, args, icon, workdir = sys.executable, "", sys.executable, os.path.dirname(sys.executable)
    else:
        target = sys.executable
        pyw = os.path.join(os.path.dirname(target), "pythonw.exe")
        if os.path.exists(pyw):
            target = pyw
        args = '"' + os.path.join(base, "main.py") + '"'
        icon = os.path.join(base, "assets", "rewind.ico")
        workdir = base

    def q(v):  # PowerShell single-quoted literal
        return "'" + str(v).replace("'", "''") + "'"

    ps = (
        "$s = New-Object -ComObject WScript.Shell;"
        "$desk = [Environment]::GetFolderPath('Desktop');"
        "$menu = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs';"
        "foreach ($dir in @($desk, $menu)) {"
        "  $l = $s.CreateShortcut((Join-Path $dir 'Rewind.lnk'));"
        f"  $l.TargetPath = {q(target)}; $l.Arguments = {q(args)}; $l.WorkingDirectory = {q(workdir)};"
        f"  $l.IconLocation = {q(icon)}; $l.Description = 'Open Rewind instant replay';"
        f"  if ($dir -eq $desk) {{ try {{ $l.Hotkey = {q(hotkey)} }} catch {{}} }};"
        "  $l.Save() }"
    )
    try:
        r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps],
                           capture_output=True, timeout=30, **popen_kwargs())
        return r.returncode == 0
    except Exception:
        return False


def _launch_args() -> list:
    if getattr(sys, "frozen", False):
        return [sys.executable, "--autostart"]
    main = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "main.py")
    return [sys.executable, main, "--autostart"]


def _set_launch_agent(enabled: bool) -> bool:
    import plistlib
    try:
        if not enabled:
            if os.path.exists(LAUNCH_AGENT):
                os.remove(LAUNCH_AGENT)
            return True
        os.makedirs(os.path.dirname(LAUNCH_AGENT), exist_ok=True)
        with open(LAUNCH_AGENT, "wb") as f:
            plistlib.dump({"Label": "com.rewind.instantreplay", "ProgramArguments": _launch_args(),
                           "RunAtLoad": True, "ProcessType": "Interactive"}, f)
        return True
    except OSError:
        return False


def set_run_at_startup(enabled: bool) -> bool:
    if IS_MAC:
        return _set_launch_agent(enabled)
    if not IS_WIN:
        return False
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
            if enabled:
                winreg.SetValueEx(k, "Rewind", 0, winreg.REG_SZ, startup_command())
            else:
                try:
                    winreg.DeleteValue(k, "Rewind")
                except FileNotFoundError:
                    pass
        return True
    except Exception:
        return False


def open_path(path: str) -> None:
    try:
        if IS_WIN:
            os.startfile(path)  # noqa
        elif IS_MAC:
            subprocess.Popen(["open", path])
        else:
            subprocess.Popen(["xdg-open", path])
    except Exception:
        pass


def reveal_in_explorer(path: str) -> None:
    try:
        if IS_WIN:
            subprocess.Popen(f'explorer /select,"{os.path.normpath(path)}"')
        elif IS_MAC:
            subprocess.Popen(["open", "-R", path])
        else:
            subprocess.Popen(["xdg-open", os.path.dirname(path)])
    except Exception:
        pass
