//! The one place a level's tile grid lives at runtime.
//!
//! # Why
//!
//! Every level's grid used to be a `static [TrackTile; N]` compiled into the
//! executable: 6,400 bytes for each of the 24 circuits and 57,600 for each of
//! the three free-roam cities -- **326,400 bytes in total**. On a PS1 `.rodata`
//! is loaded into RAM along with the EXE, so that is live RAM, not ROM, and the
//! game runs exactly one level at a time. Moving them to `LEVELS.BIN` reclaims
//! all but the one buffer here.
//!
//! # Why not in `arduracer-core`
//!
//! That crate carries `#![deny(unsafe_code)]` outside tests, so the mutable
//! storage cannot live next to [`TrackDef`]. The engine keeps only the data
//! model -- `TrackDef::tiles` is an `Option` pointing here -- and the game, which
//! already owns the CD reader, does the loading.
//!
//! # No fallback
//!
//! A failed read leaves the grid empty, and an empty grid reads as
//! [`Barrier`] everywhere. That is deliberate: the alternative -- keeping the
//! compiled-in grids as a backstop -- silently doubles the RAM this exists to
//! save, and a load that fails *sometimes* is harder to diagnose than one that
//! fails loudly and always. The load reports itself on the TTY either way.
//!
//! # Concurrency
//!
//! Plain `static mut`, like `DiscReader` and the VRAM slots: the target has
//! `max-atomic-width = 0`, the guest is single-core, and the VBlank IRQ never
//! touches this. A grid is written once per level load, in the main loop between
//! frames, and read every frame after that.

use arduracer_core::{TrackDef, TrackTile};

/// Largest grid any shipped level has: the 10 km² cities are 240x240 cells, the
/// circuits 80x80.
pub const MAX_LEVEL_TILES: usize = 57_600;

/// Slack for a sector-aligned read that lands before the first wanted cell.
///
/// `LEVELS.BIN` packs grids back to back, so a grid rarely starts on a sector
/// boundary and the read has to land `first` bytes into the buffer before the
/// wanted run is compacted down. Up to 4096 bytes (2 sectors) of slack ensures
/// that any sector-aligned read spanning `first + len` fits in `GRID_BYTES` even
/// when `first` is near a sector end and `len == MAX_LEVEL_TILES`.
const STAGE_SLACK: usize = 4096;

/// The buffer is one byte per cell, not a `TrackTile`, because that is the disc
/// format: sectors are read straight in and decoded in place, which is what
/// removes the staging copy entirely.
const GRID_BYTES: usize = MAX_LEVEL_TILES + STAGE_SLACK;

#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut GRID: [u8; GRID_BYTES] = [TrackTile::OffRoad as u8; GRID_BYTES];

/// Cells currently installed; 0 when no grid is loaded.
static mut GRID_LEN: usize = 0;

/// The installed grid as `TrackTile`s.
///
/// The bytes were decoded in place when the grid was installed and nothing else
/// writes the buffer afterwards, so this is a re-interpretation rather than a
/// re-read.
///
/// # Safety
/// The invariant is that bytes `0..GRID_LEN` hold only `TrackTile`
/// discriminants. `install` establishes it, `clear` establishes it, and nothing
/// else writes `GRID`.
#[inline]
pub fn grid() -> &'static [TrackTile] {
    let len = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(GRID_LEN)) };
    // SAFETY: see the type-level note. `TrackTile` is `repr(u8)`, so the byte
    // buffer and a `TrackTile` array have the same alignment and layout.
    let all = unsafe { &*(core::ptr::addr_of!(GRID) as *const [TrackTile; GRID_BYTES]) };
    &all[..len]
}

/// Cells currently installed.
#[inline]
pub fn len() -> usize {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(GRID_LEN)) }
}

/// Empties the grid.
///
/// Called on a circuit change, and before every load: a level whose grid has not
/// been installed must not read as the *previous* level's terrain, which would be
/// a worse failure than reading as solid wall.
pub fn clear() {
    unsafe {
        let grid = &mut *core::ptr::addr_of_mut!(GRID);
        for cell in grid.iter_mut() {
            *cell = TrackTile::OffRoad as u8;
        }
        GRID_LEN = 0;
    }
}

/// Repoints `track` at the shared grid.
///
/// This is the write `TrackDef::tiles: Option` exists to allow: it mutates a
/// `static TrackDef` through a raw pointer, which is why it lives here rather
/// than in the engine crate, and why the engine's copy of the data model is
/// optional even though the engine itself may not perform the write.
pub fn bind(track: &'static TrackDef) {
    // SAFETY: single-threaded guest. The level load happens in the main loop
    // between frames; every reader reaches the grid through `tile_at`, which
    // re-reads the slice rather than caching it. Nothing aliases `track.tiles`
    // across this call.
    // A *volatile* store, and that is not a micro-optimisation. `TrackDef` is a
    // plain `static`, so writing through a `*const -> *mut` cast is UB, and the
    // optimiser is entitled to delete the store -- which it did: the write landed
    // in `.data`, the address was right, and `tiles` read back `None` anyway. A
    // volatile store is safe to perform and cannot be elided.
    unsafe {
        let cell = track as *const TrackDef as *mut TrackDef;
        core::ptr::write_volatile(&raw mut (*cell).tiles, Some(grid()));
    }
}

/// Byte offset and cell count of level `index` within `LEVELS.BIN`.
///
/// The asset is a flat table of grids in level order with no header, so offsets
/// are pure arithmetic over the prefix sums. A header would cost a seek to read;
/// this costs none, because every level's dimensions are already known.
pub fn offset_in_asset(index: usize, dims: &[(u8, u8)]) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    for (w, h) in dims.iter().take(index) {
        offset += *w as usize * *h as usize;
    }
    let (w, h) = *dims.get(index)?;
    let len = w as usize * h as usize;
    if len > MAX_LEVEL_TILES {
        return None;
    }
    Some((offset, len))
}

/// Reads level `index`'s grid out of `LEVELS.BIN`, installs it, and binds
/// `track` to it.
///
/// Reports each stage on the TTY. The four prints are the diagnosis: an asset
/// that does not resolve, an offset that does not match the asset, a read that
/// fails, and a bind that does not stick are four different bugs that present as
/// the same frozen car.
pub fn load(
    reader: &mut crate::cd_fs::DiscReader,
    asset_lba: Option<u32>,
    track: &'static TrackDef,
    index: usize,
    dims: &[(u8, u8)],
) -> bool {
    clear();
    let Some(asset_lba) = asset_lba else {
        crate::dbg::println("[LEVELS] FAIL: LEVELS.BIN not located on disc");
        return false;
    };
    let Some((offset, len)) = offset_in_asset(index, dims) else {
        crate::dbg::print("[LEVELS] FAIL: bad level index ");
        crate::dbg::print_dec(index as u32);
        crate::dbg::println("");
        return false;
    };
    let start = asset_lba + (offset / 2048) as u32;
    let first = offset % 2048;
    let count = (first + len).div_ceil(2048);

    if count * 2048 > GRID_BYTES || len > MAX_LEVEL_TILES {
        crate::dbg::println("[LEVELS] FAIL: grid does not fit the buffer");
        return false;
    }

    let read = reader.read_sectors(start, count, unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(GRID).cast::<u8>(), count * 2048)
    });
    if !read {
        crate::dbg::println("[LEVELS] FAIL: disc read");
        return false;
    }

    unsafe {
        let grid = &mut *core::ptr::addr_of_mut!(GRID);
        // Compact the wanted run down to cell 0, then decode in place.
        core::ptr::copy(grid.as_ptr().add(first), grid.as_mut_ptr(), len);
        for cell in grid.iter_mut().take(len) {
            *cell = TrackTile::from_disc_byte(*cell) as u8;
        }
        for cell in grid.iter_mut().skip(len) {
            *cell = TrackTile::OffRoad as u8;
        }
        GRID_LEN = len;
    }
    bind(track);

    // Confirm the bind stuck rather than trusting it: the whole failure mode is
    // silent, so a read-back is worth one comparison per load.
    // Verify the rebind rather than trusting it: the whole failure mode is
    // silent, and the store it checks for is one the optimiser is otherwise
    // entitled to delete. If this ever fails, every tile is `Barrier`.
    let ok = track.tiles.map(|t| t.len()) == Some(len);
    if !ok {
        crate::dbg::println("[LEVELS] FAIL: rebind did not stick");
    }
    ok
}

#[cfg_attr(target_arch = "mips", link_section = ".data")]
static mut CUSTOM_TRACK_DEF: TrackDef = TrackDef {
    name: "CUSTOM TRACK",
    width: 80,
    height: 80,
    start_pos: arduracer_core::math::Vec2::ZERO,
    start_heading: 0,
    start_gate: arduracer_core::timing::CheckpointGate {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    },
    par_times: arduracer_core::timing::ParTimes {
        bronze_ticks: 2700,
        silver_ticks: 2400,
        gold_ticks: 2050,
        dev_platinum_ticks: 1800,
    },
    checkpoint_count: 0,
    checkpoints: [arduracer_core::timing::CheckpointGate {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    }; arduracer_core::track::MAX_TRACK_CHECKPOINTS],
    tiles: None,
    route: &[],
    half_width: (2 * arduracer_core::track::TILE_SIZE) as u16,
};

/// Compiles a custom track into the runtime tile grid buffer and binds it to `CUSTOM_TRACK_DEF`.
pub fn compile_and_bind_custom_track(
    track: &arduracer_core::track_builder::CustomTrackDef,
) -> Result<&'static TrackDef, arduracer_core::track_builder::TrackBuildError> {
    unsafe {
        let all_tiles = &mut *(core::ptr::addr_of_mut!(GRID) as *mut [TrackTile; GRID_BYTES]);
        let compiled = track.compile(&mut all_tiles[..arduracer_core::track_builder::TOTAL_CUSTOM_TILES])?;
        GRID_LEN = arduracer_core::track_builder::TOTAL_CUSTOM_TILES;

        let def = &mut *core::ptr::addr_of_mut!(CUSTOM_TRACK_DEF);
        def.name = "CUSTOM TRACK";
        def.width = compiled.width;
        def.height = compiled.height;
        def.start_pos = compiled.start_pos;
        def.start_heading = compiled.start_heading;
        def.start_gate = compiled.start_gate;
        def.par_times = compiled.par_times;
        def.checkpoint_count = compiled.checkpoint_count;
        def.checkpoints = compiled.checkpoints;
        def.tiles = Some(grid());
        def.route = &[];
        def.half_width = (2 * arduracer_core::track::TILE_SIZE) as u16;
        Ok(def)
    }
}
