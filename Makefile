SHELL := /bin/bash
ROOT  := $(shell pwd)

DIST     := $(ROOT)/dist
GAME_DIR := $(ROOT)/game
GAME_EXE := $(GAME_DIR)/target/mipsel-sony-psx/release/arduracer.exe
GAME_MAP := $(DIST)/arduracer.map
MKISOPSX := $(ROOT)/psoxide/tools/mkisopsx

# The linker script is the single source of truth for the PSX RAM budget. The
# CI gate parses it rather than hard-coding a byte count, so tightening or
# relaxing the budget in one place moves the gate with it.
LINKER_SCRIPT := $(firstword $(wildcard psoxide/sdk/psoxide.ld))

# Release version, used for the distribution zip and SHA256SUMS. Defaults to the
# most recent tag so a tagged build labels itself correctly; override explicitly
# with `make release VERSION=1.2.3`. Previously hardcoded to 1.0.0, so tagging
# v1.1.0 shipped an archive still called v1.0.0.
VERSION ?= $(shell v=`git describe --tags --abbrev=0 2>/dev/null | sed 's/^v//'`; if [ -n "$$v" ]; then echo "$$v"; else echo 1.0.0; fi)

DUCKSTATION_APPIMAGES := \
	$(HOME)/Downloads/DuckStation-x64.AppImage \
	$(HOME)/Downloads/DuckStation-x86_64.AppImage \
	$(HOME)/Applications/DuckStation-x86_64.AppImage \
	/opt/duckstation/DuckStation-x86_64.AppImage

# Ceiling for text+data+bss, as a percentage of the linker script's RAM region.
# The linker itself hard-errors when the sections outgrow the region, so this is
# the *early* gate: it turns "section will not fit in region" from the middle of a
# 300-crate link into one readable line naming the real budget, and it catches a
# runaway static buffer long before it becomes a link error. The other half stays
# free for the runtime stack and any future heap.
RAM_GATE_MAX_PCT ?= 50

RETROARCH_CORE_DIRS := \
	$(HOME)/.config/retroarch/cores \
	/usr/lib/x86_64-linux-gnu/libretro \
	/usr/lib/libretro

RETROARCH_PS1_CORES := \
	duckstation_libretro.so \
	swanstation_libretro.so \
	mednafen_psx_hw_libretro.so \
	mednafen_psx_libretro.so \
	pcsx_rearmed_libretro.so

.PHONY: all help test playtest calibrate-tracks tracks clippy clippy-host clippy-game fmt fmt-check ci ci-host ci-game ci-disc exe iso disc release web clean run assets deps require-sdk require-fx-levels require-version

all: test exe

help:
	@echo "Arduracer PSX - Build Targets:"
	@echo "  make test      - Run automated host-side game logic tests (arduracer-core)"
	@echo "  make playtest  - Prove all 24 circuits are drivable (player + 5 AI rivals)"
	@echo "                  and that the stored par times still match the physics."
	@echo "                  Par drift is a FAILURE. Escape hatch for the recalibration"
	@echo "                  window only: make playtest PLAYTEST_FLAGS=--allow-par-drift"
	@echo "  make tracks    - Regenerate levels.rs from FX CSVs + par calibration"
	@echo "  make calibrate-tracks - Re-measure par times, then regenerate levels.rs"
	@echo "  make exe       - Build bare-metal MIPS PSX executable (dist/arduracer.exe)"
	@echo "  make assets    - Cook FMV intro video and CD-DA Redbook audio tracks"
	@echo "  make disc      - Master bootable PS1 disc image (dist/arduracer.bin/.cue)"
	@echo "  make iso       - Master simple data ISO (dist/arduracer.iso)"
	@echo "  make release   - Package full release ZIP and SHA256 checksums"
	@echo "  make web       - Serve Arduracer Web Arcade on http://localhost:8080"
	@echo "  make clippy    - Run clippy linting on every crate, game included (-D warnings)"
	@echo "  make fmt-check - Check code formatting across all crates"
	@echo "  make fmt       - Format code across all crates"
	@echo "  make ci        - Run CI verification (host + game). Disc mastering needs"
	@echo "                  psxavenc/ffmpeg, so it is a separate target:"
	@echo "  make ci-disc   - Master the disc and smoke-test the bootable image"
	@echo "  make run       - Run mastered disc in emulator (DuckStation / RetroArch)"
	@echo "  make clean     - Clean build outputs"
	@echo ""
	@echo "Tunables:"
	@echo "  VERSION=$(VERSION)   release archive version (default: most recent tag)"
	@echo "  RAM_GATE_MAX_PCT=$(RAM_GATE_MAX_PCT)      PSX static-RAM ceiling, percent of the linker RAM region"
	@echo "  PLAYTEST_FLAGS=...  extra flags for tools/playtest (see above)"

# --- SDK dependency gate ---------------------------------------------------
# The game and the test crates depend on the PSX runtime through path
# dependencies on `psoxide/sdk`. That tree is committed here rather than fetched:
# upstream it is a *generated* component that exists on no remote (REVIEW.md
# section 1c, psoxide/PROVENANCE.md). Every cargo command that resolves a
# workspace fails with a manifest error if it is missing, so check it up front
# and say what to do instead.
SDK_MARKER := psoxide/sdk/crates/psx-rt/Cargo.toml

.PHONY: deps require-sdk
deps:
	@if [ -f "$(SDK_MARKER)" ]; then \
		echo "SDK present: $(SDK_MARKER) (vendored; see psoxide/PROVENANCE.md)"; \
		exit 0; \
	fi; \
	echo "ERROR: the vendored PSoXide SDK is missing."; \
	echo ""; \
	echo "  $(SDK_MARKER) does not exist. It is committed in this repository,"; \
	echo "  so a correct checkout has it. Either your checkout is incomplete"; \
	echo "  (git status / git restore) or it was deleted locally."; \
	echo ""; \
	echo "  If you are migrating from a machine that had the SDK generated by"; \
	echo "  the PSoXide bootstrap, see psoxide/PROVENANCE.md for the copy steps."; \
	exit 1

require-sdk:
	@if [ ! -f "$(SDK_MARKER)" ]; then $(MAKE) --no-print-directory deps; exit 1; fi

# --- CI Gates ---
ci: ci-host ci-game
	@echo ""
	@echo "=========================================================="
	@echo "  ALL ARDURACER CI CHECKS PASSED SUCCESSFULLY!"
	@echo "=========================================================="

ci-host: fmt-check clippy test playtest
	@echo "--- CI Host Verification Passed ---"

ci-game: exe
	@echo "--- CI PSX Build & RAM Budget Gate ---"
	@set -euo pipefail; \
	exe="$(GAME_EXE)"; map="$(GAME_MAP)"; ld="$(LINKER_SCRIPT)"; \
	if [ ! -f "$$exe" ]; then echo "ERROR: $$exe missing"; exit 1; fi; \
	if [ ! -f "$$map" ]; then echo "ERROR: link map $$map missing (make exe emits it)"; exit 1; fi; \
	if [ ! -f "$$ld" ]; then echo "ERROR: linker script $$ld missing"; exit 1; fi; \
	ld_num() { \
	  local raw; \
	  raw=$$(sed -n "s/^[[:space:]]*$$1[[:space:]]*=[[:space:]]*\([^;]*\);.*/\1/p" "$$ld" | head -1 | tr -d '[:space:]'); \
	  case "$$raw" in \
	    '') echo "ERROR: $$1 is not defined in $$ld" >&2; exit 1;; \
	    *K|*k) echo $$(( $${raw%[Kk]} * 1024 ));; \
	    *M|*m) echo $$(( $${raw%[Mm]} * 1048576 ));; \
	    *G|*g) echo $$(( $${raw%[Gg]} * 1073741824 ));; \
	    *) echo $$(( $$raw ));; \
	  esac; \
	}; \
	map_addr() { \
	  local v; \
	  v=$$(awk -v s="$$1" '$$0 ~ s" = \\.$$" { print $$1; exit }' "$$map"); \
	  if [ -z "$$v" ]; then echo "ERROR: $$1 missing from $$map" >&2; exit 1; fi; \
	  echo $$(( 0x$$v )); \
	}; \
	ram_size=$$(ld_num RAM_SIZE); \
	bios=$$(ld_num BIOS_SIZE); \
	stack=$$(ld_num STACK_RESERVE); \
	header=$$(ld_num HEADER_SIZE); \
	budget=$$(( ram_size - bios - stack )); \
	ceiling=$$(( budget * $(RAM_GATE_MAX_PCT) / 100 )); \
	t_start=$$(map_addr __text_start); \
	d_start=$$(map_addr __data_start); \
	d_end=$$(map_addr __data_end); \
	b_start=$$(map_addr __bss_start); \
	b_end=$$(map_addr __bss_end); \
	text=$$(( d_start - t_start )); \
	data=$$(( d_end - d_start )); \
	bss=$$(( b_end - b_start )); \
	static=$$(( b_end - t_start )); \
	file=$$(stat -c%s "$$exe"); \
	payload=$$(od -An -tu4 -j 28 -N 4 "$$exe" | tr -d '[:space:]'); \
	echo "PSX static RAM (from $(notdir $(LINKER_SCRIPT))):"; \
	echo "  text  $$text"; \
	echo "  data  $$data"; \
	echo "  bss   $$bss  <- NOLOAD, absent from the .exe, largest single term"; \
	echo "  ----"; \
	echo "  total $$static bytes = $$(( static * 100 / budget ))% of the $$budget-byte RAM region"; \
	echo "     (RAM_SIZE $$ram_size - BIOS_SIZE $$bios - STACK_RESERVE $$stack)"; \
	echo "     ceiling $(RAM_GATE_MAX_PCT)% = $$ceiling bytes"; \
	if [ "$$(( text + data ))" != "$$payload" ]; then \
	  echo "ERROR: link map is stale -- it reports $$(( text + data )) bytes of text+data"; \
	  echo "       but the PSX-EXE header says $$payload. Re-run 'make exe'."; \
	  exit 1; \
	fi; \
	if [ "$$file" != "$$(( header + text + data ))" ]; then \
	  echo "ERROR: PSX-EXE is $$file bytes, expected $$(( header + text + data )) (header + text + data)"; \
	  exit 1; \
	fi; \
	if [ "$$static" -gt "$$budget" ]; then \
	  echo "ERROR: statics use $$static bytes of the $$budget-byte RAM region."; \
	  echo "       Shrink .bss/.data or raise the budget in $$ld."; \
	  exit 1; \
	fi; \
	if [ "$$static" -gt "$$ceiling" ]; then \
	  echo "ERROR: statics use $$static bytes, over the $$ceiling-byte ceiling"; \
	  echo "       ($(RAM_GATE_MAX_PCT)% of the RAM region). A runaway static buffer"; \
	  echo "       has eaten the headroom reserved for the stack and any heap."; \
	  echo "       Find it: the offending symbols are between __bss_start and"; \
	  echo "       __bss_end in $(GAME_MAP) (biggest size first)."; \
	  exit 1; \
	fi; \
	echo "OK: $$static bytes of statics (text $$text + data $$data + bss $$bss) within $$ceiling."
	@echo "--- CI PSX RAM Budget Passed ---"

# Disc smoke test. `test -f` on two files proves only that mkisopsx ran; a disc
# image with no ISO9660 primary volume descriptor, no SYSTEM.CNF, a CUE pointing
# at some other file, or a silently-degraded intro FMV all still pass that. Check
# the things a PS1 actually needs in order to boot:
#
#   * the CUE resolves to the .bin we just built,
#   * LBA 16 carries the ISO9660 primary volume descriptor ("CD001"). The .bin is
#     a raw 2352-byte-per-sector Mode 2 image, so the 2048 bytes of user data
#     start 24 bytes into each sector and the "CD001" signature sits 25 bytes in;
#     the check scans the first 32 bytes of the sector rather than assuming the
#     padding is exactly 24.
#   * SYSTEM.CNF is present, so the BIOS has something to execute,
#   * the image is long enough to be a disc,
#   * assets/INTRO.STR is big enough to be a real FMV. cook_intro_str.py
#     silently substitutes a procedural placeholder -- video, no audio -- when
#     psxavenc is missing, and that shipped once as a 92 KB-smaller exe.
#
# No `#` comments inside the recipe below: a comment inside a backslash-
# continued shell command swallows the continuation and make echoes the rest.
ci-disc: disc
	@echo "--- CI Disc Mastering Check ---"
	@set -euo pipefail; \
	bin="$(DIST)/arduracer.bin"; cue="$(DIST)/arduracer.cue"; \
	if [ ! -f "$$bin" ]; then echo "ERROR: $$bin missing"; exit 1; fi; \
	if [ ! -f "$$cue" ]; then echo "ERROR: $$cue missing"; exit 1; fi; \
	if [ ! -f "$(ROOT)/assets/INTRO.STR" ]; then echo "ERROR: assets/INTRO.STR missing"; exit 1; fi; \
	cuefile=$$(sed -n 's/^[[:space:]]*FILE[[:space:]]*"\(.*\)".*/\1/p' "$$cue" | head -1); \
	echo "CUE FILE entry: '$$cuefile'"; \
	if [ "$$cuefile" != "arduracer.bin" ]; then \
	  echo "ERROR: $$cue does not point at arduracer.bin (got '$$cuefile')"; exit 1; \
	fi; \
	desc=$$(dd if="$$bin" bs=2352 skip=16 count=1 2>/dev/null | dd bs=1 skip=16 count=16 2>/dev/null | grep -ao 'CD001' | head -1); \
	echo "ISO9660 primary volume descriptor at LBA 16: '$$desc'"; \
	if [ "$$desc" != "CD001" ]; then \
	  echo "ERROR: $$bin has no ISO9660 primary volume descriptor at LBA 16"; exit 1; \
	fi; \
	if ! grep -qa 'SYSTEM.CNF' "$$bin"; then \
	  echo "ERROR: $$bin does not contain SYSTEM.CNF; the BIOS has nothing to boot"; exit 1; \
	fi; \
	sectors=$$(( $$(stat -c%s "$$bin") / 2352 )); \
	if [ "$$sectors" -lt 100 ]; then \
	  echo "ERROR: $$bin is only $$sectors sectors; that cannot hold a bootable disc"; exit 1; fi; \
	str_bytes=$$(stat -c%s "$(ROOT)/assets/INTRO.STR"); \
	echo "Intro STR: $$str_bytes bytes"; \
	if [ "$$str_bytes" -lt 200000 ]; then \
	  echo "ERROR: assets/INTRO.STR is only $$str_bytes bytes."; \
	  echo "       cook_intro_str.py falls back to a silent procedural placeholder"; \
	  echo "       when psxavenc is unavailable. Install it and re-run 'make assets'."; \
	  exit 1; \
	fi; \
	ls -lh "$$bin" "$$cue"
	@echo "--- CI Disc Check Passed ---"

fmt-check: require-sdk
	@echo "Checking formatting across crates..."
	@for m in crates/arduracer-core tools/test_game_logic tools/playtest tools/test_memcard tools/test_ui game; do \
		if [ -f "$$m/Cargo.toml" ]; then \
			echo "Checking: $$m"; \
			cargo fmt --manifest-path "$$m/Cargo.toml" --all -- --check || exit 1; \
		fi \
	done

fmt:
	@echo "Formatting code across crates..."
	@for m in crates/arduracer-core tools/test_game_logic tools/playtest tools/test_memcard tools/test_ui game; do \
		if [ -f "$$m/Cargo.toml" ]; then \
			echo "Formatting: $$m"; \
			cargo fmt --manifest-path "$$m/Cargo.toml" --all; \
		fi \
	done

clippy: clippy-host clippy-game
	@echo "--- Clippy clean across every crate ---"

# Host-testable crates: full `--all-targets` coverage.
.PHONY: clippy-host
clippy-host: require-sdk
	@echo "Running clippy on host-compatible crates..."
	@if [ -f "crates/arduracer-core/Cargo.toml" ]; then \
		cargo clippy --manifest-path crates/arduracer-core/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi
	@if [ -f "tools/test_game_logic/Cargo.toml" ]; then \
		cargo clippy --manifest-path tools/test_game_logic/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi
	@if [ -f "tools/playtest/Cargo.toml" ]; then \
		cargo clippy --manifest-path tools/playtest/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi
	@if [ -f "tools/test_memcard/Cargo.toml" ]; then \
		cargo clippy --manifest-path tools/test_memcard/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi
	@if [ -f "tools/test_ui/Cargo.toml" ]; then \
		cargo clippy --manifest-path tools/test_ui/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi

# `cargo test` is impossible for `game`: `game/.cargo/config.toml` sets
# `build-std = ["core"]` for the mipsel-sony-psx target, so there is no `test`
# crate to link and `--all-targets` dies with `error[E0463]: can't find crate for
# 'test'` before clippy ever runs. Its bin target is therefore the only thing
# clippy can be pointed at, and `--all-targets` must stay off it. That is also
# why `game/src` had no lint coverage at all until now: nothing in CI ran clippy
# on it. `--release` matches what actually ships (and is the only profile with
# `panic = "abort"`), so the linted code is the linked code.
#
# The `cd` is load-bearing, not stylistic. Cargo discovers `.cargo/config.toml`
# from the *current directory*, not from `--manifest-path`, so running this from
# the repo root with `--manifest-path game/Cargo.toml` silently ignores
# `target = "mipsel-sony-psx"` and tries to build bare-metal Rust for the host:
# `error: #[panic_handler] function required, but not found` plus a pile of
# unresolved SDK imports. Nothing about that failure says "wrong target".
#
# The `-D warnings` here is not aspirational: it fails the build today on the
# warnings listed in the audit, and that is the point -- they are the reason
# `game/src` was unchecked.
.PHONY: clippy-game
clippy-game: require-sdk
	@echo "Running clippy on the bare-metal game crate (bin target only)..."
	@if [ -f "game/Cargo.toml" ]; then \
		( cd $(GAME_DIR) && cargo clippy --release -- -D warnings ) || exit 1; \
	fi

test: require-sdk
	@echo "Running host-side tests in crates/arduracer-core..."
	@if [ -f "crates/arduracer-core/Cargo.toml" ]; then \
		cargo test --manifest-path crates/arduracer-core/Cargo.toml || exit 1; \
	fi
	@echo "Running test_game_logic harness..."
	@if [ -f "tools/test_game_logic/Cargo.toml" ]; then \
		cargo run --manifest-path tools/test_game_logic/Cargo.toml || exit 1; \
	fi
	@echo "Running memory card persistence suite..."
	@if [ -f "tools/test_memcard/Cargo.toml" ]; then \
		cargo test --manifest-path tools/test_memcard/Cargo.toml || exit 1; \
	fi
	@echo "Running UI state-machine suite..."
	@if [ -f "tools/test_ui/Cargo.toml" ]; then \
		cargo test --manifest-path tools/test_ui/Cargo.toml || exit 1; \
	fi
	@echo "Running playtest invariant suite..."
	@echo "(AI obstacle avoidance, championship progression, interior-wall exit)"
	@if [ -f "tools/playtest/Cargo.toml" ]; then \
		cargo test --manifest-path tools/playtest/Cargo.toml || exit 1; \
	fi

# Simulates real laps on all 24 circuits with the real core physics.
# This is the gate that would have caught the unplayable-lap regressions.
#
# It is also the gate on the *generated* par-time table: a measured lap that no
# longer matches the `dev`/`gold`/`silver`/`bronze` ticks compiled into
# `crates/arduracer-core/src/levels.rs` is a failure, because every medal
# boundary in the game just moved. Escape hatches, in order of preference:
#
#   make calibrate-tracks                    re-measure and regenerate (correct fix)
#   make playtest PLAYTEST_FLAGS=--allow-par-drift
#                                           report the drift without failing, for
#                                           the window between landing a physics
#                                           change and re-committing the table
#
# `--calibrate` never fails on drift: it is the command that fixes it.
playtest: require-sdk
	@echo "Verifying every circuit is drivable and the par table is in sync..."
	@if [ -f "tools/playtest/Cargo.toml" ]; then \
		cargo run --manifest-path tools/playtest/Cargo.toml --release -- $(PLAYTEST_FLAGS) || exit 1; \
	fi

# Regenerates crates/arduracer-core/src/levels.rs. Fails if any circuit has an
# unreachable gate, an off-road gate, or a start box outside the racing surface.
tracks:
	@$(MAKE) --no-print-directory require-fx-levels
	@python3 tools/track_cook/convert_levels.py

# Measures real reference laps and rewrites the par-time table before
# regenerating the level data.
calibrate-tracks:
	@cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate
	@$(MAKE) --no-print-directory require-fx-levels
	@python3 tools/track_cook/convert_levels.py

# `tools/track_cook/convert_levels.py` compiles the 20 legacy ArduRacer FX level
# CSVs, which live in `ArduRacerFx/` -- a *reference* directory that .gitignore
# excludes, so it is absent from a clean checkout. Without this the script dies
# with a bare `FileNotFoundError` traceback after the calibration has already
# rewritten par_calibration.json, which reads as "calibration failed" rather than
# "you are missing the reference tree".
FX_LEVEL_CSV := ArduRacerFx/Levels/Level1.csv
.PHONY: require-fx-levels
require-fx-levels:
	@if [ -f "$(FX_LEVEL_CSV)" ]; then exit 0; fi; \
	echo "ERROR: $(FX_LEVEL_CSV) is missing."; \
	echo ""; \
	echo "  tools/track_cook/convert_levels.py compiles the 20 legacy ArduRacer"; \
	echo "  FX level CSVs from ArduRacerFx/, a reference tree that .gitignore"; \
	echo "  excludes, so it is not in a fresh clone. Regenerating"; \
	echo "  crates/arduracer-core/src/levels.rs needs it."; \
	echo ""; \
	echo "  Restore ArduRacerFx/ next to this checkout (or from your own copy of"; \
	echo "  the FX source) and re-run. Par calibration itself does not need it:"; \
	echo "  'cargo run --manifest-path tools/playtest/Cargo.toml --release -- --calibrate'"; \
	echo "  only writes tools/track_cook/par_calibration.json."; \
	exit 1

# Emits the rust-lld link map alongside the PSX-EXE. The `.exe` is a raw
# PS-X EXE image (a 2 KiB header then text+data), not an ELF, and `.bss` is
# NOLOAD, so the file on disk cannot show the static footprint that actually
# occupies PSX RAM. The map is where `__text_start` / `__data_start` /
# `__bss_start` / `__bss_end` live, and `ci-game` reads them from there.
exe: require-sdk
	@mkdir -p $(DIST)
	cd $(GAME_DIR) && cargo rustc --release -- -C link-arg=-Map=$(GAME_MAP)
	@cp $(GAME_EXE) $(DIST)/arduracer.exe
	@test -f $(GAME_MAP) || { echo "ERROR: link map $(GAME_MAP) was not produced"; exit 1; }
	@echo "BUILT PSX-EXE -> $(DIST)/arduracer.exe"
	@echo "LINK MAP     -> $(GAME_MAP)"

iso: exe
	@mkdir -p $(DIST)
	cargo run --release --manifest-path $(MKISOPSX)/Cargo.toml -- \
		--exe $(DIST)/arduracer.exe \
		--out $(DIST)/arduracer.iso \
		--volume ARDURACER \
		--iso
	@echo "SUCCESS! Mastered ISO: $(DIST)/arduracer.iso"

assets:
	@mkdir -p $(ROOT)/assets/cdda
	@python3 $(ROOT)/tools/fmv_cook/cook_intro_str.py $(ROOT)/assets/INTRO.STR
	@python3 $(ROOT)/tools/audio_cook/wav2vag.py --cook-cdda $(ROOT)/assets/cdda $(ROOT)/AssetSource

disc: assets exe
	@mkdir -p $(DIST)
	cargo run --release --manifest-path $(MKISOPSX)/Cargo.toml -- \
		--exe $(DIST)/arduracer.exe \
		--out $(DIST)/arduracer.bin \
		--volume ARDURACER \
		--file $(ROOT)/assets/INTRO.STR \
		--cdda-track $(ROOT)/assets/cdda/track02_title.raw \
		--cdda-track $(ROOT)/assets/cdda/track03_circuit.raw \
		--cdda-track $(ROOT)/assets/cdda/track04_coastal.raw \
		--cdda-track $(ROOT)/assets/cdda/track05_cyber.raw \
		--cdda-track $(ROOT)/assets/cdda/track06_canyon.raw \
		--cdda-track $(ROOT)/assets/cdda/track07_victory.raw
	@echo "SUCCESS! Bootable PS1 Disc Mastered:"
	@echo "  CUE: $(DIST)/arduracer.cue"
	@echo "  BIN: $(DIST)/arduracer.bin"

web: disc iso
	@mkdir -p $(ROOT)/web/roms
	@for f in arduracer.exe arduracer.cue arduracer.bin arduracer.iso; do \
		if [ ! -f "$(DIST)/$$f" ]; then echo "ERROR: $(DIST)/$$f missing"; exit 1; fi; \
		cp "$(DIST)/$$f" "$(ROOT)/web/roms/"; \
	done
	@python3 $(ROOT)/tools/serve_web.py 8080 $(ROOT)/web

# Fails before the (expensive) disc/iso prerequisites rather than after.
.PHONY: require-version
require-version:
	@if [ -z "$(VERSION)" ]; then \
	  echo "ERROR: VERSION is empty."; \
	  echo "       release.yml passes the tag with the leading 'v' stripped; omit the"; \
	  echo "       variable entirely to derive it from the most recent tag."; \
	  exit 1; \
	fi

release: require-version disc iso
	@mkdir -p $(DIST)
	@echo "Packaging Arduracer PSX release distribution..."
	@rm -rf /tmp/arduracer_release && mkdir -p /tmp/arduracer_release/ArduracerPSX
	@for f in arduracer.bin arduracer.cue arduracer.iso arduracer.exe; do \
		if [ ! -f "$(DIST)/$$f" ]; then echo "ERROR: $(DIST)/$$f missing"; exit 1; fi; \
		cp "$(DIST)/$$f" /tmp/arduracer_release/ArduracerPSX/; \
	done
	@# There is no README.md in this repository (GAME.md is the player-facing
	@# manual). The old `cp README.md GAME.md ... || true` copied GAME.md alone and
	@# hid the missing file behind `|| true`, so the omission was invisible.
	@cp GAME.md /tmp/arduracer_release/ArduracerPSX/
	@mkdir -p /tmp/arduracer_release/ArduracerPSX/artwork
	@if ls web/assets/*.svg web/assets/*.png >/dev/null 2>&1; then \
		cp web/assets/*.svg web/assets/*.png /tmp/arduracer_release/ArduracerPSX/artwork/; \
	else \
		echo "WARNING: no artwork found under web/assets/; shipping without it"; \
	fi
	@cd /tmp/arduracer_release && zip -r $(DIST)/ArduracerPSX-v$(VERSION)-PSX.zip ArduracerPSX
	@cd $(DIST) && sha256sum ArduracerPSX-v$(VERSION)-PSX.zip arduracer.bin arduracer.cue arduracer.iso arduracer.exe > SHA256SUMS
	@# release.yml uploads `dist/ArduracerPSX-v<version>-PSX.zip` by that exact name,
	@# where <version> is the tag with the leading `v` stripped. If the archive is
	@# named anything else the release ships with no game in it and no error.
	@if [ ! -f "$(DIST)/ArduracerPSX-v$(VERSION)-PSX.zip" ]; then \
		echo "ERROR: expected $(DIST)/ArduracerPSX-v$(VERSION)-PSX.zip"; exit 1; \
	fi
	@grep -q 'ArduracerPSX-v$(VERSION)-PSX.zip' $(DIST)/SHA256SUMS || { \
		echo "ERROR: SHA256SUMS does not cover the versioned archive"; exit 1; }
	@echo "SUCCESS! Packaged release:"
	@ls -lh $(DIST)/ArduracerPSX-v$(VERSION)-PSX.zip $(DIST)/SHA256SUMS

run: disc
	@cue="$(DIST)/arduracer.cue"; \
	if [ -n "$(EMULATOR)" ]; then \
		echo "Launching $$cue with EMULATOR=$(EMULATOR)..."; \
		"$(EMULATOR)" "$$cue"; \
		exit $$?; \
	fi; \
	for a in $(DUCKSTATION_APPIMAGES); do \
		if [ -f "$$a" ]; then \
			echo "Launching $$cue in DuckStation ($$a)..."; \
			APPIMAGE_EXTRACT_AND_RUN=1 "$$a" "$$cue"; \
			exit $$?; \
		fi \
	done; \
	if command -v duckstation-qt >/dev/null 2>&1; then \
		duckstation-qt "$$cue"; exit $$?; \
	fi; \
	if command -v duckstation >/dev/null 2>&1; then \
		duckstation "$$cue"; exit $$?; \
	fi; \
	echo "No PS1 emulator automatically detected. Please run: EMULATOR=/path/to/emulator make run"

clean:
	@if [ -d "$(GAME_DIR)" ]; then cd $(GAME_DIR) && cargo clean; fi
	@rm -rf $(DIST) $(ROOT)/assets
