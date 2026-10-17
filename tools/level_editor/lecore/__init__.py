"""Arduracer PSX level editor core.

Headless, GUI-free level model, SVG author/parser, raster compiler, validator and
procedural generator. Everything the PyQt6 front end does is available here as
a plain library so batch work and CI never need a display.

The format is the game's, not ours. A level is authored as an SVG in cell units
with the exact palette colours `compile_circuit.load_data` matches by equality,
and compiled to the `.data.png` / `.visual.png` / `.line.json` /
`.palette.json` quartet `build_atlas.py` already consumes. Nothing in the
runtime changes.
"""

__version__ = "1.0.0"

SCHEMA_VERSION = 1