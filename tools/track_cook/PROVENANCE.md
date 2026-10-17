# Vendored ArduRacer FX level data

The 20 legacy circuit layouts, kept as a **provenance record only**. The FX-CSV
cooker that compiled them (`convert_levels.py`) has been deleted along with the
par-table target that ran it: circuits are authored as images in `tracks/` and
compiled by `build_atlas.py`, and nothing in the build reads `ArduRacerFx/` any
more. `crates/arduracer-core/src/levels.rs` and `visual_tex.rs` cannot be
regenerated from these files.

They stay tracked because they are the pristine upstream record of where the
original circuits came from, which is worth keeping once the code that consumed
them is gone.

Before this was committed, `.gitignore` excluded the whole `ArduRacerFx`
reference directory and `git ls-files ArduRacerFx | wc -l` returned **0**. That
made the pipeline unreproducible from a clean clone, which is why the files were
committed while the cooker still existed.

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
upstream-side, and nothing edits them now. Geometry changes are made in
`generate_svg_circuit.py` and painted into `tracks/*.png`; the CSVs are only the
historical record of the pre-image circuits.

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