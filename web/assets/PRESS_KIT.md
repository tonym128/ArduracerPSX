# ARDURACER: HIGH-OCTANE ARCADE MOTORSPORT
## Official Press Kit & Retail Fact Sheet
### Bare-Metal Rust on 1994 Hardware (Sony PlayStation 1 / PSX)

---

## 📌 Fact Sheet

* **Title**: Arduracer: High-Octane Arcade Motorsport
* **Developer**: Arduracer Development Team
* **Platform**: Sony PlayStation 1 (PS1 / PSX)
* **Release Format**: CD-ROM (CUE/BIN Image), Cooked ISO, Standalone PSX-EXE, Physical Retail Jewel Case Edition
* **Target Audience**: Retro racing enthusiasts, PS1 collectors, arcade fans (Ridge Racer / Micro Machines / Super Sprint), Rust embedded developers
* **Language Support**: English, Français, Deutsch, Italiano, Español
* **Hardware Compatibility**: All PS1 consoles (NTSC-U/C, PAL, NTSC-J) via Modchip, UniROM, XStation, PSIO; Emulators (DuckStation, RetroArch, Mednafen, PSoXide)
* **Peripherals Supported**: Standard Digital Controller, DualShock® Analog Controller (360° analog steering + dual-motor vibration feedback), PlayStation Memory Card (1 Block)
* **Serial Number**: `SLUS-00999` (NTSC Master ID) / `SLES-00999` (PAL Catalog Reference)
* **License**: GPL-2.0-or-later

---

## 🎯 Elevator Pitch

> **Ridge Racer adrenaline meets classic top-down precision motorsport — built from scratch in bare-metal Rust for 1994 Sony PlayStation hardware.**

Tear through 24 diverse world circuits across 4 Championship Cups. Featuring realistic slip-angle drift physics, kinetic barrier deflections, 6 customizable performance parameters, an aggressive AI rival pack, Red Book CD-DA Eurobeat soundtrack, and full-motion MDEC intro cinema running at a rock-solid 60 FPS.

---

## 🌟 Key Selling Points & Features

### 1. Genuine Bare-Metal Rust Engineering
* Compiled directly to **MIPS R3000A** bare-metal machine code using `rustc` and the open-source **PSoXide** SDK.
* Zero runtime overhead, `#![no_std]` architecture, utilizing only 311 KB of the PS1's 2 MB main RAM budget (~14%).
* Core simulation logic cleanly decoupled into `crates/arduracer-core` and verified with 40 host-side integration test suites.

### 2. High-Speed 60 FPS Top-Down Renderer
* Smooth 60 frames-per-second arcade rendering on real hardware at 320x240 NTSC (or PAL 50/60Hz).
* Dynamic camera tracking with velocity-lead framing and look-ahead anticipation.
* Surface-aware tilemap blitting with asphalt, red/white rumble curbs, gravel verges, emerald off-road grass, iridescent oil slicks, neon boost chevrons, and Armco safety barriers.

### 3. Deep Arcade Drift & Traction Physics
* Real slip-angle vehicle physics separating directional heading from velocity momentum.
* Progressive counter-steering stabilization: hold reverse lock to catch high-speed slides and rocket out of hairpins.
* Nitro turbo meter with dynamic discharge and passive on-track regeneration.

### 4. 24 Diverse Championship Circuits
* **Cup 1 — Novice Sprint**: Arduboy Oval, Twin Hairpin, The Serpent, Canyon Chicane, Switchback Pass, Grand Ring.
* **Cup 2 — Clubman League**: Sprint Short, Octagon Speedway, Devil's Elbow, Metropolis 10, Forest Expressway, Coastal Link.
* **Cup 3 — Pro Grand Prix**: Alpine Drift, Industrial Yard, Nightway Circuit, Harbor Slalom, Mountain Gauntlet, Super Speedway.
* **Cup 4 — World Championship**: Endurance Colosseum, Championship Final, Neo Tokyo Expressway, Canyon Drift Apex, Cyber Circuit 2097, Monaco GP Classic.

### 5. 20-Point Garage Vehicle Tuning
* Allocate 20 points across **Top Speed**, **Acceleration**, **Grip**, and **Boost** to customize your racing style.
* Save best lap times, gold cup trophies, unlocked tracks, and vehicle tunings to a single authentic PS1 Memory Card block (CRC-16 verified).

### 6. Red Book CD-DA Audio & 15 FPS MDEC Cinema
* 6 adrenaline-pumping Eurobeat and synthwave soundtrack tracks streaming directly via Red Book CD-DA.
* Full-motion 15 FPS MDEC video intro cinematic (`INTRO.STR`) letterboxed to crisp 4:3 with SPU ADPCM sound effects.
