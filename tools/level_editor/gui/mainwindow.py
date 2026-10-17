"""The main window: menus, docks, actions, and the publish path.

Layout is canvas in the middle, properties and features on the right, and a
validation report along the bottom. The report is a dock rather than a dialog
because it has to be readable *while* editing, which is the only time it is
useful.
"""

from __future__ import annotations

import os

from PyQt6.QtCore import QSize, Qt, pyqtSlot
from PyQt6.QtGui import QAction, QActionGroup, QColor, QKeySequence, QPixmap
from PyQt6.QtWidgets import (
    QDockWidget, QFileDialog, QLabel, QMainWindow, QMessageBox, QPlainTextEdit,
    QProgressDialog, QToolBar, QWidget,
)

from lecore import constants as C
from lecore import generate as gen
from lecore import raster
from lecore import svgio
from lecore.project import LevelStore, default_store
from . import canvas as CV
from .dialogs import GenerateDialog, LibraryDialog
from .inspector import LevelPanel, MarkPanel, swatch
from .worker import Validator

APP_NAME = "Arduracer PSX Level Editor"


class MainWindow(QMainWindow):
    def __init__(self, store: LevelStore | None = None):
        super().__init__()
        self.setWindowTitle(APP_NAME)
        self.resize(1360, 880)

        self.store = store or default_store()
        self.store.ensure()
        self.level = None
        self.level_path: str | None = None
        self._dirty = False

        self.canvas = CV.TrackCanvas(self)
        self.setCentralWidget(self.canvas)

        self.level_panel = LevelPanel(self)
        self.mark_panel = MarkPanel(self)
        self.report = QPlainTextEdit()
        self.report.setReadOnly(True)
        self.report.setMaximumBlockCount(400)

        self.status = self.statusBar()
        self._build_docks()
        self._build_actions()
        self._connect()
        self._build_toolbar()
        self._set_dirty(False)

        self.validator = Validator(self)
        self.validator.resultReady.connect(self._on_validated)
        self.validator.busyChanged.connect(self._on_busy)

        self.status.showMessage("Ready.")

        self._new_level()

    # --- construction --------------------------------------------------------

    def _build_docks(self) -> None:
        right = QDockWidget("Level", self)
        right.setObjectName("dock-level")
        right.setWidget(self.level_panel)
        right.setFeatures(QDockWidget.DockWidgetFeature.DockWidgetMovable)
        self.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea, right)

        feats = QDockWidget("Features", self)
        feats.setObjectName("dock-features")
        feats.setWidget(self.mark_panel)
        self.addDockWidget(Qt.DockWidgetArea.RightDockWidgetArea, feats)

        bottom = QDockWidget("Validation", self)
        bottom.setObjectName("dock-report")
        bottom.setWidget(self.report)
        bottom.setMinimumHeight(110)
        self.addDockWidget(Qt.DockWidgetArea.BottomDockWidgetArea, bottom)
        self.report.setPlainText(
            "Nothing open.\n\n"
            "Open a circuit from the library, import one from tracks/, or "
            "generate a new one. Validation runs as you edit and says here "
            "what the game build would say."
        )

    def _act(self, text, slot, shortcut=None, tip=None, checkable=False,
             checked=False) -> QAction:
        a = QAction(text, self)
        a.triggered.connect(slot)
        if shortcut:
            a.setShortcut(QKeySequence(shortcut))
        if tip:
            a.setStatusTip(tip)
        a.setCheckable(checkable)
        a.setChecked(checked)
        return a

    def _build_actions(self) -> None:
        m = self.menuBar()

        f = m.addMenu("&File")
        self.action_new = self._act("&New circuit", self._new_level, "Ctrl+N")
        self.action_open = self._act("&Open library...", self._open_library,
                                     "Ctrl+O")
        self.action_publish = self._act(
            "&Publish into tracks/ (adds to the game)", self._publish,
            "Ctrl+Shift+B",
            "Write the compiled level into tracks/, which make atlas reads. "
            "This is the step that puts the level in the game.",
        )
        for a in (self.action_new, self.action_open,
                  self._act("&Import SVG...", self._import_svg),
                  self._act("Import from game &tracks/...", self._import_tracks)):
            f.addAction(a)
        f.addSeparator()
        for a in (self._act("&Save", self._save, "Ctrl+S"),
                  self._act("Save &as...", self._save_as, "Ctrl+Shift+S"),
                  self._act("Export S&VG...", self._export_svg)):
            f.addAction(a)
        f.addSeparator()
        for a in (self._act("&Build in store", self._build, "Ctrl+B"),
                  self.action_publish,
                  self._act("&Remove from tracks/", self._unpublish)):
            f.addAction(a)
        f.addSeparator()
        f.addAction(self._act("&Quit", self.close, "Ctrl+Q"))

        e = m.addMenu("&Edit")
        e.addAction(self._act("&Undo", self._undo, "Ctrl+Z"))
        e.addSeparator()
        e.addAction(self._act("Select &all", self.canvas.select_all, "Ctrl+A"))
        e.addAction(self._act("&Invert selection", self.canvas.invert_selection,
                              "Ctrl+Shift+I"))
        e.addAction(self._act("De&lete selected nodes", self.canvas.delete_selected,
                              "Del"))
        e.addSeparator()
        e.addAction(self._act("&Smooth all lines", self._on_smooth_all))
        e.addAction(self._act("&Simplify nodes", self._on_simplify))
        e.addAction(self._act("&Distribute nodes evenly", self._on_distribute))

        t = m.addMenu("&Tool")
        self.tool_group = QActionGroup(self)
        self.tool_group.setExclusive(True)
        for tool, label, key, tip in (
            (CV.TOOL_SELECT, "&Select / move", "1",
             "Drag nodes. Drag the orange handle to round a corner in polygon "
             "mode. Shift-click to add to the selection."),
            (CV.TOOL_NODE, "&Add nodes", "2",
             "Click to place centreline nodes. Double-click or Enter to close "
             "the ring and rebuild the curve."),
            (CV.TOOL_DRAW, "&Draw curve", "3",
             "Click out a shape, double-click to close it. Points are snapped "
             "to a quarter cell and the curve is fitted through them."),
            (CV.TOOL_MARK, "Place &features", "4",
             "Click on the road to drop the selected feature at that point on "
             "the lap, snapped to the centreline."),
            (CV.TOOL_ERASE, "&Erase", "5",
             "Click a node or a feature on the road to remove it."),
        ):
            a = self._act(label, lambda _=False, tl=tool: self._set_tool(tl),
                          key, tip, checkable=True,
                          checked=tool == CV.TOOL_SELECT)
            self.tool_group.addAction(a)
            t.addAction(a)
        t.addSeparator()
        self.mark_group = QActionGroup(self)
        self.mark_group.setExclusive(True)
        for i, kind in enumerate(C.PLACEABLE_MARKS):
            a = self._act(C.MARK_KINDS[kind][0].title() if False
                          else kind.capitalize(),
                          lambda _=False, k=kind: self._set_mark(k),
                          f"Ctrl+{i + 1}", f"Place a {kind}",
                          checkable=True, checked=(kind == "boost"))
            self.mark_group.addAction(a)
            t.addAction(a)

        v = m.addMenu("&View")
        for what, label, key in (
            ("grid", "Grid", "G"),
            ("nodes", "Node handles", "H"),
            ("centreline", "Centreline", "L"),
            ("labels", "Cell coordinates", "Ctrl+L"),
            ("outside", "Surroundings", "Ctrl+B"),
        ):
            v.addAction(self._act(label, lambda _=False, w=what: self._toggle(w),
                                  key, checkable=True, checked=True))
        v.addSeparator()
        v.addAction(self._act("&Fit to view", self.canvas.zoom_fit, "F"))
        v.addAction(self._act("Zoom &in", lambda: self.canvas.set_zoom(
            self.canvas.zoom * 1.25), "Ctrl++"))
        v.addAction(self._act("Zoom &out", lambda: self.canvas.set_zoom(
            self.canvas.zoom / 1.25), "Ctrl+-"))

        h = m.addMenu("&Help")
        h.addAction(self._act("How levels work", self._show_help))
        h.addAction(self._act("Palette reference", self._show_palette))
        h.addAction(self._act("&About", self._about))

    def _build_toolbar(self) -> None:
        bar = QToolBar("Tools")
        bar.setIconSize(QSize(16, 16))
        bar.setToolButtonStyle(Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        self.addToolBar(bar)
        for a in self.tool_group.actions():
            bar.addAction(a)
        bar.addSeparator()
        for a in self.mark_group.actions():
            bar.addAction(a)
        bar.addSeparator()
        for a in (self.action_new, self.action_open, self.action_publish):
            bar.addAction(a)
        self.toolbar = bar

    def _connect(self) -> None:
        self.canvas.levelChanged.connect(self._on_level_changed)
        self.canvas.statusMessage.connect(self.status.showMessage)
        self.level_panel.changed.connect(self._on_panel_changed)
        self.mark_panel.changed.connect(self._on_panel_changed)
        self.level_panel.refitRequested.connect(self._on_fit)

    # --- state ---------------------------------------------------------------

    def _set_dirty(self, dirty: bool) -> None:
        self._dirty = dirty
        name = self.level.name if self.level else "no level"
        where = os.path.basename(self.level_path) if self.level_path else "unsaved"
        self.setWindowTitle(f"{APP_NAME} — {name}"
                            f"{' *' if dirty else ''}  [{where}]")

    def _set_tool(self, tool: str) -> None:
        self.canvas.set_tool(tool)
        for a in self.tool_group.actions():
            a.setChecked(a.text().endswith(
                {"select": "move", "node": "nodes", "draw": "curve",
                 "mark": "features", "erase": "Erase"}[tool]))

    def _set_mark(self, kind: str) -> None:
        self.canvas.set_mark_kind(kind)
        if self.canvas.tool != CV.TOOL_MARK:
            self._set_tool(CV.TOOL_MARK)

    def _toggle(self, what: str) -> None:
        self.canvas.toggle(what)

    # --- level lifecycle -----------------------------------------------------

    def _new_level(self) -> None:
        if not self._confirm_discard():
            return
        level = gen.generate(gen.GenParams(seed=_seed(), difficulty="medium"))
        level.name = "New Circuit"
        level.meta.pop("generated", None)
        self._adopt(level, None)

    def _adopt(self, level, path: str | None) -> None:
        self.level = level
        self.level_path = path
        self.canvas.set_level(level)
        self.level_panel.set_level(level)
        self.mark_panel.set_level(level)
        self._set_dirty(False)
        self.validator.submit(level)

    def _open_library(self) -> None:
        if not self._confirm_discard():
            return
        dlg = LibraryDialog(self.store, self)
        dlg.levelChosen.connect(self._open_from_library)
        dlg.exec()

    def _open_from_library(self, stem: str, origin: str) -> None:
        try:
            if origin == "store":
                level = self.store.load(stem)
                path = self.store.path_for(stem)
            else:
                svg = os.path.join(C.TRACKS_DIR, f"{stem}.svg")
                level = svgio.load_svg(svg)
                level.meta["adopted_from"] = svg
                path = None
        except (ValueError, OSError) as exc:
            QMessageBox.critical(self, "Could not open level", str(exc))
            return
        self._adopt(level, path)

    def _import_svg(self) -> None:
        if not self._confirm_discard():
            return
        path, _ = QFileDialog.getOpenFileName(
            self, "Import circuit SVG", C.TRACKS_DIR, "SVG files (*.svg)")
        if not path:
            return
        try:
            level = svgio.load_svg(path)
        except (ValueError, OSError) as exc:
            QMessageBox.critical(
                self, "Could not read that SVG",
                f"{exc}\n\nA game-format circuit paints its road as a "
                f"{C.RGB[C.TARMAC]} stroke on the centreline. Anything else is "
                f"not a circuit this editor can open."
            )
            return
        level.meta["adopted_from"] = os.path.abspath(path)
        self._adopt(level, None)
        self.status.showMessage(
            f"Imported {os.path.basename(path)} — the centreline was recovered "
            f"from the drawing, so it may differ from the original by a fraction "
            f"of a cell. Save it to keep it."
        )

    def _import_tracks(self) -> None:
        if not self._confirm_discard():
            return
        dlg = LibraryDialog(self.store, self)
        dlg._adopt_tracks()
        dlg.show()

    def _save(self) -> None:
        if self.level is None:
            return
        if not self.level_path:
            self._save_as()
            return
        try:
            self.store.save(self.level, self.level_path)
        except OSError as exc:
            QMessageBox.critical(self, "Save failed", str(exc))
            return
        self._set_dirty(False)
        self.status.showMessage(f"Saved {self.level_path}")

    def _save_as(self) -> None:
        if self.level is None:
            return
        suggestion = os.path.join(
            self.store.root, f"{self.level.stem}{'.ardlvl'}")
        path, _ = QFileDialog.getSaveFileName(
            self, "Save level", suggestion, "Arduracer level (*.ardlvl)")
        if not path:
            return
        if not path.endswith(".ardlvl"):
            path += ".ardlvl"
        try:
            self.store.save(self.level, path)
        except OSError as exc:
            QMessageBox.critical(self, "Save failed", str(exc))
            return
        self.level_path = path
        self._set_dirty(False)
        self.status.showMessage(f"Saved {path}")

    def _export_svg(self) -> None:
        if self.level is None:
            return
        path, _ = QFileDialog.getSaveFileName(
            self, "Export SVG",
            os.path.join(self.store.build_dir, f"{self.level.stem}.svg"),
            "SVG files (*.svg)")
        if not path:
            return
        try:
            svgio.save_svg(self.level, path)
        except OSError as exc:
            QMessageBox.critical(self, "Export failed", str(exc))
            return
        self.status.showMessage(f"Exported {path}")

    def _confirm_discard(self) -> bool:
        if not self._dirty or self.level is None:
            return True
        r = QMessageBox.question(
            self, "Unsaved changes",
            f"'{self.level.name}' has unsaved changes. Discard them?",
            QMessageBox.StandardButton.Save
            | QMessageBox.StandardButton.Discard
            | QMessageBox.StandardButton.Cancel,
        )
        if r == QMessageBox.StandardButton.Cancel:
            return False
        if r == QMessageBox.StandardButton.Save:
            self._save()
            return not self._dirty
        return True

    def _undo(self) -> None:
        self.status.showMessage(
            "No undo history. Levels are saved often enough that undo is "
            "usually the wrong instinct — use the store."
        )

    # --- build / publish -----------------------------------------------------

    def _build(self) -> None:
        if self.level is None:
            return
        result = self._run_busy("Build failed",
                                lambda: self.store.build(self.level, visual=False))
        if result is not None:
            self._show_result(result, "Built")

    def _publish(self) -> None:
        if self.level is None:
            return
        if not self.level_path:
            QMessageBox.question(
                self, "Not saved",
                "This level has never been saved to the store. Publishing "
                "puts compiled files in tracks/, which the game build reads, "
                "but there would be no editable source to go back to.\n\n"
                "Save first?",
            )
            if not self._confirm_discard():
                return
            self._save()
        result = self._run_busy(
            "Publish failed", lambda: self.store.publish(self.level, visual=True)
        )
        if result is None:
            return
        self._show_result(result, "Published")
        self.status.showMessage(
            f"{result.stem} is in tracks/ ({published} levels in the store). "
            f"Run 'make atlas' to compile it into the game."
        )

    def _unpublish(self) -> None:
        if self.level is None:
            return
        stem = self.level.stem
        r = QMessageBox.question(
            self, "Remove from the game",
            f"Delete every compiled file for '{stem}' from tracks/?\n\n"
            f"The level stays in the store. It will disappear from the game on "
            f"the next 'make atlas'.",
        )
        if r != QMessageBox.StandardButton.Yes:
            return
        removed = self.store.unpublish(stem)
        self.status.showMessage(
            f"Removed {len(removed)} file(s) for {stem}. Run 'make atlas' to "
            f"drop it from the game's track list."
        )

    def _show_result(self, result, verb: str) -> None:
        ok = result.ok
        self.report.setPlainText(_format_report(result, verb))
        if not ok:
            self.report.setFocus()
        QMessageBox.information(
            self, f"{verb}: {result.level.name}",
            f"{'Valid.' if ok else 'Has errors.'}\n\n"
            + "\n".join(result.report.errors or result.report.warnings)
            + f"\n\nFiles:\n"
            + "\n".join(f"  {v}" for v in result.paths())
        )

    def _run_busy(self, title: str, fn):
        """Run a blocking job with a wait cursor, reporting failure as a dialog.

        Compiling writes five files and paints a megapixel; a frozen window
        during it reads as a crash. The exception is shown rather than raised
        so a failure is attributed to the level or the disk rather than to a
        traceback the person has to read.
        """
        from PyQt6.QtGui import QCursor
        from PyQt6.QtWidgets import QApplication

        QApplication.setOverrideCursor(QCursor(Qt.CursorShape.WaitCursor))
        QApplication.processEvents()
        try:
            return fn()
        except Exception as exc:                        # noqa: BLE001
            QMessageBox.critical(self, title, str(exc))
            return None
        finally:
            QApplication.restoreOverrideCursor()

    # --- edits ---------------------------------------------------------------

    def _on_level_changed(self) -> None:
        self._set_dirty(True)
        self.level_panel.refresh_stats()
        self.validator.submit(self.level)

    def _on_panel_changed(self) -> None:
        self.canvas.invalidate()
        self._on_level_changed()

    def _on_fit(self) -> None:
        if self.level is None:
            return
        moved = self.level.scale_to_fit()
        self.canvas.invalidate()
        self._on_level_changed()
        self.status.showMessage(
            "Scaled the circuit down to fit." if moved else "Already fits."
        )

    def _on_smooth_all(self) -> None:
        if self.level is None:
            return
        from lecore import geometry as G

        pts = G.smooth_closed(self.level.points(), iterations=2, strength=0.6)
        for n, (x, y) in zip(self.level.nodes, pts):
            n.x, n.y = x, y
        self._on_level_changed()
        self.status.showMessage("Smoothed every node twice, centroid preserved.")

    def _on_simplify(self) -> None:
        if self.level is None:
            return
        from lecore import geometry as G

        before = len(self.level.nodes)
        pts = G.simplify_closed(self.level.centreline(), 0.5)
        if len(pts) < 3:
            self.status.showMessage("Already minimal; nothing to remove.")
            return
        radii = svgio.estimate_radii(pts)
        self.level.nodes = [
            type(self.level.nodes[0])(x, y, r)
            for (x, y), r in zip(pts, radii)
        ]
        self._on_level_changed()
        self.status.showMessage(
            f"{before} nodes -> {len(self.level.nodes)}; the drawn shape is "
            f"unchanged to within half a cell."
        )

    def _on_distribute(self) -> None:
        if self.level is None:
            return
        from lecore import geometry as G

        n = len(self.level.nodes)
        pts = G.resample_closed(self.level.points(), n)
        radii = svgio.estimate_radii(pts)
        for node, (x, y), r in zip(self.level.nodes, pts, radii):
            node.x, node.y, node.r = x, y, r
        self._on_level_changed()
        self.status.showMessage("Nodes spread to even arc length.")

    # --- validation ----------------------------------------------------------

    @pyqtSlot(object)
    def _on_validated(self, result) -> None:
        if isinstance(result, Exception):
            self.report.setPlainText(f"Could not rasterise: {result}")
            return
        self.report.setPlainText(_format_report(result, "Live check"))
        self.level_panel.set_gates_display(len(self.level.gate_marks())
                                           if self.level else 6)

    @pyqtSlot(bool)
    def _on_busy(self, busy: bool) -> None:
        self.setCursor(
            Qt.CursorShape.BusyCursor if busy else Qt.CursorShape.ArrowCursor
        )

    # --- help ----------------------------------------------------------------

    def _show_help(self) -> None:
        QMessageBox.information(self, "How levels work", HELP_TEXT)

    def _show_palette(self) -> None:
        rows = "\n".join(
            f"  {p['code']:2d}  {p['hex']}  {p['name']:<13}"
            f"{'drivable' if p['code'] in C.SURFACE_CODES else ''}"
            for p in C.palette_report()
        )
        QMessageBox.information(
            self, "Level palette",
            f"The 16 colours the data image must use exactly. Matching is by "
            f"RGB equality, so nothing here can be approximated.\n\n{rows}")

    def _about(self) -> None:
        QMessageBox.about(
            self, "About",
            f"<b>{APP_NAME}</b><br><br>"
            f"Draws levels in the same SVG format and palette the game build "
            f"compiles, on the same {C.GRID}-cell grid and "
            f"{C.WORLD_PER_CELL}-world-unit cell.<br><br>"
            f"Store: <code>{self.store.root}</code><br>"
            f"Published into: <code>{C.TRACKS_DIR}</code>"
        )

    def closeEvent(self, event) -> None:
        if not self._confirm_discard():
            event.ignore()
            return
        self.validator.shutdown()
        event.accept()


def _format_report(result, verb: str) -> str:
    r = result.report
    lines = [f"{verb}: {result.level.name}",
             f"centreline {r.info.get('centreline_length_cells', '?')} cells, "
             f"{r.info.get('centreline_samples', '?')} samples, "
             f"{result.level.road_half_cells:g}-cell half-width "
             f"({int(result.level.road_half_cells * 2)} cells wide)",
             f"road cells {r.info.get('road_cells', '?')} in "
             f"{r.info.get('road_components', '?')} component(s)",
             ""]
    if r.errors:
        lines.append("ERRORS — make atlas will refuse this circuit:")
        lines += [f"  • {e}" for e in r.errors]
    else:
        lines.append("No errors. This compiles.")
    if r.warnings:
        lines.append("")
        lines.append("Worth knowing:")
        lines += [f"  • {w}" for w in r.warnings]
    hist = r.info.get("histogram")
    if hist:
        lines.append("")
        lines.append("cells: " + ", ".join(f"{k} {v}" for k, v in
                                           sorted(hist.items(),
                                                  key=lambda kv: -kv[1])))
    return "\n".join(lines)


def _seed() -> int:
    import random

    return random.randrange(1, 2 ** 31 - 1)


HELP_TEXT = """<b>What a level is</b><br>
A centreline plus a road width, painted as five concentric strokes -- barrier,
runoff, kerb, tarmac -- in cell units on a 80x80 grid, in sixteen exact
colours. <code>make atlas</code> rasterises it, and the game reads the cells.<br><br>

<b>Drawing</b><br>
• <b>Select</b>: drag nodes. In polygon mode, drag the orange handle to round
  a corner. Shift-click to multi-select; drag moves all of them.<br>
• <b>Add nodes</b>: click to place points, double-click to close the ring.<br>
• <b>Draw curve</b>: click out a shape, double-click to close it. Points snap
  to a quarter cell and the spline is fitted through them.<br>
• <b>Features</b>: click on the road to drop a boost pad, checkpoint gate or
  oil slick at that point on the lap. It snaps to the centreline and keeps its
  position as a fraction of the lap, so it survives you dragging the road.<br>
• <b>Erase</b>: click a node, or a feature on the road, to remove it.<br><br>

<b>Spline vs polygon</b><br>
Spline is smooth: curvature is continuous through every corner. Polygon has a
circular fillet per corner, which is what the shipped circuits use and what
lets you round one corner hard while leaving the next square.<br><br>

<b>Where things go</b><br>
Levels are saved to the store as <code>.ardlvl</code> files, outside the game.
<b>Publish</b> writes the compiled files into <code>tracks/</code>, which is
what <code>make atlas</code> reads. Anything in <code>tracks/</code> becomes a
track in the game, so publishing is the step that matters -- and
<code>build_atlas</code> refuses if there are more circuits than track slots.<br><br>

<b>Validation</b><br>
The panel along the bottom runs the same rules the compiler runs, on a real
raster of what you are drawing. It is the same answer <code>make atlas</code>
will give, not an approximation of it."""

#: Convenience alias so `_show_palette` can ask the format which codes are
#: drivable without importing the validator into the window.
C.SURFACE_CODES = frozenset(
    {C.TARMAC, C.TARMAC_WORN, C.KERB_WHITE, C.KERB_RED, C.GRASS, C.GRAVEL,
     C.SAND, C.OIL, C.BOOST, C.START_LINE, C.GATE, C.TUNNEL, C.BRIDGE,
     C.SCENERY}
)