"""Checks GitHub for a newer Rewind release and installs it in place."""
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
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

    # ------------------------------------------------------------ install
    progress = Signal(str)       # human-readable step
    install_failed = Signal(str)
    ready_to_restart = Signal()

    def can_install(self) -> bool:
        """In-place updates only work for the packaged app, not when running from source."""
        if not getattr(sys, "frozen", False) or not self.url.endswith((".dmg", ".exe")):
            return False
        return self._mac_bundle() is not None if sys.platform == "darwin" else sys.platform == "win32"

    def install(self):
        if self._busy:
            return
        self._busy = True
        threading.Thread(target=self._install, daemon=True).start()

    @staticmethod
    def _mac_bundle():
        exe = os.path.realpath(sys.executable)
        bundle = os.path.dirname(os.path.dirname(os.path.dirname(exe)))
        return bundle if bundle.endswith(".app") else None

    def _download(self, dest):
        req = urllib.request.Request(self.url, headers={"User-Agent": f"Rewind/{APP_VERSION}"})
        with urllib.request.urlopen(req, timeout=30) as r, open(dest, "wb") as f:
            total = int(r.headers.get("Content-Length") or 0)
            done, last = 0, -1
            while True:
                chunk = r.read(1 << 16)
                if not chunk:
                    break
                f.write(chunk)
                done += len(chunk)
                pct = int(done * 100 / total) if total else -1
                if pct != last and (pct % 5 == 0 or pct < 0):
                    last = pct
                    self.progress.emit(f"Downloading Rewind {self.version}… {pct}%" if pct >= 0 else
                                       f"Downloading Rewind {self.version}…")

    def _install(self):
        work = tempfile.mkdtemp(prefix="rewind-update-")
        try:
            if sys.platform == "darwin":
                self._install_mac(work)
            else:
                self._install_windows(work)
            self.ready_to_restart.emit()
        except Exception as e:
            shutil.rmtree(work, ignore_errors=True)
            self.install_failed.emit(f"Update failed: {e}. You can still download it from rewindtake.com.")
        finally:
            self._busy = False

    def _install_mac(self, work):
        target = self._mac_bundle()
        dmg = os.path.join(work, "Rewind.dmg")
        self._download(dmg)
        self.progress.emit("Unpacking…")
        mnt = os.path.join(work, "mnt")
        os.makedirs(mnt)
        subprocess.run(["hdiutil", "attach", dmg, "-nobrowse", "-readonly", "-quiet", "-mountpoint", mnt],
                       check=True, timeout=120)
        stage = os.path.join(work, "Rewind.app")
        try:
            src = os.path.join(mnt, "Rewind.app")
            if not os.path.isdir(src):
                raise RuntimeError("the download didn't contain Rewind.app")
            subprocess.run(["ditto", src, stage], check=True, timeout=300)
        finally:
            subprocess.run(["hdiutil", "detach", mnt, "-quiet", "-force"], timeout=60)
        q = shlex.quote
        script = os.path.join(work, "swap.sh")
        with open(script, "w") as f:
            f.write(f"""#!/bin/sh
while kill -0 {os.getpid()} 2>/dev/null; do sleep 0.2; done
rm -rf {q(target)}.old
if mv {q(target)} {q(target)}.old && ditto {q(stage)} {q(target)}; then
  rm -rf {q(target)}.old
else
  rm -rf {q(target)}; mv {q(target)}.old {q(target)}
fi
xattr -dr com.apple.quarantine {q(target)} 2>/dev/null
open {q(target)}
rm -rf {q(work)}
""")
        subprocess.Popen(["/bin/sh", script], start_new_session=True, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def _install_windows(self, work):
        exe = os.path.join(work, "Installer.exe")
        self._download(exe)
        self.progress.emit("Installing…")
        flags = getattr(subprocess, "DETACHED_PROCESS", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        subprocess.Popen([exe, "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"],
                         creationflags=flags, close_fds=True)
