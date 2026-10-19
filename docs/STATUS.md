# Documentation Index — what to read, what's authoritative

The repository keeps a lot of planning docs at the root. This index says which
are **current**, which are **point-in-time**, and which are **superseded**, so
you do not implement against a stale plan.

> If a doc here contradicts `AGENTS.md` or the `Makefile`, those two win. Fix
> the doc in the same change.

## Start here (authoritative)

| Doc | What it is | Status |
| --- | --- | --- |
| [`AGENTS.md`](../AGENTS.md) | The engineering/review protocol: layer boundaries, build & verify commands, worktree workflow, 4-perspective review gate. | **Current — authoritative** |
| [`GAME.md`](../GAME.md) | The game itself: features, controls, content. Also the basis of the release manual. | **Current** |
| `track*.md` (e.g. [`trackit.md`](../trackit.md)) | Per-worktree backlog for the active branch, with working rules and a progress log. Read the one for your branch. | **Current — branch-scoped** |
| [`docs/REVIEW-TEMPLATE.md`](REVIEW-TEMPLATE.md) | Fill-in template for the §4 multi-perspective review. | **Current** |
| [`docs/PHYSICS-HANDLING.md`](PHYSICS-HANDLING.md) | Handling model used by `arduracer-core` + a repeatable baseline. | **Current** |
| [`LEVEL-FORMAT.md`](../LEVEL-FORMAT.md) | The decision on how circuits are authored (colour-coded images). | **Current decision** |
| [`Makefile`](../Makefile) | The only build entry point. `make help` lists every target. | **Current — authoritative** |

## Point-in-time records (do not treat as live plans)

| Doc | What it is | Status |
| --- | --- | --- |
| [`REVIEW.md`](../REVIEW.md) | Four-perspective audit + sign-off for one task (TASK-1001). | Historical |
| [`AUDIT-2026-10.md`](../AUDIT-2026-10.md) | Code audit, top-20 issues, on `review/audit-2026-10`. | Historical snapshot |
| [`docs/FRAME-PROFILING-2026-10.md`](FRAME-PROFILING-2026-10.md) | Frame-time profiling method and numbers for Oct 2026. | Historical snapshot |

## Proposals / not-yet-shipped

| Doc | What it is | Status |
| --- | --- | --- |
| [`TODO.md`](../TODO.md) | Roadmap, parallel worktree assignments, review milestones. | Proposal / tracker |
| [`NO1.md`](../NO1.md) | "Make it the #1 TrackMania-style racer" blueprint (worktree `.worktrees/no1`). | Proposal |
| [`OVERHAUL.md`](../OVERHAUL.md) | Track/camera/race-start overhaul proposals, each with a "done" condition. | Proposal |
| [`AI_IMAGE_PROMPTS.md`](../AI_IMAGE_PROMPTS.md) | Prompts for stage illustrations ("24 stages"). | Proposal — check count before use |

## Superseded

| Doc | What it is | Status |
| --- | --- | --- |
| [`STAGE-ATLAS.md`](../STAGE-ATLAS.md) | Earlier tile-grid / parametric stage authoring model. | **Superseded in part** — circuits are now colour-coded images; see `LEVEL-FORMAT.md` |

## Release / packaging

| Doc | What it is |
| --- | --- |
| [`packaging/MANUAL.md`](../packaging/MANUAL.md) | Instruction manual (ships in the release). |
| [`packaging/DISC_ART.md`](../packaging/DISC_ART.md) | Disc-surface silkscreen artwork spec. |
| [`packaging/JEWEL_CASE.md`](../packaging/JEWEL_CASE.md) | Jewel-case packaging spec. |

## Generated outputs — never hand-edit

- `crates/arduracer-core/src/levels.rs` and `visual_tex.rs` — emitted by `make atlas` (`tools/track_cook/build_atlas.py`). Editing these files directly is denied by `opencode.json`.
- Disc assets under `assets/**`, cooked levels in `levels/`, and images in `build/circuits/` are pipeline outputs.
