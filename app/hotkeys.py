"""Global hotkeys via Win32 RegisterHotKey (no keyboard hooks, no admin needed)."""
import sys
import threading

from PySide6.QtCore import QObject, Signal

IS_WIN = sys.platform == "win32"

MOD_ALT = 0x1
MOD_CONTROL = 0x2
MOD_SHIFT = 0x4
MOD_WIN = 0x8
MOD_NOREPEAT = 0x4000
WM_HOTKEY = 0x0312
WM_QUIT = 0x0012


class HotkeyManager(QObject):
    triggered = Signal(str)

    def __init__(self):
        super().__init__()
        self._thread = None
        self._tid = None

    def set_hotkeys(self, mapping: dict) -> dict:
        """mapping: name -> (mods, vk). Returns name -> True/False/None (None = unset)."""
        self.stop()
        if not IS_WIN:
            return {k: (True if v[1] else None) for k, v in mapping.items()}
        results: dict = {}
        ready = threading.Event()
        self._thread = threading.Thread(target=self._run, args=(dict(mapping), results, ready), daemon=True)
        self._thread.start()
        ready.wait(3)
        return dict(results)

    def _run(self, mapping, results, ready):
        import ctypes
        from ctypes import wintypes
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        kernel32 = ctypes.WinDLL("kernel32")
        user32.RegisterHotKey.argtypes = [wintypes.HWND, ctypes.c_int, wintypes.UINT, wintypes.UINT]
        user32.UnregisterHotKey.argtypes = [wintypes.HWND, ctypes.c_int]
        user32.GetMessageW.argtypes = [ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT, wintypes.UINT]
        user32.PeekMessageW.argtypes = [ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT,
                                        wintypes.UINT, wintypes.UINT]
        msg = wintypes.MSG()
        user32.PeekMessageW(ctypes.byref(msg), None, 0, 0, 0)  # make sure the thread has a queue
        self._tid = kernel32.GetCurrentThreadId()
        ids = {}
        for i, (name, (mods, vk)) in enumerate(mapping.items(), start=1):
            if not vk:
                results[name] = None
                continue
            ok = user32.RegisterHotKey(None, i, int(mods) | MOD_NOREPEAT, int(vk))
            results[name] = bool(ok)
            if ok:
                ids[i] = name
        ready.set()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            if msg.message == WM_HOTKEY:
                name = ids.get(int(msg.wParam))
                if name:
                    self.triggered.emit(name)
        for i in ids:
            user32.UnregisterHotKey(None, i)

    def stop(self):
        if IS_WIN and self._thread is not None and self._tid:
            import ctypes
            from ctypes import wintypes
            user32 = ctypes.WinDLL("user32")
            user32.PostThreadMessageW.argtypes = [wintypes.DWORD, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
            user32.PostThreadMessageW(self._tid, WM_QUIT, 0, 0)
            self._thread.join(1.5)
        self._thread = None
        self._tid = None
