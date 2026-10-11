#!/usr/bin/env python3
"""Arduracer PSX level editor -- GUI entry point.

    python3 tools/level_editor/level_editor.py [level.ardlvl]

The format constants are checked against `tools/track_cook` before the window
opens. That check is the difference between finding out your palette drifted
now, and finding out from `make atlas` forty minutes later with a message
naming a pixel coordinate.
"""

from __future__ import annotations

import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from lecore import constants as C           # noqa: E402
from lecore.project import LevelStore, default_store   # noqa: E402


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        prog="level_editor",
        description="Draw and edit Arduracer PSX circuits.",
    )
    ap.add_argument("level", nargs="?", help="a .ardlvl file to open")
    ap.add_argument("--store", default=None,
                    help=f"level library directory (default: {C.DEFAULT_STORE})")
    ap.add_argument("--skip-checks",
                    action="store_true",
                    help="skip the format consistency check against track_cook")
    args = ap.parse_args(argv)

    if not args.skip_checks:
        try:
            C.assert_agrees_with_pipeline()
        except AssertionError as exc:
            print(f"level editor: {exc}", file=sys.stderr)
            return 2

    store = LevelStore(args.store) if args.store else default_store()
    store.ensure()

    try:
        from PyQt6.QtWidgets import QApplication
        from gui.mainwindow import MainWindow
    except ImportError as exc:
        print(
            f"level editor: the GUI needs PyQt6 ({exc}).\n"
            "  pip install PyQt6\n"
            "  Everything except the window is in level_editor_cli.py, which "
            "needs no display.",
            file=sys.stderr,
        )
        return 3

    app = QApplication(sys.argv[:1])
    app.setApplicationName("Arduracer PSX Level Editor")
    win = MainWindow(store)

    if args.level:
        path = args.level
        try:
            if path.endswith(".svg"):
                from lecore import svgio

                level = svgio.load_svg(path)
                win._adopt(level, None)
            else:
                level = store.load_path(path)
                win._adopt(level, store.path_for(level.stem))
        except (ValueError, OSError) as exc:
            print(f"level editor: could not open {path}: {exc}", file=sys.stderr)
            return 1

    win.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())