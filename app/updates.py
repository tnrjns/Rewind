"""Checks GitHub for a newer Rewind release."""
import json
import re
import sys
import threading
import urllib.request

from PySide6.QtCore import QObject, Signal

from .settings import APP_VERSION

REPO = "tnrjns/Rewind"
API = f"https://api.github.com/repos/{REPO}/releases/latest"
RELEASES = f"https://github.com/{REPO}/releases/latest"


def _version(tag: str) -> tuple:
    return tuple(int(n) for n in re.findall(r"\d+", tag)[:3])


def _asset_for_this_os(assets) -> str:
    want = ".dmg" if sys.platform == "darwin" else ".exe" if sys.platform == "win32" else ".AppImage"
    for a in assets:
        if a.get("name", "").endswith(want):
            return a.get("browser_download_url", "")
    return ""


class UpdateChecker(QObject):
    # status: "newer" | "current" | "error"; version; download url
    checked = Signal(str, str, str)

    def __init__(self):
        super().__init__()
        self.status = ""
        self.version = ""
        self.url = ""
        self._busy = False

    def check(self):
        if self._busy:
            return
        self._busy = True
        threading.Thread(target=self._run, daemon=True).start()

    def _run(self):
        try:
            req = urllib.request.Request(API, headers={"Accept": "application/vnd.github+json",
                                                       "User-Agent": f"Rewind/{APP_VERSION}"})
            with urllib.request.urlopen(req, timeout=10) as r:
                data = json.load(r)
            tag = data.get("tag_name", "")
            newer = _version(tag) > _version(APP_VERSION)
            status, version = ("newer" if newer else "current"), tag.lstrip("vV")
            url = (_asset_for_this_os(data.get("assets", [])) or data.get("html_url") or RELEASES) if newer else ""
        except Exception:
            status, version, url = "error", "", ""
        self.status, self.version, self.url = status, version, url
        self._busy = False
        self.checked.emit(status, version, url)
