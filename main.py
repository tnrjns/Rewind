"""Rewind — instant replay for your screen."""
import signal
import sys

from PySide6.QtCore import Qt, QTimer
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWidgets import QApplication

from app import theme, winapi
from app.settings import sub_dir

SERVER_NAME = "RewindInstantReplay-SingleInstance"


def main():
    winapi.set_app_id()
    QApplication.setHighDpiScaleFactorRoundingPolicy(Qt.HighDpiScaleFactorRoundingPolicy.PassThrough)
    app = QApplication(sys.argv)
    app.setApplicationName("Rewind")
    app.setQuitOnLastWindowClosed(False)

    # single instance: if Rewind is already running, bring it to the front instead
    sock = QLocalSocket()
    sock.connectToServer(SERVER_NAME)
    if sock.waitForConnected(300):
        sock.write(b"show")
        sock.flush()
        sock.waitForBytesWritten(300)
        return 0
    QLocalServer.removeServer(SERVER_NAME)
    server = QLocalServer()
    server.listen(SERVER_NAME)

    app.setStyle("Fusion")
    app.setFont(theme.ui_font())
    app.setStyleSheet(theme.stylesheet(theme.chevron_file(sub_dir("ui"))))
    app.setWindowIcon(theme.app_icon())

    from app.controller import Controller
    ctrl = Controller(app, autostart="--autostart" in sys.argv)
    server.newConnection.connect(lambda: (server.nextPendingConnection(), ctrl.show_window()))
    app.aboutToQuit.connect(ctrl.recorder.shutdown)
    signal.signal(signal.SIGTERM, lambda *_: app.quit())
    signal.signal(signal.SIGINT, lambda *_: app.quit())
    wake = QTimer()  # lets Python run signal handlers while Qt's loop is idle
    wake.timeout.connect(lambda: None)
    wake.start(500)
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
