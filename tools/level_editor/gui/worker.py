"""Background work: rasterising and validating off the UI thread.

Validation rasterises, which is about half a second for a circuit. Half a
second is nothing in a build and an eternity in a mouse-drag loop, so it runs
on its own thread and the result is applied when it arrives.

The thread owns a *latest-request-wins* slot rather than a queue. Dragging a
node fires `changed` sixty times a second, and sixty queued rasterisations is
sixty times the work for one answer -- worse, the answer that arrives last is
the only one anybody reads, so the other fifty-nine were not slow, they were
wasted. A new request replaces the pending one; a request already running is
left to finish and its result dropped if it has since been superseded.
"""

from __future__ import annotations

import threading

from PyQt6.QtCore import QObject, QThread, QTimer, pyqtSignal, pyqtSlot

from lecore.model import Level


class _RasterThread(QThread):
    """Runs one rasterise-and-validate at a time, newest request wins."""

    resultReady = pyqtSignal(object, int)     # (CompileResult | Exception, token)
    busyChanged = pyqtSignal(bool)

    def __init__(self, parent=None):
        super().__init__(parent)
        self._lock = threading.Lock()
        self._pending: tuple[Level, int] | None = None
        self._stop = False
        self._wake = threading.Event()

    def submit(self, level: Level, token: int) -> None:
        with self._lock:
            self._pending = (level, token)
        self._wake.set()

    def stop(self) -> None:
        self._stop = True
        self._wake.set()

    @pyqtSlot()
    def run(self) -> None:
        while not self._stop:
            self._wake.wait(0.25)
            self._wake.clear()
            with self._lock:
                job = self._pending
                self._pending = None
            if job is None:
                continue
            level, token = job
            self.busyChanged.emit(True)
            try:
                from lecore.raster import compile_level

                result = compile_level(level, write=False)
            except Exception as exc:                    # noqa: BLE001
                # A rasteriser failure is information, not a crash: the person
                # is mid-drag and the geometry can be degenerate for a frame.
                result = exc
            self.busyChanged.emit(False)
            with self._lock:
                superseded = self._pending is not None
            if not superseded:
                self.resultReady.emit(result, token)


class Validator(QObject):
    """Debounced front end for `_RasterThread`, owned by the main window."""

    resultReady = pyqtSignal(object)
    busyChanged = pyqtSignal(bool)

    DEBOUNCE_MS = 200

    def __init__(self, parent=None):
        super().__init__(parent)
        self._thread = _RasterThread(self)
        self._thread.resultReady.connect(self._on_result)
        self._thread.busyChanged.connect(self.busyChanged)
        self._thread.start()

        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(self.DEBOUNCE_MS)
        self._timer.timeout.connect(self._dispatch)
        self._queued: Level | None = None
        self._token = 0

    def submit(self, level: Level | None) -> None:
        if level is None:
            return
        self._queued = level.copy()
        self._timer.start()

    @pyqtSlot()
    def _dispatch(self) -> None:
        if self._queued is None:
            return
        level = self._queued
        self._queued = None
        self._token += 1
        self._thread.submit(level, self._token)

    @pyqtSlot(object, int)
    def _on_result(self, result, token: int) -> None:
        if token == self._token:
            self.resultReady.emit(result)

    def shutdown(self) -> None:
        self._timer.stop()
        self._thread.stop()
        self._thread.quit()
        self._thread.wait(3000)