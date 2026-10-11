"""Level -> the file quartet `build_atlas.py` compiles.

    <Stem>.svg           authored drawing, cell units, exact palette colours
    <Stem>.data.png      flat 4-bit-indexed palette, exact colours only
    <Stem>.visual.png    appearance, display only
    <Stem>.line.json     centreline sidecar
    <Stem>.palette.json  the exact RGB of every code
    <Stem>.level.json    descriptors, and the per-level road width

The rasterisation, cell resolution and visual painting are **not**
reimplemented here. `generate_svg_circuit.rasterise`, `.snap_to_codes`,
`.resolve_cells`, `.paint_visual` and `.compress_1024_jpeg_500kb` are imported
and called, because those are the functions the twenty-four shipped circuits
were baked with, and a second implementation of them would be a second set of
answers to "what does a kerb look like at three pixels per cell". Anything that
changes what the game draws belongs in `generate_svg_circuit.py`, where
`make circuits` and this editor both see it.
"""

from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass, field

import numpy as np
from PIL import Image

from . import constants as C
from . import geometry as G
from . import svgio
from . import validate as V
from .model import Level


@dataclass
class CompileResult:
    """What a compile produced, and what the compiler would think of it."""
    level: Level
    stem: str
    outdir: str
    files: dict[str, str] = field(default_factory=dict)
    svg_text: str = ""
    centreline: list[G.Point] = field(default_factory=list)
    codes: np.ndarray | None = None
    report: V.Report = field(default_factory=V.Report)
    seconds: float = 0.0

    @property
    def ok(self) -> bool:
        return self.report.ok

    def paths(self) -> list[str]:
        return [self.files[k] for k in sorted(self.files)]


def rasterise_codes(level: Level, svg_text: str | None = None) -> tuple[np.ndarray, list[G.Point], str]:
    """SVG -> resolved cell grid, at the data image's supersampled resolution.

    Anti-aliasing is the trap this exists to survive. Every rasteriser blends
    edges, and a single blended pixel makes a cell unreadable --
    `compile_circuit.load_data` rejects the whole circuit with "matches no
    palette code". Taking the raster at `SUPERSAMPLE` times the cell resolution
    and resolving each cell by majority vote over its own pixels is what makes
    a flat data image legal without post-hoc colour snapping, which would be a
    lie about the drawing.
    """
    gen = C.track_cook_module("generate_svg_circuit")
    if svg_text is None:
        svg_text, centre = svgio.build_svg(level)
    else:
        _, centre = svgio.build_svg(level)

    fine = gen.rasterise(svg_text, C.DATA_PX * C.SUPERSAMPLE)
    codes = gen.resolve_cells(fine, C.DATA_PX * C.SUPERSAMPLE)
    return codes, centre, svg_text


def write_data_png(codes: np.ndarray, path: str) -> str:
    """The flat palette image, built from the code map rather than from the raster.

    Indexed through a lookup table rather than by matching the raster back to
    colours: the data image must be *exactly* palette colours, and building it
    from the code map guarantees that rather than hoping the rasteriser agreed.
    """
    lut = np.zeros((16, 3), dtype=np.uint8)
    for code, rgb in C.RGB_TUPLE.items():
        lut[code] = rgb
    dat = np.repeat(np.repeat(codes, C.DATA_PX, axis=0), C.DATA_PX, axis=1)
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    Image.fromarray(lut[dat], "RGB").save(path)
    return path


def write_visual_png(codes: np.ndarray, path: str) -> str:
    """The 1024x1024 appearance image, at the resolution the disc streams.

    `paint_visual` is a per-pixel Python loop over a megapixel, so it costs
    about as long as everything else in a compile put together. That is fine
    when publishing and ruinous in an editor, which is why `with_visual`
    defaults to off and the preview path below paints at a size a person can
    look at.
    """
    gen = C.track_cook_module("generate_svg_circuit")
    vis = gen.paint_visual(codes, C.SIZE)
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    Image.fromarray(vis, "RGB").save(path)
    return path


def write_palette_json(path: str) -> str:
    payload = {
        "cell_px": C.DATA_PX,
        "grid": C.GRID,
        "visual_size": C.SIZE,
        "visual_px_per_cell": C.SIZE // C.GRID,
        "palette": {str(k): list(v) for k, v in C.RGB_TUPLE.items()},
    }
    _write_json(path, payload)
    return path


def write_line_json(level: Level, centre: list[G.Point], path: str) -> str:
    """The centreline sidecar, in cell units as the compiler expects.

    The centreline is written at full float precision, not rounded to whole
    cells. Rounding is what made the start heading collapse to due north on
    levels whose first two samples fell inside one tile: the car spawned facing
    the barrier and drove into it, with nothing in the emitted data looking
    wrong, because 0 is a perfectly valid BAM value.
    """
    _write_json(path, {
        "cell_px": C.DATA_PX,
        "grid": C.GRID,
        "levels": 1,
        "road_cells": level.road_half_cells * 2,
        "source": "svg",
        "samples": [[round(x, 3), round(y, 3), 0] for x, y in centre],
    })
    return path


def _write_json(path: str, payload: dict) -> str:
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    tmp = f"{path}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1, ensure_ascii=False)
        f.write("\n")
    os.replace(tmp, path)
    return path


def compile_level(
    level: Level,
    outdir: str = C.TRACKS_DIR,
    *,
    write: bool = True,
    with_visual: bool = False,
) -> CompileResult:
    """Rasterise, validate, and (by default) write the whole file set.

    Writing happens even when validation fails, on purpose. A level that does
    not compile is the one you most want to look at, and the four files are
    enough for `compile_circuit` to print the same error the editor just
    printed plus anything it did not check.
    """
    t0 = time.time()
    stem = level.stem
    codes, centre, svg_text = rasterise_codes(level)
    report = V.validate_level(level, codes)

    base = os.path.join(outdir, stem)
    files: dict[str, str] = {}
    if write:
        files["svg"] = _write_text(base + ".svg", svg_text)
        files["data"] = write_data_png(codes, base + ".data.png")
        files["line"] = write_line_json(level, centre, base + ".line.json")
        files["palette"] = write_palette_json(base + ".palette.json")
        files["level"] = _write_json(base + ".level.json", level.descriptor())
        if with_visual:
            files["visual"] = write_visual_png(codes, base + ".visual.png")

    report.info["centreline_samples"] = len(centre)
    report.info["centreline_length_cells"] = round(G.path_length(centre), 2)
    report.info["histogram"] = V.code_histogram(codes)

    return CompileResult(
        level=level,
        stem=stem,
        outdir=outdir,
        files=files,
        svg_text=svg_text,
        centreline=centre,
        codes=codes,
        report=report,
        seconds=time.time() - t0,
    )


def _write_text(path: str, text: str) -> str:
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    tmp = f"{path}.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text + "\n")
    os.replace(tmp, path)
    return path


def preview_png(codes: np.ndarray, size: int = 512) -> Image.Image:
    """A flat palette preview of a resolved grid. Fast, and exactly the truth.

    Nearest-neighbour upscaling of the code map rather than `paint_visual`, so
    the preview shows the cells the car will actually drive on -- kerbs a half
    cell wide and a boost pad two cells long are the difference between a road
    that reads as wide and one that reads as a car park, and smoothing them
    away in a preview hides precisely the thing being edited.
    """
    lut = np.zeros((16, 3), dtype=np.uint8)
    for code, rgb in C.RGB_TUPLE.items():
        lut[code] = rgb
    return Image.fromarray(lut[codes.astype(np.uint8)]).resize(
        (size, size), Image.Resampling.NEAREST
    )


def painted_preview(codes: np.ndarray, size: int = 256) -> Image.Image:
    """The appearance image at preview size. The look, not the physics."""
    gen = C.track_cook_module("generate_svg_circuit")
    return Image.fromarray(gen.paint_visual(codes, size))