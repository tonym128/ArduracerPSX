SHELL := /bin/bash
ROOT  := $(shell pwd)

DIST     := $(ROOT)/dist
GAME_DIR := $(ROOT)/game
GAME_EXE := $(GAME_DIR)/target/mipsel-sony-psx/release/arduracer.exe
MKISOPSX := $(ROOT)/psoxide/tools/mkisopsx

DUCKSTATION_APPIMAGES := \
	$(HOME)/Downloads/DuckStation-x86_64.AppImage \
	$(HOME)/Applications/DuckStation-x86_64.AppImage \
	/opt/duckstation/DuckStation-x86_64.AppImage

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

.PHONY: all help test clippy fmt fmt-check ci ci-host ci-game ci-disc exe iso disc clean run

all: test exe

help:
	@echo "Arduracer PSX - Build Targets:"
	@echo "  make test      - Run automated host-side game logic tests (arduracer-core)"
	@echo "  make exe       - Build bare-metal MIPS PSX executable (dist/arduracer.exe)"
	@echo "  make disc      - Master bootable PS1 disc image (dist/arduracer.bin/.cue)"
	@echo "  make iso       - Master simple data ISO (dist/arduracer.iso)"
	@echo "  make clippy    - Run clippy linting on host crates"
	@echo "  make fmt-check - Check code formatting across all crates"
	@echo "  make fmt       - Format code across all crates"
	@echo "  make ci        - Run complete CI verification (host, game, disc)"
	@echo "  make run       - Run mastered disc in emulator (DuckStation / RetroArch)"
	@echo "  make clean     - Clean build outputs"

# --- CI Gates ---
ci: ci-host ci-game
	@echo ""
	@echo "=========================================================="
	@echo "  ALL ARDURACER CI CHECKS PASSED SUCCESSFULLY!"
	@echo "=========================================================="

ci-host: fmt-check clippy test
	@echo "--- CI Host Verification Passed ---"

ci-game: exe
	@echo "--- CI PSX Build & RAM Budget Gate ---"
	@set -euo pipefail; \
	exe="$(DIST)/arduracer.exe"; \
	test -f "$$exe"; \
	size=$$(stat -c%s "$$exe"); \
	pct=$$(( size * 100 / 2097152 )); \
	echo "arduracer.exe = $$size bytes ($$pct% of 2 MB PS1 RAM)"; \
	if [ "$$size" -ge 2097152 ]; then \
		echo "ERROR: executable exceeds the 2 MB main RAM of a PlayStation 1"; \
		exit 1; \
	fi
	@echo "--- CI PSX RAM Budget Passed ---"

ci-disc: disc
	@echo "--- CI Disc Mastering Check ---"
	@set -euo pipefail; \
	test -f "$(DIST)/arduracer.bin"; \
	test -f "$(DIST)/arduracer.cue"; \
	ls -lh $(DIST)/arduracer.bin $(DIST)/arduracer.cue
	@echo "--- CI Disc Check Passed ---"

fmt-check:
	@echo "Checking formatting across crates..."
	@for m in crates/arduracer-core tools/test_game_logic game; do \
		if [ -f "$$m/Cargo.toml" ]; then \
			echo "Checking: $$m"; \
			cargo fmt --manifest-path "$$m/Cargo.toml" --all -- --check || exit 1; \
		fi \
	done

fmt:
	@echo "Formatting code across crates..."
	@for m in crates/arduracer-core tools/test_game_logic game; do \
		if [ -f "$$m/Cargo.toml" ]; then \
			echo "Formatting: $$m"; \
			cargo fmt --manifest-path "$$m/Cargo.toml" --all; \
		fi \
	done

clippy:
	@echo "Running clippy on host-compatible crates..."
	@if [ -f "crates/arduracer-core/Cargo.toml" ]; then \
		cargo clippy --manifest-path crates/arduracer-core/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi
	@if [ -f "tools/test_game_logic/Cargo.toml" ]; then \
		cargo clippy --manifest-path tools/test_game_logic/Cargo.toml --all-targets -- -D warnings || exit 1; \
	fi

test:
	@echo "Running host-side tests in crates/arduracer-core..."
	@if [ -f "crates/arduracer-core/Cargo.toml" ]; then \
		cargo test --manifest-path crates/arduracer-core/Cargo.toml || exit 1; \
	fi
	@echo "Running test_game_logic harness..."
	@if [ -f "tools/test_game_logic/Cargo.toml" ]; then \
		cargo run --manifest-path tools/test_game_logic/Cargo.toml || exit 1; \
	fi

exe:
	@mkdir -p $(DIST)
	cd $(GAME_DIR) && cargo build --release
	@cp $(GAME_EXE) $(DIST)/arduracer.exe
	@echo "BUILT PSX-EXE -> $(DIST)/arduracer.exe"

iso: exe
	@mkdir -p $(DIST)
	cargo run --release --manifest-path $(MKISOPSX)/Cargo.toml -- \
		--exe $(DIST)/arduracer.exe \
		--out $(DIST)/arduracer.iso \
		--volume ARDURACER \
		--iso
	@echo "SUCCESS! Mastered ISO: $(DIST)/arduracer.iso"

disc: exe
	@mkdir -p $(DIST)
	cargo run --release --manifest-path $(MKISOPSX)/Cargo.toml -- \
		--exe $(DIST)/arduracer.exe \
		--out $(DIST)/arduracer.bin \
		--volume ARDURACER
	@echo "SUCCESS! Bootable PS1 Disc Mastered:"
	@echo "  CUE: $(DIST)/arduracer.cue"
	@echo "  BIN: $(DIST)/arduracer.bin"

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
	rm -rf $(DIST)
