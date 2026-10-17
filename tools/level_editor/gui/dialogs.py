"""Dialogs: the generator, the level library, and the batch runner."""

from __future__ import annotations

import os

from PyQt6.QtCore import Qt, QThread, pyqtSignal
from PyQt6.QtWidgets import (
    QAbstractItemView, QCheckBox, QComboBox, QDialog, QDialogButtonBox,
    QFormLayout, QHBoxLayout, QHeaderView, QLabel, QLineEdit, QListWidget,
    QListWidgetItem, QMessageBox, QPlainTextEdit, QProgressBar, QPushButton,
    QSpinBox, QTabWidget, QVBoxLayout, QWidget,
)

from lecore import constants as C
from lecore import generate as gen
from lecore.project import LevelStore

#: Road half-widths the batch "mix" cycles through, in cells.
#:
#: Derived from the label next to it rather than the other way round: the two
#: were separate literals, so a change to the list left the label promising a
#: cycle the generator did not perform.
BATCH_WIDTH_MIX = (2.0, 3.0, 3.0, 4.0)


class GenerateDialog(QDialog):
    """One random circuit, or a batch of them."""

    def __init__(self, store: LevelStore, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Generate circuit")
        self.resize(460, 470)
        self.store = store

        root = QVBoxLayout(self)
        self.tabs = QTabWidget(self)
        root.addWidget(self.tabs)

        # --- one ---------------------------------------------------------------
        one = QWidget()
        f1 = QFormLayout(one)
        self.one_difficulty = QComboBox()
        for name in gen.DIFFICULTIES:
            cfg = gen.DIFFICULTIES[name]
            self.one_difficulty.addItem(
                f"{name.title()}   ({cfg['perimeter']} cells, "
                f"{cfg['nodes']} nodes)", name
            )
        self.one_difficulty.setCurrentIndex(1)
        f1.addRow("Difficulty", self.one_difficulty)

        self.one_width = QComboBox()
        for v in C.ROAD_HALF_CHOICES:
            self.one_width.addItem(C.road_half_label(v), v)
        # Select the design width explicitly. A `QComboBox` opens on index 0,
        # and index 0 is now the *narrowest* legal road -- so a user who opened
        # Generate, typed a name and pressed OK used to get a two-cell circuit
        # that immediately failed validation. Defaulting to the width every
        # shipped circuit uses is what the dialog has always meant.
        self.one_width.setCurrentIndex(
            max(0, self.one_width.findData(C.DEFAULT_ROAD_HALF_CELLS))
        )
        f1.addRow("Road width", self.one_width)

        self.one_mode = QComboBox()
        self.one_mode.addItem("Spline", "spline")
        self.one_mode.addItem("Polygon", "poly")
        f1.addRow("Centreline", self.one_mode)

        self.one_pads = QSpinBox()
        self.one_pads.setRange(0, 12)
        self.one_pads.setValue(3)
        f1.addRow("Boost pads", self.one_pads)

        self.one_oil = QSpinBox()
        self.one_oil.setRange(0, 12)
        self.one_oil.setValue(1)
        f1.addRow("Oil slicks", self.one_oil)

        self.one_seed = QSpinBox()
        self.one_seed.setRange(0, 2 ** 31 - 1)
        self.one_seed.setValue(_fresh_seed())
        f1.addRow("Seed", self.one_seed)

        self.one_name = QLineEdit()
        self.one_name.setPlaceholderText("auto")
        f1.addRow("Name", self.one_name)
        self.tabs.addTab(one, "One circuit")

        # --- batch -------------------------------------------------------------
        many = QWidget()
        f2 = QFormLayout(many)
        self.batch_count = QSpinBox()
        self.batch_count.setRange(1, 500)
        self.batch_count.setValue(24)
        f2.addRow("How many", self.batch_count)

        self.batch_difficulties = QComboBox()
        self.batch_difficulties.addItem("cycle through all", "")
        for name in gen.DIFFICULTIES:
            self.batch_difficulties.addItem(name.title(), name)
        f2.addRow("Difficulty", self.batch_difficulties)

        self.batch_widths = QComboBox()
        self.batch_widths.addItem(f"cycle {' / '.join(f'{int(v)}' for v in BATCH_WIDTH_MIX)}", "mix")
        for v in C.ROAD_HALF_CHOICES:
            self.batch_widths.addItem(C.road_half_label(v), v)
        f2.addRow("Road width", self.batch_widths)

        self.batch_prefix = QLineEdit()
        self.batch_prefix.setPlaceholderText("e.g. Cup 2 ")
        f2.addRow("Name prefix", self.batch_prefix)

        self.batch_seed = QSpinBox()
        self.batch_seed.setRange(0, 2 ** 31 - 1)
        self.batch_seed.setValue(_fresh_seed())
        f2.addRow("Seed", self.batch_seed)

        self.batch_batchname = QLineEdit("batch")
        f2.addRow("Record as batch", self.batch_batchname)

        self.batch_build = QCheckBox("Validate each one as it is generated")
        self.batch_build.setChecked(True)
        f2.addRow(self.batch_build)

        self.batch_publish = QCheckBox("Also publish into tracks/ (adds to the game)")
        self.batch_publish.setToolTip(
            "tracks/ is the published set. Everything there becomes a TrackDef "
            "when you run make atlas, and build_atlas refuses if there are more "
            "circuits than track slots."
        )
        f2.addRow(self.batch_publish)

        self.progress = QProgressBar()
        self.progress.setVisible(False)
        f2.addRow(self.progress)
        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.setMaximumHeight(120)
        f2.addRow(self.log)
        self.tabs.addTab(many, "Batch")

        self.buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel
        )
        self.buttons.accepted.connect(self._on_accept)
        self.buttons.rejected.connect(self.reject)
        root.addWidget(self.buttons)

        self.result_levels: list = []

    def _on_accept(self) -> None:
        if self.tabs.currentIndex() == 0:
            self._generate_one()
        else:
            self._generate_batch()
        if self.result_levels:
            self.accept()

    def _generate_one(self) -> None:
        width = self.one_width.currentData()
        width = 3.0 if width is None else float(width)
        params = gen.GenParams(
            seed=self.one_seed.value(),
            difficulty=self.one_difficulty.currentData(),
            mode=self.one_mode.currentData(),
            road_half_cells=width,
            boost_pads=self.one_pads.value(),
            oil_slicks=self.one_oil.value(),
            name=self.one_name.text().strip(),
        )
        avoid = {s.stem for s in self.store.levels()}
        level = gen.generate(params, avoid=avoid)
        self.result_levels = [level]

    def _generate_batch(self) -> None:
        want = self.batch_difficulties.currentData()
        pool = [want] if want else list(gen.DIFFICULTIES)
        w = self.batch_widths.currentData()
        widths = list(BATCH_WIDTH_MIX) if w == "mix" else [float(w)]
        self.progress.setVisible(True)
        self.progress.setRange(0, self.batch_count.value())
        self.result_levels = gen.generate_batch(
            self.batch_count.value(),
            seed=self.batch_seed.value(),
            store=self.store.root,
            difficulties=pool,
            road_widths=widths,
            prefix=self.batch_prefix.text(),
            publish=self.batch_publish.isChecked(),
            on_progress=self._on_batch_progress,
        )
        name = self.batch_batchname.text().strip()
        if name:
            self.store.write_batch(
                name, self.result_levels,
                seed=self.batch_seed.value(),
                difficulties=pool,
                road_widths=widths,
                prefix=self.batch_prefix.text(),
            )
            self.log.appendPlainText(f"batch recorded as {name!r}")

    def _on_batch_progress(self, done: int, total: int, level) -> None:
        self.progress.setValue(done)
        row = (f"{done}/{total}  {level.name}  "
               f"{level.meta.get('difficulty', '?')}  "
               f"{level.road_half_cells:g} half-width  "
               f"{level.centreline_length():.0f} cells")
        self.log.appendPlainText(row)


class LibraryDialog(QDialog):
    """The external store, and the game's own circuits, side by side."""

    levelChosen = pyqtSignal(str, str)     # (stem, origin) where origin is
                                           # 'store' or 'tracks'

    def __init__(self, store: LevelStore, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Level library")
        self.resize(720, 480)
        self.store = store

        root = QVBoxLayout(self)
        root.addWidget(QLabel(
            f"<b>Store</b> <code>{store.root}</code> &nbsp; "
            f"&mdash; levels live outside the game until you publish them."
        ))
        self.list = QListWidget()
        self.list.setSelectionMode(QAbstractItemView.SelectionMode.SingleSelection)
        self.list.itemDoubleClicked.connect(self._on_open)
        root.addWidget(self.list)

        self.detail = QLabel("Double-click to open.")
        self.detail.setWordWrap(True)
        self.detail.setTextFormat(Qt.TextFormat.RichText)
        root.addWidget(self.detail)
        self.list.currentItemChanged.connect(self._on_select)

        row = QHBoxLayout()
        b_open = QPushButton("Open")
        b_open.clicked.connect(self._on_open)
        row.addWidget(b_open)
        b_import = QPushButton("Adopt from game tracks/")
        b_import.setToolTip(
            "Import the shipped circuits' SVGs so they can be edited. "
            "The game's own copies are untouched until you publish back."
        )
        b_import.clicked.connect(self._adopt_tracks)
        row.addWidget(b_import)
        row.addStretch(1)
        root.addLayout(row)

        self.buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close)
        self.buttons.rejected.connect(self.reject)
        self.buttons.accepted.connect(self._on_open)
        root.addWidget(self.buttons)
        self.reload()

    def reload(self) -> None:
        self.list.clear()
        stored = {s.stem: s for s in self.store.levels()}
        for stem in sorted(stored):
            s = stored[stem]
            flags = []
            if self.store.is_published(stem):
                flags.append("in game")
            if s.level.meta.get("difficulty"):
                flags.append(s.level.meta["difficulty"])
            item = QListWidgetItem(
                f"{s.level.name}   "
                f"[{s.level.road_half_cells:g} half, {s.level.mode}, "
                f"{s.level.centreline_length():.0f} cells]"
                + (f"   {'  '.join(flags)}" if flags else "")
            )
            item.setData(Qt.ItemDataRole.UserRole, (stem, "store"))
            self.list.addItem(item)

        game = [
            f[:-4] for f in sorted(os.listdir(C.TRACKS_DIR))
            if f.endswith(".svg") and f[:-4] not in stored
        ] if os.path.isdir(C.TRACKS_DIR) else []
        for stem in game:
            item = QListWidgetItem(f"{stem.replace('_', ' ').title()}   [in game]")
            item.setData(Qt.ItemDataRole.UserRole, (stem, "tracks"))
            self.list.addItem(item)

    def _on_select(self, current, _previous) -> None:
        if current is None:
            return
        stem, origin = current.data(Qt.ItemDataRole.UserRole)
        try:
            if origin == "store":
                lv = self.store.load(stem)
                marks = len(lv.marks)
                gates = len(lv.gate_marks())
                pads = sum(1 for m in lv.marks if m.kind == "boost")
                self.detail.setText(
                    f"<b>{lv.name}</b> &nbsp; {len(lv.nodes)} nodes, "
                    f"{lv.mode} centreline, {lv.centreline_length():.0f} cells long<br>"
                    f"road {int(lv.road_half_cells * 2)} cells wide, "
                    f"runoff {C.CODE_NAME[lv.runoff].lower()}<br>"
                    f"{gates} gates, {pads} boost pads, {marks} features total"
                )
            else:
                self.detail.setText(
                    f"<b>{stem.replace('_', ' ').title()}</b><br>"
                    f"Read from <code>tracks/{stem}.svg</code>. Opening it "
                    f"recovers the centreline and features; saving goes to the "
                    f"store, not back to the game."
                )
        except (ValueError, OSError) as exc:
            self.detail.setText(f"<b>Could not read this level.</b><br>{exc}")

    def _on_open(self) -> None:
        item = self.list.currentItem()
        if item is None:
            return
        stem, origin = item.data(Qt.ItemDataRole.UserRole)
        self.levelChosen.emit(stem, origin)
        self.accept()

    def _adopt_tracks(self) -> None:
        try:
            levels = self.store.import_from_game_tracks()
        except (ValueError, OSError) as exc:
            QMessageBox.warning(self, "Import failed", str(exc))
            return
        for lv in levels:
            if not self.store.exists(lv.stem):
                self.store.save(lv)
        self.reload()
        self.detail.setText(
            f"Adopted {len(levels)} circuit(s) from tracks/ into the store. "
            f"They are editable now; the game's copies are unchanged."
        )


def _fresh_seed() -> int:
    import random

    return random.randrange(1, 2 ** 31 - 1)