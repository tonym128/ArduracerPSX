"""The external level store.

Levels live **outside** the game tree by default, in a directory of `.ardlvl`
files. That separation is the point, not a convenience:

* `tracks/` is the *published* set. Everything in it becomes a `TrackDef` in
  `levels.rs`, and `build_atlas` fills `ALL_TRACKS` round-robin from the
  circuits it finds -- more circuits than slots is a hard error, refused at
  build time rather than discovered in the track-select carousel. A hundred
  generated circuits parked in `tracks/` would therefore be a broken build.
* The store can hold a thousand. They are compiled, validated and previewed on
  demand, and one gets published when it is worth playing.

A batch is just a set of store entries plus a manifest, so "generate fifty
circuits, keep the twelve that validate" is a directory listing and not a
bookkeeping problem.
"""

from __future__ import annotations

import json
import os
import shutil
import time
from dataclasses import dataclass
from datetime import datetime, timezone

from . import constants as C
from .model import Level

SUFFIX = ".ardlvl"
MANIFEST = "batch.json"


@dataclass
class StoredLevel:
    """A level on disk, with the metadata the store keeps beside it."""
    level: Level
    path: str
    modified: float

    @property
    def stem(self) -> str:
        return self.level.stem

    def age_days(self) -> float:
        return (time.time() - self.modified) / 86400.0


class LevelNotPublishable(Exception):
    """A level the game would not accept, refused before anything is written.

    Its own type so the CLI can print the reason without a traceback and the
    window can show it as a dialog, while a genuine crash still looks like one.
    """


class LevelStore:
    """A directory of `.ardlvl` files, with a batch manifest alongside."""

    def __init__(self, root: str = C.DEFAULT_STORE):
        self.root = os.path.abspath(root)

    # --- layout -------------------------------------------------------------

    def path_for(self, stem: str) -> str:
        return os.path.join(self.root, f"{stem}{SUFFIX}")

    @property
    def build_dir(self) -> str:
        return os.path.join(self.root, "build")

    @property
    def batch_dir(self) -> str:
        return os.path.join(self.root, "batches")

    def ensure(self) -> "LevelStore":
        for d in (self.root, self.build_dir, self.batch_dir):
            os.makedirs(d, exist_ok=True)
        return self

    # --- queries ------------------------------------------------------------

    def stems(self) -> list[str]:
        if not os.path.isdir(self.root):
            return []
        return sorted(
            f[: -len(SUFFIX)] for f in os.listdir(self.root)
            if f.endswith(SUFFIX)
        )

    def exists(self, stem: str) -> bool:
        return os.path.exists(self.path_for(stem))

    def levels(self) -> list[StoredLevel]:
        out: list[StoredLevel] = []
        for stem in self.stems():
            path = self.path_for(stem)
            try:
                level = Level.load(path)
            except (ValueError, OSError, json.JSONDecodeError):
                # A corrupt or half-written file must not take the whole store
                # down with it. Losing one level is recoverable; losing the
                # list of four hundred is not.
                continue
            out.append(StoredLevel(level=level, path=path,
                                   modified=os.path.getmtime(path)))
        return out

    def load(self, stem: str) -> Level:
        return Level.load(self.path_for(stem))

    def load_path(self, path: str) -> Level:
        """Open a level from anywhere, and adopt it into the store on save."""
        level = Level.load(path)
        level.meta.setdefault("adopted_from", os.path.abspath(path))
        return level

    # --- mutation -----------------------------------------------------------

    def save(self, level: Level, path: str | None = None) -> str:
        target = path or self.path_for(level.stem)
        level.save(target)
        return target

    def delete(self, stem: str) -> bool:
        path = self.path_for(stem)
        if not os.path.exists(path):
            return False
        os.remove(path)
        return True

    def rename(self, old_stem: str, new_stem: str) -> str:
        src = self.path_for(old_stem)
        dst = self.path_for(new_stem)
        if src == dst:
            return dst
        if os.path.exists(dst):
            raise FileExistsError(f"{new_stem} already exists in the store")
        os.rename(src, dst)
        for ext in (".svg", ".data.png", ".visual.png", ".line.json",
                    ".palette.json", ".level.json"):
            for base in (self.build_dir, C.TRACKS_DIR):
                a = os.path.join(base, old_stem + ext)
                b = os.path.join(base, new_stem + ext)
                if os.path.exists(a) and not os.path.exists(b):
                    shutil.move(a, b)
        return dst

    def import_svg(self, svg_path: str, name: str | None = None) -> Level:
        """Re-import a circuit SVG and save it to the store."""
        from . import svgio

        level = svgio.load_svg(svg_path, name=name)
        return level

    def import_from_game_tracks(self, stems: list[str] | None = None) -> list[Level]:
        """Adopt the game's own circuits into the store, editable.

        This is what "load and change any level in the game" means in practice:
        open `tracks/Willow_Creek.svg`, recover the centreline, and edit it.
        The store's copy is the editable one; the game keeps its own until the
        edited level is published back.
        """
        from . import svgio

        targets = stems or [
            f[:-4] for f in sorted(os.listdir(C.TRACKS_DIR)) if f.endswith(".svg")
        ]
        out: list[Level] = []
        for stem in targets:
            path = os.path.join(C.TRACKS_DIR, f"{stem}.svg")
            if not os.path.exists(path):
                continue
            level = svgio.load_svg(path)
            level.meta.setdefault("adopted_from", path)
            out.append(level)
        return out

    # --- compiled output ----------------------------------------------------

    def compiled_paths(self, stem: str) -> dict[str, str]:
        return {
            "svg": os.path.join(self.build_dir, f"{stem}.svg"),
            "data": os.path.join(self.build_dir, f"{stem}.data.png"),
            "visual": os.path.join(self.build_dir, f"{stem}.visual.png"),
            "line": os.path.join(self.build_dir, f"{stem}.line.json"),
            "palette": os.path.join(self.build_dir, f"{stem}.palette.json"),
            "level": os.path.join(self.build_dir, f"{stem}.level.json"),
        }

    def is_compiled(self, stem: str) -> bool:
        return os.path.exists(self.compiled_paths(stem)["data"])

    def build(self, level: Level, *, visual: bool = False) -> object:
        """Compile into the store's own `build/` directory.

        Kept separate from publishing so a level can be built and checked
        without appearing in the game's track list. `build/` is git-ignored by
        convention: it is derived, and `tracks/` is the source of truth for the
        game.
        """
        from .raster import compile_level

        self.ensure()
        return compile_level(level, self.build_dir, with_visual=visual)

    def is_published(self, stem: str) -> bool:
        return os.path.exists(os.path.join(C.TRACKS_DIR, f"{stem}.data.png"))

    def publish(self, level: Level, *, visual: bool = True) -> object:
        """Compile into `tracks/`, which is what `make atlas` compiles.

        Refuses a level the game would not accept, rather than writing it and
        letting `make test` fail on it. The two failures are minutes apart and
        in different places, and only one of them names the road width.
        """
        from . import validate as V
        from .raster import compile_level

        floor = C.min_half_width()
        if level.road_half_cells * C.WORLD_PER_CELL < floor:
            raise LevelNotPublishable(
                f"'{level.name}' is {int(level.road_half_cells * 2)} cells "
                f"across; a car cannot fit across a road narrower than "
                f"{floor} world units of half-width "
                f"(`MIN_HALF_WIDTH` in tools/test_game_logic, "
                f"`MIN_ROAD_HALF_WIDTH` in arduracer_core::track -- one lane). "
                f"It draws and rasterises perfectly, but `make test` fails on "
                f"any track narrower than that. Widen the road to at least "
                f"{C.road_half_label(floor / C.WORLD_PER_CELL)}, or keep the level "
                f"in the store."
            )
        report = V.validate_level(level)
        if not report.ok:
            raise LevelNotPublishable(
                f"'{level.name}' does not compile:\n"
                + "\n".join(f"  • {e}" for e in report.errors)
            )
        return compile_level(level, C.TRACKS_DIR, with_visual=visual)

    def unpublish(self, stem: str) -> list[str]:
        """Remove a level from the game's track list, keeping the store copy.

        Every artefact `build_atlas` reads is removed. A half-published level
        is worse than either state: `build_atlas` refuses when a circuit
        fails, so removing only the `.data.png` leaves a level whose source
        SVG still gets rasterised into a phantom `TrackDef`.
        """
        removed: list[str] = []
        for ext in (".data.png", ".visual.png", ".visual.gen.png", ".line.json",
                    ".palette.json", ".level.json", ".svg", ".jpg", ".qoi",
                    ".arsv"):
            path = os.path.join(C.TRACKS_DIR, f"{stem}{ext}")
            if os.path.exists(path):
                os.remove(path)
                removed.append(path)
        return removed

    # --- batches ------------------------------------------------------------

    def batch_path(self, name: str) -> str:
        from .model import safe_stem

        return os.path.join(self.batch_dir, f"{safe_stem(name)}.json")

    def write_batch(self, name: str, levels: list[Level], **extra) -> str:
        """Record a named batch: which levels, from which seed, with what settings."""
        self.ensure()
        payload = {
            "name": name,
            "created": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "count": len(levels),
            "levels": [
                {
                    "stem": lv.stem,
                    "name": lv.name,
                    "difficulty": lv.meta.get("difficulty"),
                    "seed": lv.meta.get("seed"),
                    "road_half_cells": lv.road_half_cells,
                    "mode": lv.mode,
                    "nodes": len(lv.nodes),
                    "centreline_length_cells": round(lv.centreline_length(), 2),
                }
                for lv in levels
            ],
            **extra,
        }
        path = self.batch_path(name)
        tmp = f"{path}.tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(payload, f, indent=2, ensure_ascii=False)
            f.write("\n")
        os.replace(tmp, path)
        return path

    def read_batch(self, name: str) -> dict | None:
        path = self.batch_path(name)
        if not os.path.exists(path):
            return None
        with open(path, encoding="utf-8") as f:
            return json.load(f)

    def batches(self) -> list[dict]:
        if not os.path.isdir(self.batch_dir):
            return []
        out = []
        for f in sorted(os.listdir(self.batch_dir)):
            if not f.endswith(".json"):
                continue
            with open(os.path.join(self.batch_dir, f), encoding="utf-8") as fh:
                try:
                    out.append(json.load(fh))
                except json.JSONDecodeError:
                    continue
        return out

    def batch_levels(self, name: str) -> list[Level]:
        data = self.read_batch(name)
        if not data:
            return []
        out = []
        for row in data.get("levels", []):
            stem = row.get("stem")
            if stem and self.exists(stem):
                try:
                    out.append(self.load(stem))
                except (ValueError, json.JSONDecodeError):
                    continue
        return out


def default_store() -> LevelStore:
    """The store, honouring `ARDURACER_LEVELS` if it is set."""
    return LevelStore(os.environ.get("ARDURACER_LEVELS", C.DEFAULT_STORE))