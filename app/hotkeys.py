"""Global hotkeys without keyboard hooks or extra permissions.

Windows uses Win32 RegisterHotKey; macOS uses Carbon RegisterEventHotKey.
"""
import sys
import threading

from PySide6.QtCore import QObject, Signal

IS_WIN = sys.platform == "win32"
IS_MAC = sys.platform == "darwin"

MOD_ALT = 0x1
MOD_CONTROL = 0x2
MOD_SHIFT = 0x4
MOD_WIN = 0x8
MOD_NOREPEAT = 0x4000
WM_HOTKEY = 0x0312
WM_QUIT = 0x0012

K_EVENT_HOTKEY_PRESSED = 5
_CARBON_MODS = ((MOD_ALT, 0x0800), (MOD_CONTROL, 0x1000), (MOD_SHIFT, 0x0200), (MOD_WIN, 0x0100))

if IS_MAC:
    import ctypes

    class _EventHotKeyID(ctypes.Structure):
        _fields_ = [("signature", ctypes.c_uint32), ("id", ctypes.c_uint32)]

    class _EventTypeSpec(ctypes.Structure):
        _fields_ = [("eventClass", ctypes.c_uint32), ("eventKind", ctypes.c_uint32)]

    _HANDLER_T = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p)
    _lib = None

    def _carbon():
        global _lib
        if _lib is None:
            try:
                lib = ctypes.CDLL("/System/Library/Frameworks/Carbon.framework/Carbon")
            except OSError:
                return None
            lib.GetApplicationEventTarget.restype = ctypes.c_void_p
            lib.InstallEventHandler.argtypes = [ctypes.c_void_p, _HANDLER_T, ctypes.c_ulong,
                                                ctypes.POINTER(_EventTypeSpec), ctypes.c_void_p,
                                                ctypes.POINTER(ctypes.c_void_p)]
            lib.RegisterEventHotKey.argtypes = [ctypes.c_uint32, ctypes.c_uint32, _EventHotKeyID, ctypes.c_void_p,
                                                ctypes.c_uint32, ctypes.POINTER(ctypes.c_void_p)]
            lib.RegisterEventHotKey.restype = ctypes.c_int32
            lib.UnregisterEventHotKey.argtypes = [ctypes.c_void_p]
            lib.GetEventParameter.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p,
                                              ctypes.c_ulong, ctypes.c_void_p, ctypes.c_void_p]
            lib.GetEventParameter.restype = ctypes.c_int32
            _lib = lib
        return _lib


def _fourcc(code: str) -> int:
    return int.from_bytes(code.encode("ascii"), "big")


def _carbon_mods(mods: int) -> int:
    return sum(carbon for flag, carbon in _CARBON_MODS if mods & flag)


class HotkeyManager(QObject):
    triggered = Signal(str)

    def __init__(self):
        super().__init__()
        self._thread = None
        self._tid = None
        self._mac_handler = None
        self._mac_cb = None
        self._mac_refs: list = []
        self._mac_names: dict = {}

    def set_hotkeys(self, mapping: dict) -> dict:
        """mapping: name -> (mods, vk). Returns name -> True/False/None (None = unset)."""
        self.stop()
        if IS_MAC:
            return self._mac_register(mapping)
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

    def _mac_register(self, mapping):
        carbon = _carbon()
        if carbon is None:
            return {k: (None if v[1] is None else False) for k, v in mapping.items()}
        if self._mac_handler is None:
            spec = _EventTypeSpec(_fourcc("keyb"), K_EVENT_HOTKEY_PRESSED)
            self._mac_cb = _HANDLER_T(self._mac_event)
            ref = ctypes.c_void_p()
            carbon.InstallEventHandler(carbon.GetApplicationEventTarget(), self._mac_cb, 1,
                                       ctypes.byref(spec), None, ctypes.byref(ref))
            self._mac_handler = ref
        results: dict = {}
        self._mac_names = {}
        for i, (name, (mods, key)) in enumerate(mapping.items(), start=1):
            if key is None:
                results[name] = None
                continue
            ref = ctypes.c_void_p()
            hk_id = _EventHotKeyID(_fourcc("RWND"), i)
            err = carbon.RegisterEventHotKey(int(key), _carbon_mods(int(mods)), hk_id,
                                             carbon.GetApplicationEventTarget(), 0, ctypes.byref(ref))
            results[name] = err == 0
            if err == 0:
                self._mac_refs.append(ref)
                self._mac_names[i] = name
        return results

    def _mac_event(self, _call, event, _data):
        hk_id = _EventHotKeyID()
        err = _carbon().GetEventParameter(event, _fourcc("----"), _fourcc("hkid"), None,
                                          ctypes.sizeof(hk_id), None, ctypes.byref(hk_id))
        if err == 0:
            name = self._mac_names.get(hk_id.id)
            if name:
                self.triggered.emit(name)
        return 0

    def stop(self):
        if IS_MAC:
            carbon = _carbon()
            for ref in self._mac_refs:
                carbon.UnregisterEventHotKey(ref)
            self._mac_refs = []
            self._mac_names = {}
            return
        if IS_WIN and self._thread is not None and self._tid:
            import ctypes
            from ctypes import wintypes
            user32 = ctypes.WinDLL("user32")
            user32.PostThreadMessageW.argtypes = [wintypes.DWORD, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
            user32.PostThreadMessageW(self._tid, WM_QUIT, 0, 0)
            self._thread.join(1.5)
        self._thread = None
        self._tid = None
