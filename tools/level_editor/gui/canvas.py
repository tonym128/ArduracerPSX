"""The editing canvas: a QWidget that paints the level and handles the tools.

Painted with `QPainter` from the same numbers `build_svg` writes, rather than
by rendering the SVG. Two reasons: the same dense centreline sample list that
becomes the `.line.json` sidecar is what gets stroked here, so what you see on
the canvas and what the compiler reads are the same polyline and cannot drift;
and an interactive surface that has to re-rasterise an SVG per mouse move is
not interactive.

The paint order is the format's paint order -- barrier, runoff, kerb, tarmac,
marks -- because that is what ends up in the file. Drawing the road any other
way round would show a level that does not compile to what you see.
"""

from __future__ import annotations

import math

from PyQt6.QtCore import QPointF, QRectF, Qt, pyqtSignal
from PyQt6.QtGui import (
    QBrush, QColor, QFont, QPainter, QPainterPath, QPainterPathStroker,
    QPen, QPolygonF,
)
from PyQt6.QtWidgets import QWidget

from lecore import constants as C
from lecore import geometry as G
from lecore.model import Level, Mark

TOOL_SELECT = "select"
TOOL_NODE = "node"
TOOL_DRAW = "draw"
TOOL_MARK = "mark"
TOOL_ERASE = "erase"

BG = QColor(18, 20, 24)
GRID_MINOR = QColor(38, 42, 50)
GRID_MAJOR = QColor(58, 64, 76)
CENTRELINE = QColor(0, 210, 255, 70)
NODE_FILL = QColor(250, 250, 255)
NODE_EDGE = QColor(24, 26, 32)
NODE_SEL = QColor(255, 190, 40)
RADIUS_HANDLE = QColor(255, 140, 30)
HOVER = QColor(120, 255, 160)

#: Node hit radius in *cells*, so the handle stays grabbable at any zoom.
NODE_HIT_CELLS = 0.9


def _q(hexstr: str) -> QColor:
    return QColor(hexstr)


class TrackCanvas(QWidget):
    """The level, drawn at whatever zoom and centre the person chose."""

    levelChanged = pyqtSignal()
    statusMessage = pyqtSignal(str)
    toolChanged = pyqtSignal(str)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.level: Level | None = None
        self.tool = TOOL_SELECT
        self.mark_kind = "boost"

        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self.setMinimumSize(420, 420)

        self._scale = 9.0          # screen px per cell
        self._origin = QPointF(0, 0)   # screen px of cell (0, 0)
        self._selected: set[int] = set()
        self._hover_node: int | None = None
        self._hover_cell: tuple[float, float] | None = None
        self._drag: dict | None = None
        self._pending: list[tuple[float, float]] = []
        self._show_nodes = True
        self._show_grid = True
        self._show_centreline = True
        self._show_labels = False
        self._show_outside = True
        self._dirty = True

    # --- model ---------------------------------------------------------------

    def set_level(self, level: Level | None) -> None:
        self.level = level
        self._selected.clear()
        self._pending.clear()
        if level is not None:
            self.zoom_fit()
        self.invalidate()

    def invalidate(self) -> None:
        self._dirty = True
        self.update()

    # --- view ----------------------------------------------------------------

    def _cell_to_screen(self, x: float, y: float) -> QPointF:
        return QPointF(self._origin.x() + x * self._scale,
                       self._origin.y() + y * self._scale)

    def _screen_to_cell(self, p: QPointF) -> tuple[float, float]:
        return ((p.x() - self._origin.x()) / self._scale,
                (p.y() - self._origin.y()) / self._scale)

    def zoom_fit(self) -> None:
        if self.level is None:
            return
        margin = 1.0
        w = max(1e-6, self.width() / self._scale)
        h = max(1e-6, self.height() / self._scale)
        self._scale = min(w, h) / (C.GRID + 2 * margin)
        self._centre_on(C.GRID / 2.0, C.GRID / 2.0)

    def _centre_on(self, cx: float, cy: float) -> None:
        self._origin = QPointF(
            self.width() / 2.0 - cx * self._scale,
            self.height() / 2.0 - cy * self._scale,
        )

    @property
    def zoom(self) -> float:
        return self._scale

    def set_zoom(self, scale: float) -> None:
        self._scale = max(1.5, min(80.0, scale))
        self.invalidate()

    def pan_cells(self, dx: float, dy: float) -> None:
        self._origin += QPointF(-dx * self._scale, -dy * self._scale)
        self.invalidate()

    # --- painting ------------------------------------------------------------

    def paintEvent(self, event) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.RenderHint.Antialiasing, False)
        p.fillRect(self.rect(), BG)
        if self.level is None:
            p.setPen(QColor(120, 126, 140))
            p.drawText(self.rect(), Qt.AlignmentFlag.AlignCenter,
                       "No level open.  File > New, or open one from the library.")
            return

        p.save()
        self._paint_ground(p)
        if self._show_grid:
            self._paint_grid(p)
        self._paint_road(p)
        self._paint_marks(p)
        if self._show_centreline:
            self._paint_centreline(p)
        self._paint_shortcuts(p)
        if self._show_nodes:
            self._paint_nodes(p)
        self._paint_pending(p)
        self._paint_border_frame(p)
        p.restore()
        self._dirty = False

    def _paint_ground(self, p: QPainter) -> None:
        assert self.level is not None
        if not self._show_outside:
            return
        col = _q(C.RGB[self.level.outside])
        p.fillRect(self._grid_rect(), col)

    def _grid_rect(self) -> QRectF:
        tl = self._cell_to_screen(0, 0)
        br = self._cell_to_screen(C.GRID, C.GRID)
        return QRectF(tl, br)

    def _paint_grid(self, p: QPainter) -> None:
        step = 1
        while step * self._scale < 14:
            step *= 5 if step in (1, 5) else 2
        p.setPen(QPen(GRID_MINOR, 1))
        i = 0
        while i <= C.GRID:
            x = self._cell_to_screen(i, 0).x()
            p.drawLine(QPointF(x, self._cell_to_screen(0, 0).y()),
                       QPointF(x, self._cell_to_screen(0, C.GRID).y()))
            y = self._cell_to_screen(0, i).y()
            p.drawLine(QPointF(self._cell_to_screen(0, 0).x(), y),
                       QPointF(self._cell_to_screen(C.GRID, 0).x(), y))
            i += step
        p.setPen(QPen(GRID_MAJOR, 1))
        for i in range(0, C.GRID + 1, 10):
            x = self._cell_to_screen(i, 0).x()
            p.drawLine(QPointF(x, self._cell_to_screen(0, 0).y()),
                       QPointF(x, self._cell_to_screen(0, C.GRID).y()))
            y = self._cell_to_screen(0, i).y()
            p.drawLine(QPointF(self._cell_to_screen(0, 0).x(), y),
                       QPointF(self._cell_to_screen(C.GRID, 0).x(), y))
            if self._show_labels and self._scale * step > 18:
                p.setPen(QPen(QColor(150, 158, 172)))
                p.drawText(QPointF(x + 3, self._cell_to_screen(0, 0).y() - 3),
                           str(i))
                p.drawText(QPointF(self._cell_to_screen(0, 0).x() + 3, y - 3),
                           str(i))
        # Centre cross: every circuit is generated about it, so it is the one
        # landmark whose meaning is fixed across all of them.
        mid = self._cell_to_screen(C.GRID / 2.0, C.GRID / 2.0)
        p.setPen(QPen(QColor(90, 100, 118), 1, Qt.PenStyle.DashLine))
        p.drawLine(QPointF(mid.x(), self._cell_to_screen(0, 0).y()),
                   QPointF(mid.x(), self._cell_to_screen(0, C.GRID).y()))
        p.drawLine(QPointF(self._cell_to_screen(0, 0).x(), mid.y()),
                   QPointF(self._cell_to_screen(C.GRID, 0).x(), mid.y()))

    def _centre_path(self) -> QPainterPath:
        assert self.level is not None
        path = QPainterPath()
        try:
            centre = self.level.centreline()
        except (ValueError, ZeroDivisionError):
            return path
        if len(centre) < 2:
            return path
        first = self._cell_to_screen(*centre[0])
        path.moveTo(first)
        for x, y in centre[1:]:
            s = self._cell_to_screen(x, y)
            path.lineTo(s)
        path.closeSubpath()
        return path

    def _stroke(self, p: QPainter, path: QPainterPath, colour: str,
                width_cells: float, cap: Qt.PenCapStyle = Qt.PenCapStyle.FlatCap,
                dash: float | None = None) -> None:
        pen = QPen(_q(colour))
        pen.setWidthF(max(1.0, width_cells * self._scale))
        pen.setCapStyle(cap)
        pen.setJoinStyle(Qt.PenJoinStyle.MiterJoin)
        if dash:
            pen.setDashPattern([max(1.0, dash * self._scale),
                                max(1.0, dash * self._scale)])
        p.setPen(pen)
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawPath(path)

    def _paint_road(self, p: QPainter) -> None:
        assert self.level is not None
        hw = self.level.half_widths()
        path = self._centre_path()
        if path.isEmpty():
            return
        self._stroke(p, path, C.RGB[C.WALL], 2 * hw["wall"])
        self._stroke(p, path, C.RGB[self.level.runoff], 2 * hw["runoff"])
        self._stroke(p, path, C.RGB[C.KERB_WHITE], 2 * hw["kerb"])
        self._stroke(p, path, C.RGB[C.KERB_RED], 2 * hw["kerb"],
                     dash=C.KERB_DASH)
        self._stroke(p, path, C.RGB[C.TARMAC], 2 * hw["road"])

    def _paint_shortcuts(self, p: QPainter) -> None:
        assert self.level is not None
        if not self.level.shortcuts:
            return
        hw = self.level.half_widths()
        for d in self.level.shortcuts:
            path = QPainterPath()
            pts = G_point_list(d)
            if len(pts) < 2:
                continue
            first = self._cell_to_screen(*pts[0])
            path.moveTo(first)
            for x, y in pts[1:]:
                s = self._cell_to_screen(x, y)
                path.lineTo(s)
            cap = Qt.PenCapStyle.RoundCap
            self._stroke(p, path, C.RGB[C.WALL], 2 * hw["wall"], cap)
            self._stroke(p, path, C.RGB[self.level.runoff], 2 * hw["runoff"], cap)
            self._stroke(p, path, C.RGB[C.KERB_WHITE], 2 * hw["kerb"], cap)
            self._stroke(p, path, C.RGB[C.TARMAC], 2 * hw["road"], cap)

    def _paint_marks(self, p: QPainter) -> None:
        assert self.level is not None
        centre = self.level.centreline()
        if len(centre) < 2:
            return
        p.setBrush(Qt.BrushStyle.NoBrush)
        for mark in self.level.sorted_marks():
            try:
                at, tangent = G.point_at_fraction(centre, mark.frac)
            except ValueError:
                continue
            corners = G.band_points(
                at, tangent,
                self.level.mark_half_len(mark),
                self.level.mark_half_thick(mark),
                mark.offset,
            )
            poly = QPolygonF([self._cell_to_screen(*c) for c in corners])
            p.setPen(QPen(QColor(0, 0, 0, 110), 1))
            p.setBrush(QBrush(_q(C.RGB[mark.code])))
            p.drawPolygon(poly)
        p.setBrush(Qt.BrushStyle.NoBrush)

    def _paint_centreline(self, p: QPainter) -> None:
        p.setPen(QPen(CENTRELINE, 1, Qt.PenStyle.DashLine))
        p.drawPath(self._centre_path())

    def _paint_nodes(self, p: QPainter) -> None:
        assert self.level is not None
        r = max(3.0, self._scale * 0.22)
        show_r = self.level.mode == "poly" and self._scale > 6
        font = QFont()
        font.setPointSizeF(max(6.0, r * 0.85))

        for i, n in enumerate(self.level.nodes):
            s = self._cell_to_screen(n.x, n.y)
            if i in self._selected:
                p.setPen(QPen(NODE_SEL, 2))
                p.setBrush(QBrush(QColor(255, 235, 190)))
            elif i == self._hover_node:
                p.setPen(QPen(HOVER, 2))
                p.setBrush(QBrush(HOVER))
            else:
                p.setPen(QPen(NODE_EDGE, 1))
                p.setBrush(QBrush(NODE_FILL))
            p.drawEllipse(s, r, r)

            if show_r:
                a = self.level.nodes[(i - 1) % len(self.level.nodes)]
                b = self.level.nodes[(i + 1) % len(self.level.nodes)]
                hx, hy = _radius_handle(n, a, b)
                hs = self._cell_to_screen(hx, hy)
                p.setPen(QPen(RADIUS_HANDLE, 1.5))
                p.drawLine(s, hs)
                p.setBrush(QBrush(RADIUS_HANDLE))
                p.setPen(QPen(NODE_EDGE, 1))
                p.drawEllipse(hs, r * 0.7, r * 0.7)

            if self._scale > 11 or i in self._selected:
                p.setPen(QPen(QColor(30, 32, 38)))
                p.setFont(font)
                p.drawText(QRectF(s.x() - r * 2, s.y() - r * 1.6,
                                  r * 4, r * 3.2),
                           Qt.AlignmentFlag.AlignCenter, str(i))

    def _paint_pending(self, p: QPainter) -> None:
        if len(self._pending) < 1:
            return
        pts = [self._cell_to_screen(*q) for q in self._pending]
        p.setPen(QPen(QColor(255, 255, 255, 90), 1))
        for i in range(1, len(pts)):
            p.drawLine(pts[i - 1], pts[i])
        r = max(2.5, self._scale * 0.18)
        p.setBrush(QBrush(QColor(255, 255, 255)))
        p.setPen(QPen(NODE_EDGE, 1))
        for s in pts:
            p.drawEllipse(s, r, r)
        if len(pts) > 2:
            p.setPen(QPen(QColor(0, 210, 255), 1, Qt.PenStyle.DashLine))
            p.drawLine(pts[-1], pts[0])

    def _paint_border_frame(self, p: QPainter) -> None:
        p.setPen(QPen(QColor(200, 60, 60), 2))
        p.setBrush(Qt.BrushStyle.NoBrush)
        p.drawRect(self._grid_rect())

    # --- interaction ---------------------------------------------------------

    def mousePressEvent(self, event) -> None:
        level = self.level
        if level is None:
            return
        cell = self._screen_to_cell(event.position())
        button = event.button()

        if button == Qt.MouseButton.MiddleButton or (
            event.modifiers() & Qt.KeyboardModifier.ShiftModifier
            and self.tool == TOOL_SELECT
        ):
            self._drag = {"kind": "pan",
                          "last": event.position().toPoint()}
            return

        if button == Qt.MouseButton.RightButton:
            self._cancel_pending()
            self.set_tool(TOOL_SELECT)
            return

        if self.tool == TOOL_SELECT:
            self._press_select(cell, event)
        elif self.tool == TOOL_NODE:
            self._press_node(cell)
        elif self.tool == TOOL_DRAW:
            self._pending.append(cell)
            self.invalidate()
        elif self.tool == TOOL_MARK:
            self._press_mark(cell)
        elif self.tool == TOOL_ERASE:
            self._press_erase(cell)

    def _press_select(self, cell, event) -> None:
        assert self.level is not None
        idx = self.level.node_at(cell[0], cell[1], NODE_HIT_CELLS)
        additive = bool(event.modifiers() & Qt.KeyboardModifier.ShiftModifier)
        if idx is None:
            if not additive:
                self._selected.clear()
            self.invalidate()
            return
        if additive:
            self._selected.symmetric_difference_update({idx})
        elif idx not in self._selected:
            self._selected = {idx}
        # A radius handle takes priority over the node itself when both are
        # under the cursor: dragging the handle is the only way to round a
        # corner, and the node centre is a much bigger target than the handle.
        handle = self._radius_handle_cell(cell)
        self._drag = {
            "kind": "radius" if handle is not None else "move",
            "index": idx,
            "start": cell,
            "nodes": [(n.x, n.y, n.r) for n in self.level.nodes],
        }

    def _press_node(self, cell) -> None:
        assert self.level is not None
        # Replace the ring with a fresh one seeded from this click, or extend
        # the ring that is already being drawn.
        if not self._pending:
            self._pending = [cell]
        else:
            self._pending.append(cell)
        self.invalidate()

    def _press_mark(self, cell) -> None:
        assert self.level is not None
        centre = self.level.centreline()
        if len(centre) < 2:
            self.statusMessage.emit("Draw a centreline first.")
            return
        frac, dist = G.nearest_fraction(centre, cell)
        reach = self.level.half_widths()["kerb"]
        if dist > reach:
            self.statusMessage.emit(
                f"Click on the road to place a {self.mark_kind} "
                f"({dist:.1f} cells from the centreline, road is {reach:.1f})."
            )
            return
        at, tangent = G.point_at_fraction(centre, frac)
        lateral = (cell[0] - at[0]) * -tangent[1] + (cell[1] - at[1]) * tangent[0]
        self.level.marks.append(
            Mark(kind=self.mark_kind, frac=frac,
                 offset=round(max(-reach, min(reach, lateral)), 3))
        )
        self.levelChanged.emit()
        self.statusMessage.emit(
            f"{self.mark_kind} at {frac * 100:.1f}% of the lap, "
            f"{lateral:+.1f} cells across."
        )

    def _press_erase(self, cell) -> None:
        assert self.level is not None
        idx = self.level.node_at(cell[0], cell[1], NODE_HIT_CELLS)
        removed = "node"
        if idx is None:
            centre = self.level.centreline()
            if len(centre) < 2:
                return
            frac, dist = G.nearest_fraction(centre, cell)
            reach = self.level.half_widths()["kerb"]
            if dist <= reach:
                before = len(self.level.marks)
                self.level.marks = [
                    m for m in self.level.marks
                    if abs((m.frac % 1.0) - frac) > 0.02
                ]
                removed = ("mark" if len(self.level.marks) != before
                           else "nothing")
        if idx is not None:
            del self.level.nodes[idx]
            self._selected.clear()
        if removed != "nothing":
            self.levelChanged.emit()
            self.statusMessage.emit(f"Erased {removed}.")

    def mouseMoveEvent(self, event) -> None:
        level = self.level
        if level is None:
            return
        cell = self._screen_to_cell(event.position())
        self._hover_cell = cell

        if self._drag and self._drag["kind"] == "pan":
            delta = event.position().toPoint() - self._drag["last"]
            self._origin += QPointF(delta)
            self._drag["last"] = event.position().toPoint()
            self.invalidate()
            return

        if self._drag:
            self._continue_drag(cell)
            return

        self._hover_node = level.node_at(cell[0], cell[1], NODE_HIT_CELLS)
        self._update_status(cell)
        self.invalidate()

    def _continue_drag(self, cell) -> None:
        assert self.level is not None
        kind = self._drag["kind"]
        if kind == "move":
            start = self._drag["start"]
            dx = cell[0] - start[0]
            dy = cell[1] - start[1]
            for idx in self._selected:
                ox, oy, _ = self._drag["nodes"][idx]
                self.level.nodes[idx].x = ox + dx
                self.level.nodes[idx].y = oy + dy
        elif kind == "radius":
            idx = self._drag["index"]
            n = self.level.nodes[idx]
            a = self.level.nodes[(idx - 1) % len(self.level.nodes)]
            b = self.level.nodes[(idx + 1) % len(self.level.nodes)]
            n.r = _radius_for(n, a, b, cell, float(self._drag["nodes"][idx][2]))
        self.levelChanged.emit()
        self.invalidate()

    def mouseReleaseEvent(self, event) -> None:
        if self._drag and self._drag["kind"] in ("move", "radius"):
            self.levelChanged.emit()
        self._drag = None

    def mouseDoubleClickEvent(self, event) -> None:
        if self.tool == TOOL_DRAW and len(self._pending) >= 3:
            self._commit_pending()
        elif self.tool == TOOL_NODE and len(self._pending) >= 3:
            self._commit_pending()
        else:
            super().mouseDoubleClickEvent(event)

    def wheelEvent(self, event) -> None:
        before = self._screen_to_cell(event.position())
        factor = 1.18 if event.angleDelta().y() > 0 else 1 / 1.18
        self._scale = max(1.5, min(80.0, self._scale * factor))
        after = self._screen_to_cell(event.position())
        self._origin += QPointF(
            (before[0] - after[0]) * self._scale,
            (before[1] - after[1]) * self._scale,
        )
        self.invalidate()

    def keyPressEvent(self, event) -> None:
        step = 1.0 if not event.modifiers() & Qt.KeyboardModifier.ShiftModifier else 5.0
        key = event.key()
        if key == Qt.Key.Key_Left:
            self.pan_cells(step, 0)
        elif key == Qt.Key.Key_Right:
            self.pan_cells(-step, 0)
        elif key == Qt.Key.Key_Up:
            self.pan_cells(0, step)
        elif key == Qt.Key.Key_Down:
            self.pan_cells(0, -step)
        elif key in (Qt.Key.Key_Plus, Qt.Key.Key_Equal):
            self.set_zoom(self._scale * 1.18)
        elif key == Qt.Key.Key_Minus:
            self.set_zoom(self._scale / 1.18)
        elif key == Qt.Key.Key_F:
            self.zoom_fit()
        elif key == Qt.Key.Key_Delete:
            self.delete_selected()
        elif key == Qt.Key.Key_Escape:
            self._cancel_pending()
            self.set_tool(TOOL_SELECT)
        else:
            super().keyPressEvent(event)

    def _cancel_pending(self) -> None:
        if self._pending:
            self._pending.clear()
            self.statusMessage.emit("Cancelled.")
            self.invalidate()

    def _commit_pending(self) -> None:
        assert self.level is not None
        # Copied, not aliased: `_pending.clear()` below empties the same list
        # object, and aliasing it means the guard below sees zero points and
        # every drawn shape is silently thrown away.
        pts = list(self._pending)
        self._pending.clear()
        if len(pts) < 3:
            self.invalidate()
            return
        from lecore.svgio import estimate_radii

        pts = self._snapped(pts)
        self.level.nodes = [
            _node(x, y, r) for (x, y), r in zip(pts, estimate_radii(pts))
        ]
        self.level.meta.pop("imported", None)
        self.levelChanged.emit()
        self.statusMessage.emit(
            f"Centrelines rebuilt from {len(pts)} points, "
            f"centreline {self.level.centreline_length():.0f} cells."
        )
        self.invalidate()

    def _snapped(self, pts) -> list[tuple[float, float]]:
        out = []
        for x, y in pts:
            out.append((round(x * 4) / 4.0, round(y * 4) / 4.0))
        return out

    # --- commands ------------------------------------------------------------

    def delete_selected(self) -> None:
        assert self.level is not None
        if not self._selected:
            return
        n = len(self._selected)
        keep = [i for i in range(len(self.level.nodes)) if i not in self._selected]
        if len(keep) < 3:
            self.statusMessage.emit(
                "A centreline needs at least 3 nodes; nothing was deleted."
            )
            return
        self.level.nodes = [self.level.nodes[i] for i in keep]
        self._selected.clear()
        self.levelChanged.emit()
        self.statusMessage.emit(f"Deleted {n} node(s).")

    def select_all(self) -> None:
        if self.level is not None:
            self._selected = set(range(len(self.level.nodes)))
            self.invalidate()

    def invert_selection(self) -> None:
        if self.level is not None:
            self._selected = set(range(len(self.level.nodes))) - self._selected
            self.invalidate()

    def set_tool(self, tool: str) -> None:
        if tool != self.tool:
            self._pending.clear()
        self.tool = tool
        self.setCursor(Qt.CursorShape.CrossCursor if tool != TOOL_SELECT
                       else Qt.CursorShape.ArrowCursor)
        self.toolChanged.emit(tool)
        self.invalidate()

    def set_mark_kind(self, kind: str) -> None:
        self.mark_kind = kind

    def _radius_handle_cell(self, cell) -> tuple[float, float] | None:
        assert self.level is not None
        assert self.level is not None
        if self.level.mode != "poly" or not self._show_nodes:
            return None
        best, best_d = None, NODE_HIT_CELLS * 0.8
        for i, n in enumerate(self.level.nodes):
            hx, hy = _radius_handle(
                n,
                self.level.nodes[(i - 1) % len(self.level.nodes)],
                self.level.nodes[(i + 1) % len(self.level.nodes)],
            )
            d = math.hypot(hx - cell[0], hy - cell[1])
            if d < best_d:
                best, best_d = (hx, hy), d
        return best

    def _update_status(self, cell) -> None:
        assert self.level is not None
        assert self.level is not None
        reach = self.level.half_widths()
        fit = "fits" if max(
            max(abs(n.x - C.GRID / 2), abs(n.y - C.GRID / 2))
            for n in self.level.nodes
        ) <= C.max_extent(self.level.road_half_cells) else "OVERFLOWS GRID"
        self.statusMessage.emit(
            f"cell {cell[0]:.1f}, {cell[1]:.1f}   "
            f"zoom {self._scale:.0f}px/cell   "
            f"road {self.level.road_half_cells:g} cells half-width   {fit}"
        )

    # --- view toggles --------------------------------------------------------

    def toggle(self, what: str) -> None:
        attr = f"_show_{what}"
        setattr(self, attr, not getattr(self, attr))
        self.invalidate()


# --- helpers -------------------------------------------------------------------


def _node(x: float, y: float, r: float):
    from lecore.model import Node

    return Node(x, y, r)


def G_point_list(d: str):
    from lecore.svgio import flatten_path

    return flatten_path(d)


def _bisector(n, a, b) -> tuple[float, float]:
    """Outward unit direction at a node, from the two adjacent edges."""
    ux = (b.x - n.x) + (n.x - a.x)
    uy = (b.y - n.y) + (n.y - a.y)
    m = math.hypot(ux, uy)
    if m < 1e-9:
        return (0.0, 0.0)
    return (ux / m, uy / m)


def _radius_handle(n, a, b) -> tuple[float, float]:
    d = _bisector(n, a, b)
    return (n.x + d[0] * max(n.r, 0.6), n.y + d[1] * max(n.r, 0.6))


def _radius_for(n, a, b, cell, previous: float) -> float:
    """Radius implied by dragging the handle to `cell`.

    Projected onto the corner bisector, because that is the direction the
    handle moves in. Using the raw distance from the node would let a drag
    perpendicular to the bisector change the radius with no visible movement of
    anything -- and in `poly` mode, no visible change of the road either.
    """
    d = _bisector(n, a, b)
    if abs(d[0]) + abs(d[1]) < 1e-9:
        return previous
    vx, vy = cell[0] - n.x, cell[1] - n.y
    return max(0.0, vx * d[0] + vy * d[1])