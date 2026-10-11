#!/usr/bin/env python3
"""Arduracer PSX level editor -- headless CLI.

Everything the window does, without a display. Batch generation and CI
validation run here; the GUI is for drawing.

    python3 tools/level_editor/level_editor_cli.py new    --name "Test" --difficulty hard
    python3 tools/level_editor/level_editor_cli.py gen    --count 24 --seed 7
    python3 tools/level_editor/level_editor_cli.py check  --all
    python3 tools/level_editor/level_editor_cli.py build  --all --out tracks
    python3 tools/level_editor/level_editor_cli.py import tracks/Willow_Creek.svg
    python3 tools/level_editor/level_editor_cli.py publish --name "Test"

Exit status is 0 when every level checked valid and 1 when any failed, so a
Makefile target can gate on it.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from lecore import constants as C          # noqa: E402
from lecore import generate as gen         # noqa: E402
from lecore import svgio                   # noqa: E402
from lecore.model import Level             # noqa: E402
from lecore.project import (                  # noqa: E402
    LevelNotPublishable, LevelStore, default_store,
)


def _store(args) -> LevelStore:
    return LevelStore(args.store) if args.store else default_store()


def _pick(store: LevelStore, names: list[str]) -> list[Level]:
    """Named levels, or the whole store when no names are given."""
    if not names:
        return [s.level for s in store.levels()]
    out: list[Level] = []
    for name in names:
        try:
            out.append(store.load_path(name) if name.endswith(".ardlvl")
                       else store.load(name))
        except (ValueError, OSError) as exc:
            print(f"  ! {name}: {exc}", file=sys.stderr)
    return out


def _describe(lv: Level) -> str:
    return (f"{lv.name:24} {lv.road_half_cells:g} half, {lv.mode:6}, "
            f"{len(lv.nodes):3d} nodes, {lv.centreline_length():6.1f} cells, "
            f"{len(lv.gate_marks())} gates, "
            f"{sum(1 for m in lv.marks if m.kind == 'boost')} pads")


# --- commands ------------------------------------------------------------------


def cmd_new(args) -> int:
    store = _store(args)
    avoid = {s.stem for s in store.levels()}
    level = gen.generate(
        gen.GenParams(
            seed=args.seed if args.seed is not None else _random_seed(),
            difficulty=args.difficulty,
            name=args.name or "",
            mode=args.mode,
            road_half_cells=args.width,
            boost_pads=args.boost,
            oil_slicks=args.oil,
        ),
        avoid=avoid,
    )
    path = store.save(level)
    print(f"created {path}")
    print("  " + _describe(level))
    if args.build:
        r = store.build(level)
        _print_result(r)
        return 0 if r.ok else 1
    return 0


def cmd_gen(args) -> int:
    store = _store(args)
    pool = [args.difficulty] if args.difficulty else None
    widths = None
    if args.widths:
        widths = [float(w) for w in args.widths.split(",")]
    t0 = time.time()
    levels = gen.generate_batch(
        args.count,
        seed=args.seed if args.seed is not None else _random_seed(),
        store=store.root,
        difficulties=pool,
        road_widths=widths,
        modes=[args.mode] if args.mode else None,
        prefix=args.prefix or "",
        on_progress=_progress if args.verbose else None,
    )
    print(f"\ngenerated {len(levels)} circuit(s) into {store.root} "
          f"in {time.time() - t0:.1f}s")
    for lv in levels:
        print("  " + _describe(lv))
    if args.batch:
        path = store.write_batch(args.batch, levels, seed=args.seed)
        print(f"recorded batch as {path}")
    if args.check:
        return cmd_check(argparse.Namespace(**{**vars(args), "names": [],
                                              "all": True, "build": False,
                                              "out": None}))
    return 0


def cmd_check(args) -> int:
    store = _store(args)
    levels = _pick(store, args.names)
    if not levels:
        print("no levels to check")
        return 0
    from lecore.raster import compile_level

    bad = 0
    for lv in levels:
        r = compile_level(lv, write=False)
        if not r.ok:
            bad += 1
        mark = "ok  " if r.ok else "FAIL"
        print(f"{mark} {_describe(lv)}")
        for e in r.report.errors:
            print(f"       error: {e}")
        for w in r.report.warnings:
            print(f"       warn:  {w}")
    print(f"\n{len(levels) - bad}/{len(levels)} valid")
    return 1 if bad else 0


def cmd_build(args) -> int:
    store = _store(args)
    levels = _pick(store, args.names)
    if not levels:
        print("no levels to build")
        return 0
    outdir = args.out or store.build_dir
    from lecore.raster import compile_level

    bad = 0
    for lv in levels:
        r = compile_level(lv, outdir, with_visual=not args.no_visual)
        if not r.ok:
            bad += 1
        print(f"{'ok  ' if r.ok else 'FAIL'} {_describe(lv)}")
        for e in r.report.errors:
            print(f"       error: {e}")
    print(f"\nwrote {len(levels) - bad}/{len(levels)} to {outdir}")
    return 1 if bad else 0


def cmd_publish(args) -> int:
    store = _store(args)
    levels = _pick(store, args.names)
    if not levels:
        print("no levels to publish")
        return 0
    from lecore.raster import compile_level

    slots = _slots()
    # Counted the way the build counts them -- by asking `discover_circuits`
    # what it will find in `tracks/` -- rather than by counting store entries
    # marked published. Those are not the same set: the twenty-four circuits
    # from `generate_svg_circuit.CIRCUITS` are published too, and none of them
    # is in the store unless somebody adopted it.
    already = len(_discovered())

    replacements = list(args.replace or [])
    freed = 0
    for stem in replacements:
        stem = os.path.splitext(os.path.basename(stem))[0]
        if not store.is_published(stem):
            print(f"  ! --replace {stem}: it is not in tracks/, nothing to free",
                  file=sys.stderr)
            continue
        freed += len(store.unpublish(stem))
        print(f"replaced {stem}: removed its files from tracks/")

    overflow = already - freed + len(levels) - slots
    if overflow > 0:
        print(
            f"refusing to publish {len(levels)} level(s): the game has {slots} "
            f"track slots, {already} are in tracks/, and --replace freed "
            f"{freed}, leaving {overflow} too many. `ALL_TRACKS` is filled "
            f"round-robin and build_atlas.py refuses outright when there are "
            f"more circuits than slots.\n\n"
            f"  --replace <Stem> [<Stem>...]   unpublish that circuit first, "
            f"and publish these in its place.",
            file=sys.stderr,
        )
        return 1
    bad = 0
    for lv in levels:
        try:
            store.publish(lv, visual=True)
        except LevelNotPublishable as exc:
            bad += 1
            print(f"FAIL {lv.stem}: {exc}", file=sys.stderr)
            continue
        print(f"ok   {lv.stem} -> {C.TRACKS_DIR}")
    print(f"\n{len(levels) - bad}/{len(levels)} published. Run: make atlas")
    return 1 if bad else 0


def cmd_unpublish(args) -> int:
    store = _store(args)
    for name in args.names:
        stem = os.path.splitext(os.path.basename(name))[0]
        removed = store.unpublish(stem)
        print(f"removed {len(removed)} file(s) for {stem}")
    return 0


def cmd_import(args) -> int:
    store = _store(args)
    paths: list[str] = list(args.paths)
    if args.game:
        paths += [os.path.join(C.TRACKS_DIR, f)
                  for f in sorted(os.listdir(C.TRACKS_DIR)) if f.endswith(".svg")]
    if not paths:
        print("nothing to import; pass paths or --game", file=sys.stderr)
        return 1
    n = 0
    for p in paths:
        try:
            level = svgio.load_svg(p)
        except (ValueError, OSError) as exc:
            print(f"  ! {p}: {exc}", file=sys.stderr)
            continue
        level.meta.setdefault("adopted_from", os.path.abspath(p))
        if store.exists(level.stem) and not args.force:
            print(f"  = {level.name} already in the store; skipped")
            continue
        store.save(level)
        print(f"  + {_describe(level)}")
        n += 1
    print(f"imported {n} circuit(s) into {store.root}")
    return 0


def cmd_list(args) -> int:
    store = _store(args)
    levels = store.levels()
    if not levels:
        print(f"no levels in {store.root}")
        return 0
    for s in levels:
        flags = []
        if store.is_published(s.stem):
            flags.append("in game")
        if store.is_compiled(s.stem):
            flags.append("built")
        print(f"{s.stem:24} " + _describe(s.level)
              + (f"   [{', '.join(flags)}]" if flags else ""))
    print(f"\n{len(levels)} level(s) in {store.root}")
    for b in store.batches():
        print(f"batch {b.get('name', '?')}: {b.get('count', '?')} circuits")
    return 0


def cmd_show(args) -> int:
    store = _store(args)
    level = store.load_path(args.level) if args.level.endswith(".ardlvl") \
        else store.load(args.level)
    print(json.dumps(level.descriptor(), indent=2))
    return 0


def cmd_export(args) -> int:
    store = _store(args)
    level = store.load_path(args.level) if args.level.endswith(".ardlvl") \
        else store.load(args.level)
    svgio.save_svg(level, args.out)
    print(f"wrote {args.out}")
    return 0


# --- helpers -------------------------------------------------------------------


def _random_seed() -> int:
    import random

    return random.randrange(1, 2 ** 31 - 1)


def _progress(done: int, total: int, level: Level) -> None:
    print(f"\r  {done}/{total}  {level.name:<28}", end="", flush=True)


def _slots() -> int:
    """How many track slots the game has.

    Read out of `build_atlas.TRACK_SLOTS` rather than restated: it is the
    number `build_atlas` compares against, and publishing past it is the one
    way to make a level set that builds cleanly here and fails the game build.
    """
    try:
        return int(C.track_cook_module("build_atlas").TRACK_SLOTS)
    except Exception:                                  # noqa: BLE001
        return 24


def _discovered() -> list[str]:
    """Circuit names `build_atlas` will compile, straight from its own logic."""
    try:
        ba = C.track_cook_module("build_atlas")
        return [n for n, _half in ba.discover_circuits()]
    except Exception:                                  # noqa: BLE001
        return []


def _print_result(r) -> None:
    print(f"  {'valid' if r.ok else 'INVALID'} in {r.seconds:.2f}s")
    for e in r.report.errors:
        print(f"    error: {e}")
    for w in r.report.warnings:
        print(f"    warn:  {w}")
    for k, v in sorted(r.files.items()):
        print(f"    {k:7} {v}")


# --- argument parsing ----------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(
        prog="level_editor_cli",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--store", default=None,
                    help=f"level library (default: {C.DEFAULT_STORE}, or "
                         f"$ARDURACER_LEVELS)")
    ap.add_argument("--no-checks", action="store_true",
                    help="skip the format consistency check")
    sub = ap.add_subparsers(dest="command", required=True)

    def add_road(p):
        p.add_argument(
            "--width", type=float, default=3.0,
            choices=list(C.ROAD_HALF_CHOICES), metavar="CELLS_HALF",
            help="tarmac half-width in cells (default 3, i.e. 6 cells wide)",
        )

    p = sub.add_parser("new", help="create one random circuit in the store")
    p.add_argument("--name", default="")
    p.add_argument("--difficulty", default="medium", choices=list(gen.DIFFICULTIES))
    p.add_argument("--mode", default="spline", choices=["spline", "poly"])
    add_road(p)
    p.add_argument("--boost", type=int, default=3)
    p.add_argument("--oil", type=int, default=1)
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--build", action="store_true", help="compile it in the store")
    p.set_defaults(func=cmd_new)

    p = sub.add_parser("gen", help="generate a batch of circuits")
    p.add_argument("--count", type=int, default=12)
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--difficulty", default=None, choices=list(gen.DIFFICULTIES),
                   help="default: cycle through all four")
    p.add_argument("--widths", default=None,
                   help="comma-separated half-widths to cycle, e.g. 2,3,4")
    p.add_argument("--mode", default=None, choices=["spline", "poly"])
    p.add_argument("--prefix", default="")
    p.add_argument("--batch", default="", help="record the run under this name")
    p.add_argument("--check", action="store_true",
                   help="validate every one afterwards")
    p.add_argument("--verbose", action="store_true")
    p.set_defaults(func=cmd_gen)

    p = sub.add_parser("check", help="validate levels against the compiler's rules")
    p.add_argument("names", nargs="*")
    p.add_argument("--all", action="store_true", help="the whole store")
    p.add_argument("--build", action="store_true", help=argparse.SUPPRESS)
    p.add_argument("--out", default=None, help=argparse.SUPPRESS)
    p.set_defaults(func=cmd_check)

    p = sub.add_parser("build", help="rasterise levels to the format the game reads")
    p.add_argument("names", nargs="*")
    p.add_argument("--all", action="store_true")
    p.add_argument("--out", default=None,
                   help="output directory (default: the store's build/)")
    p.add_argument("--no-visual", action="store_true",
                   help="skip the 1024px appearance image (it is the slow part)")
    p.set_defaults(func=cmd_build)

    p = sub.add_parser("publish", help="write levels into tracks/, for make atlas")
    p.add_argument("names", nargs="*")
    p.add_argument("--all", action="store_true")
    p.add_argument("--replace", nargs="+", default=[], metavar="STEM",
                   help="unpublish these circuits first, to free track slots")
    p.set_defaults(func=cmd_publish)

    p = sub.add_parser("unpublish", help="remove levels from tracks/")
    p.add_argument("names", nargs="+")
    p.set_defaults(func=cmd_unpublish)

    p = sub.add_parser("import", help="re-import circuit SVGs as editable levels")
    p.add_argument("paths", nargs="*")
    p.add_argument("--game", action="store_true",
                   help="import every circuit in tracks/")
    p.add_argument("--force", action="store_true")
    p.set_defaults(func=cmd_import)

    p = sub.add_parser("list", help="what is in the store")
    p.set_defaults(func=cmd_list)

    p = sub.add_parser("show", help="print a level's descriptors as JSON")
    p.add_argument("level")
    p.set_defaults(func=cmd_show)

    p = sub.add_parser("export", help="write a level's SVG")
    p.add_argument("level")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_export)

    return ap


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if not args.no_checks:
        try:
            C.assert_agrees_with_pipeline()
        except AssertionError as exc:
            print(f"level editor: {exc}", file=sys.stderr)
            return 2
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())