# Vendored PSoXide SDK

The PSX runtime this game is built against. It is **committed here**, not
fetched, because in the upstream project it is a *generated* component that
exists on no remote — see REVIEW.md §1c for the full investigation.

## What this is

| | |
| :--- | :--- |
| Upstream repository | `EBonura/PSoXide-emulator` |
| Component | `sdk`, pinned by `components.lock.json` to `EBonura/PSoXide` @ `3a7c21a05c67579e01c8146ec112ab697a3b769a` |
| Licence | GPL-2.0-or-later (same as this project) |
| Vendored subset | `sdk/` (all 20 crates), `crates/psx-hw`, `crates/psx-iso`, `crates/psxed-format`, `tools/mkisopsx` |
| Size | ~6.2 MB, 301 files |

The subset is what `game/Cargo.toml` and `tools/test_memcard` reference through
path dependencies, plus `tools/mkisopsx` for `make disc`. `psx-hw`, `psx-iso` and
`psxed-format` are pulled in because `sdk/crates/*` depend on them by relative
path. The emulator, editor, assets and docs are **not** vendored.

`Cargo.toml` in this directory is not upstream's: upstream's root lists emulator
crates that are not here. It is a reduced member list with the
`[workspace.package]`, `[workspace.lints]` and `[workspace.dependencies]`
sections copied verbatim from upstream. `sdk/Cargo.toml` is upstream's, unedited.

## Local modifications on top of the pin

Of the 299 pinned files in this subset, 289 are byte-identical to the pin and
**ten carry local modifications**. They are not cosmetic — the FMV and CD-audio
paths depend on them:

| File | Why it matters here |
| :--- | :--- |
| `sdk/crates/psx-vram/src/lib.rs` | adds `upload_words_with`, servicing a polled device mid-upload so long VRAM writes do not starve the CD sector stream |
| `sdk/crates/psx-rt/src/interrupts.rs` | clears the CPU-side CDROM latch so an enabled CD IRQ does not become an interrupt storm |
| `sdk/crates/psx-pack/src/cd.rs` | XA-ADPCM SetMode handling (`prepare_xa`) |
| `sdk/crates/psx-fmv/src/mdec.rs` | MDEC decode changes used by the intro FMV |
| `sdk/crates/psx-io/src/cdrom.rs` | `select_index` moved next to its callers |
| `sdk/crates/psx-io/src/lib.rs` | re-exports |
| `sdk/crates/psx-fmv/tests/encoder_stream.rs` | test coverage |
| `crates/psx-hw/src/lib.rs` | register access |
| `crates/psx-iso/src/boot.rs`, `crates/psx-iso/src/lib.rs` | disc/ISO paths used by `mkisopsx` |

Two further files are **local additions** not present in the pin at all:
`sdk/crates/psx-io/src/mdec.rs` and `crates/psx-hw/src/mdec.rs`. They are
required by the `pub mod mdec;` declarations in the modified `lib.rs` files
above, so the pinned revision does not build this game and they must stay.

To verify a tree against the pin, from a checkout of
`EBonura/PSoXide-emulator`:

```sh
python3 tools/bootstrap-components.py --check
```

That check **fails on purpose** for this vendored copy — it compares against the
pinned revision and these ten files differ.

## Updating

1. Change the SDK in your `PSoXide-emulator` checkout as normal.
2. Re-copy the affected paths:
   ```sh
   SRC=/path/to/PSoXide-emulator
   rm -rf psoxide/sdk psoxide/crates psoxide/tools/mkisopsx
   mkdir -p psoxide/crates psoxide/tools
   cp -r "$SRC/sdk" psoxide/sdk
   for c in psx-hw psx-iso psxed-format; do cp -r "$SRC/crates/$c" psoxide/crates/$c; done
   cp -r "$SRC/tools/mkisopsx" psoxide/tools/mkisopsx
   find psoxide -name target -type d -prune -exec rm -rf {} +
   ```
3. `make ci` and update the table above.

The better long-term fix is to publish these ten files upstream and re-lock
`components.lock.json`, after which this directory can be deleted and
`psoxide/sdk` fetched again.