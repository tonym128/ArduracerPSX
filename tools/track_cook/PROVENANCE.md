# Vendored ArduRacer FX level data

The 20 legacy circuit layouts that `convert_levels.py` compiles into
`crates/arduracer-core/src/levels.rs`. `levels.rs` is *generated*, so these
files are generator inputs, not source-of-truth duplicates of it: they are
tracked precisely so the generation pipeline is reproducible from a clean
clone.

Before this was committed, `.gitignore` excluded the whole `ArduRacerFx`
reference directory and `git ls-files ArduRacerFx | wc -l` returned **0**. The
consequence was that `make calibrate-tracks` — the documented remediation for a
stale par table — could not run at all on a fresh checkout: the cooker died with
a bare `FileNotFoundError` on `ArduRacerFx/Levels/Level1.csv`, *after*
`tools/playtest --calibrate` had already rewritten `par_calibration.json`, so the
failure read as "calibration failed" rather than "you are missing the inputs".

## What this is

| | |
| :--- | :--- |
| Upstream repository | `tonym128/ArduRacerFx` |
| Pinned revision | `aea658726c08b83b26a4fe52dbd9b70e0831b305` (2023-12-12) |
| Author / copyright | Tony Mamacos, the owner of this repository |
| Licence | MIT (`ArduRacerFx/LICENSE`) |
| Vendored subset | `Levels/*.csv`, `Levels/*.tmx`, `Levels/*.tmj`, `Levels/SpriteSheet*.{png,aseprite,tsx}`, `LICENSE` |
| Size | 47 files, 192 KB (the upstream tree is 3.9 MB) |

Every vendored file is **byte-identical to the pin**; nothing has been edited
upstream-side. Geometry changes are made in `convert_levels.py`, which is the
single source of truth for the PSX circuits — the CSVs stay as the pristine
upstream record.

The upstream Arduboy tree is *not* vendored: not `racer.cpp`, not
`ArduRacerFx.ino`, not the tone tables, not the packed `fxdata*.bin` blobs, not
the screenshots or firmware images in `Extra/`, and not the superseded level
drafts in `Levels/Backup/`. See the `ArduRacerFx/*` block in `.gitignore`.

## The nested clone is gone

Upstream arrives as a *nested git repository*. Git treats a directory holding
its own `.git` as an embedded repository and refuses to track its contents, so
the vendored copy has no `.git` — the upstream revision is recorded in the table
above instead, which is the usual vendoring trade.

**Practical note for anyone who keeps a live `ArduRacerFx` clone:** checkout this
branch into a worktree that already has one and git will refuse to overwrite
those untracked files. Remove or move the nested clone first
(`mv ArduRacerFx ../ArduRacerFx.fx-clone`); the tracked subset can then be
restored with `git checkout`.