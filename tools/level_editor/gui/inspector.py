"""Property panels: the level's shape, its cross-section, its features.

Every control here writes straight to the model and emits `changed`. There is
no apply button and no dialog, because the feedback loop is the point: a road
width that only takes effect on OK is a road width you cannot see you got
wrong. What *is* deferred is validation, which rasterises -- see
`gui.worker.ValidateWorker`.
"""

from __future__ import annotations

from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtGui import QColor, QIcon, QPixmap
from PyQt6.QtWidgets import (
    QComboBox, QDoubleSpinBox, QFormLayout, QHBoxLayout, QHeaderView, QLabel,
    QLineEdit, QPushButton, QSpinBox, QTableWidget, QTableWidgetItem,
    QVBoxLayout, QWidget,
)

from lecore import constants as C
from lecore import geometry as G
from lecore.model import Level

MARK_LABEL = {
    "start": "Start / finish",
    "gate": "Checkpoint gate",
    "boost": "Boost pad",
    "oil": "Oil slick",
}


def swatch(code_or_hex) -> QIcon:
    """A 12x12 chip of a palette colour, for combo boxes and tables."""
    hexv = code_or_hex if isinstance(code_or_hex, str) else C.RGB[code_or_hex]
    pix = QPixmap(12, 12)
    pix.fill(QColor(hexv))
    return QIcon(pix)


class LevelPanel(QWidget):
    """Identity, shape mode, cross-section and surface choices."""
    changed = pyqtSignal()
    refitRequested = pyqtSignal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.level: Level | None = None
        self._loading = False

        form = QFormLayout(self)
        form.setLabelAlignment(Qt.AlignmentFlag.AlignRight)

        self.name = QLineEdit("Untitled")
        self.name.editingFinished.connect(self._on_name)
        form.addRow("Name", self.name)

        self.mode = QComboBox()
        self.mode.addItem("Spline  (smooth curves)", "spline")
        self.mode.addItem("Polygon  (filleted corners)", "poly")
        self.mode.currentIndexChanged.connect(self._on_mode)
        form.addRow("Centreline", self.mode)

        self.road = QComboBox()
        for v in C.ROAD_HALF_CHOICES:
            width = int(v * 2)
            self.road.addItem(
                f"{width} cells  ({v:g} half)", v
            )
        self.road.currentIndexChanged.connect(self._on_road)
        form.addRow("Road width", self.road)

        self.runoff = QComboBox()
        for code in (C.GRASS, C.GRAVEL, C.SAND):
            self.runoff.addItem(swatch(code),
                                f"{C.CODE_NAME[code].title()}", code)
        self.runoff.currentIndexChanged.connect(self._on_runoff)
        form.addRow("Runoff", self.runoff)

        self.outside = QComboBox()
        for code in (C.GRASS, C.GRAVEL, C.SAND, C.SCENERY):
            self.outside.addItem(swatch(code),
                                 f"{C.CODE_NAME[code].title()}", code)
        self.outside.currentIndexChanged.connect(self._on_surface)
        form.addRow("Surroundings", self.outside)

        self.kerb = QDoubleSpinBox()
        self.kerb.setRange(0.0, 4.0)
        self.kerb.setSingleStep(0.1)
        self.kerb.setDecimals(2)
        self.kerb.setSuffix(" cells")
        self.kerb.valueChanged.connect(self._on_band)
        form.addRow("Kerb band", self.kerb)

        self.runoff_w = QDoubleSpinBox()
        self.runoff_w.setRange(0.0, 8.0)
        self.runoff_w.setSingleStep(0.2)
        self.runoff_w.setDecimals(2)
        self.runoff_w.setSuffix(" cells")
        self.runoff_w.valueChanged.connect(self._on_band)
        form.addRow("Runoff band", self.runoff_w)

        self.gates = QSpinBox()
        self.gates.setRange(1, C.MAX_GATES)
        self.gates.setValue(C.GATES)
        self.gates.valueChanged.connect(self._on_gates)
        form.addRow("Checkpoint gates", self.gates)

        row = QHBoxLayout()
        self.standard = QPushButton("Standard lap")
        self.standard.clicked.connect(self._on_standard)
        row.addWidget(self.standard)
        self.respace = QPushButton("Re-space gates")
        self.respace.clicked.connect(self._on_respace)
        row.addWidget(self.respace)
        form.addRow("Features", row)

        row2 = QHBoxLayout()
        self.fit = QPushButton("Fit to grid")
        self.fit.clicked.connect(self.refitRequested)
        row2.addWidget(self.fit)
        self.smooth = QPushButton("Smooth lines")
        self.smooth.clicked.connect(self._on_smooth)
        row2.addWidget(self.smooth)
        row2.addStretch(1)
        form.addRow("Geometry", row2)

        self.stats = QLabel("—")
        self.stats.setWordWrap(True)
        self.stats.setTextFormat(Qt.TextFormat.RichText)
        form.addRow(self.stats)

    # --- binding -------------------------------------------------------------

    def set_level(self, level: Level | None) -> None:
        self.level = level
        self.setEnabled(level is not None)
        if level is None:
            self.stats.setText("—")
            return
        self._loading = True
        self.name.setText(level.name)
        self.mode.setCurrentIndex(max(0, self.mode.findData(level.mode)))
        self.road.setCurrentIndex(max(0, self.road.findData(level.road_half_cells)))
        self.runoff.setCurrentIndex(max(0, self.runoff.findData(level.runoff)))
        self.outside.setCurrentIndex(max(0, self.outside.findData(level.outside)))
        self.kerb.setValue(level.kerb_cells)
        self.runoff_w.setValue(level.runoff_cells)
        self.gates.setValue(max(1, len(level.gate_marks())))
        self._loading = False
        self.refresh_stats()

    def refresh_stats(self) -> None:
        if self.level is None:
            return
        lv = self.level
        try:
            length = lv.centreline_length()
        except (ValueError, ZeroDivisionError):
            length = 0.0
        gates = len(lv.gate_marks())
        boosts = sum(1 for m in lv.marks if m.kind == "boost")
        reach = C.max_extent(lv.road_half_cells)
        worst = max((max(abs(n.x - C.GRID / 2), abs(n.y - C.GRID / 2))
                     for n in lv.nodes), default=0.0)
        over = "over" if worst > reach else "fits"
        self.stats.setText(
            f"<b>{len(lv.nodes)}</b> nodes &middot; "
            f"<b>{length:.0f}</b> cells centreline &middot; "
            f"<b>{gates}</b> gates &middot; <b>{boosts}</b> pads<br>"
            f"cross-section reaches <b>{reach:.1f}</b> cells; "
            f"circuit {over}"
        )

    # --- handlers ------------------------------------------------------------

    def _emit(self) -> None:
        if not self._loading and self.level is not None:
            self.refresh_stats()
            self.changed.emit()

    def _on_name(self) -> None:
        if self.level is not None and self.name.text().strip():
            self.level.name = self.name.text().strip()
            self._emit()

    def _on_mode(self) -> None:
        if self.level is not None:
            self.level.mode = self.mode.currentData()
            self._emit()

    def _on_road(self) -> None:
        if self.level is not None:
            self.level.road_half_cells = float(self.road.currentData())
            self._emit()

    def _on_runoff(self) -> None:
        if self.level is not None:
            self.level.runoff = int(self.runoff.currentData())
            self._emit()

    def _on_surface(self) -> None:
        if self.level is not None:
            self.level.outside = int(self.outside.currentData())
            self._emit()

    def _on_band(self) -> None:
        if self.level is not None:
            self.level.kerb_cells = self.kerb.value()
            self.level.runoff_cells = self.runoff_w.value()
            self._emit()

    def _on_gates(self) -> None:
        if self.level is not None and not self._loading:
            n = self.gates.value()
            gates = sorted(level_gates(self.level), key=lambda m: m.frac)
            for i, m in enumerate(gates):
                m.frac = (i + 1) / (n + 1)
            while len(gates) < n:
                from lecore.model import Mark

                gates.append(Mark(kind="gate", frac=(len(gates) + 1) / (n + 1)))
            for m in gates[n:]:
                self.level.marks.remove(m)
            self._emit()

    def _on_standard(self) -> None:
        if self.level is not None:
            self.level.add_standard_marks(self.gates.value())
            self._emit()

    def _on_respace(self) -> None:
        if self.level is not None:
            self.level.nudge_marks()
            self._emit()

    def _on_smooth(self) -> None:
        if self.level is None:
            return
        lv = self.level
        pts = G.smooth_closed(lv.points(), iterations=2, strength=0.55)
        lv.nodes[0].x, lv.nodes[0].y = pts[0]
        for i, (x, y) in enumerate(pts):
            lv.nodes[i].x = x
            lv.nodes[i].y = y
        self._emit()

    def set_gates_display(self, n: int) -> None:
        self._loading = True
        self.gates.setValue(n)
        self._loading = False


def level_gates(level: Level):
    return [m for m in level.marks if m.kind == "gate"]


class MarkPanel(QWidget):
    """The feature table: kind, position on the lap, and lateral offset."""
    changed = pyqtSignal()

    KINDS = ("start", "gate", "boost", "oil")

    def __init__(self, parent=None):
        super().__init__(parent)
        self.level: Level | None = None
        self._loading = False

        lay = QVBoxLayout(self)
        self.table = QTableWidget(0, 4)
        self.table.setHorizontalHeaderLabels(
            ["Feature", "Lap %", "Across", ""]
        )
        self.table.verticalHeader().setVisible(False)
        self.table.setSelectionBehavior(
            QTableWidget.SelectionBehavior.SelectRows
        )
        self.table.horizontalHeader().setSectionResizeMode(
            1, QHeaderView.ResizeMode.Stretch
        )
        self.table.horizontalHeader().setSectionResizeMode(
            2, QHeaderView.ResizeMode.Stretch
        )
        self.table.itemChanged.connect(self._on_item)
        lay.addWidget(self.table)

        row = QHBoxLayout()
        self.add = QComboBox()
        for k in self.KINDS:
            self.add.addItem(swatch(C.MARK_KINDS[k][0]), MARK_LABEL[k], k)
        row.addWidget(self.add)
        b = QPushButton("Add at start line")
        b.clicked.connect(self._on_add)
        row.addWidget(b)
        lay.addLayout(row)

        row2 = QHBoxLayout()
        b2 = QPushButton("Remove selected")
        b2.clicked.connect(self._on_remove)
        row2.addWidget(b2)
        b3 = QPushButton("Clear boost pads")
        b3.clicked.connect(self._on_clear_boosts)
        row2.addWidget(b3)
        lay.addLayout(row2)

    def set_level(self, level: Level | None) -> None:
        self.level = level
        self.setEnabled(level is not None)
        self.refresh()

    def refresh(self) -> None:
        if self.level is None:
            self.table.setRowCount(0)
            return
        self._loading = True
        marks = self.level.sorted_marks()
        self.table.setRowCount(len(marks))
        for row, m in enumerate(marks):
            item = QTableWidgetItem(f"  {MARK_LABEL[m.kind]}")
            item.setIcon(swatch(C.MARK_KINDS[m.kind][0]))
            item.setData(Qt.ItemDataRole.UserRole, m.kind)
            item.setFlags(item.flags() & ~Qt.ItemFlag.ItemIsEditable)
            self.table.setItem(row, 0, item)

            pct = QTableWidgetItem(f"{m.frac * 100:.1f}")
            pct.setData(Qt.ItemDataRole.UserRole, "frac")
            pct.setFlags(pct.flags() | Qt.ItemFlag.ItemIsEditable)
            self.table.setItem(row, 1, pct)

            off = QTableWidgetItem(f"{m.offset:+.2f}")
            off.setData(Qt.ItemDataRole.UserRole, "offset")
            off.setFlags(off.flags() | Qt.ItemFlag.ItemIsEditable)
            self.table.setItem(row, 2, off)
            self.table.setItem(row, 3, QTableWidgetItem())
        self._loading = False

    def _on_item(self, item: QTableWidgetItem) -> None:
        if self._loading or self.level is None:
            return
        row = item.row()
        if row >= len(self.level.marks):
            return
        marks = self.level.sorted_marks()
        if row >= len(marks):
            return
        m = marks[row]
        if item.column() == 1:
            try:
                m.frac = (float(item.text().replace("%", "")) / 100.0) % 1.0
            except ValueError:
                pass
        elif item.column() == 2:
            try:
                m.offset = float(item.text())
            except ValueError:
                pass
        self._loading = True
        item.setText(f"{m.frac * 100:.1f}" if item.column() == 1
                     else f"{m.offset:+.2f}")
        self._loading = False
        self.changed.emit()

    def _on_add(self) -> None:
        if self.level is None:
            return
        from lecore.model import Mark

        kind = self.add.currentData()
        existing = [m.frac % 1.0 for m in self.level.marks]
        frac = 0.5
        for i in range(1, 40):
            cand = i / 40.0
            if all(min(abs(cand - e), 1 - abs(cand - e)) > 0.04
                   for e in existing):
                frac = cand
                break
        self.level.marks.append(Mark(kind=kind, frac=frac))
        self.changed.emit()

    def _on_remove(self) -> None:
        if self.level is None:
            return
        rows = sorted({i.row() for i in self.table.selectedIndexes()},
                      reverse=True)
        marks = self.level.sorted_marks()
        for row in rows:
            if 0 <= row < len(marks):
                self.level.marks.remove(marks[row])
        self.changed.emit()

    def _on_clear_boosts(self) -> None:
        if self.level is None:
            return
        self.level.marks = [m for m in self.level.marks if m.kind != "boost"]
        self.changed.emit()
