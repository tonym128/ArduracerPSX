//! Non-blocking tile fetch for the uncompressed TIM and TIM4 formats.
//!
//! # Why
//!
//! The blocking form paid a Pause/Setloc/seek and then *waited*, inside the
//! render path. Measured on the PSoXide emulator that is up to 9.7M cycles in a
//! single frame against a 1.13M-cycle frame budget -- 8.6x the whole frame.
//!
//! # The shape of the fix
//!
//! The seek stays. What goes away is the wait: the command is issued, and the
//! sectors are collected by a per-frame poll, so a frame that asks for a tile
//! pays the command's cost and nothing more. A tile that has not arrived yet
//! draws the existing placeholder and is retried next frame.
//!
//! An earlier attempt used a sector ring shared across tiles and had to hold one
//! long ReadN stream. That shape is the wrong one here: it keeps the drive
//! streaming for as long as the stream is open, and anything that stops
//! draining it -- a decode budget cut, a tile the caller stops asking for --
//! overruns the drive's two-sector FIFO and desynchronises the stream. The
//! emulator reported it plainly: `cd-sectors-dropped=44`.
//!
//! Here the stream is at most one tile long (four sectors for TIM, one for
//! TIM4) and is closed the moment the tile lands, so the drive is never asked to
//! hold more than it can deliver. That is the whole difference, and it is why
//! this needs no ring and no shared buffer.
//!
//! # Draining is not optional
//!
//! The drive pushes on its own clock. A sector that arrives and is not taken
//! costs the next one, and two unclaimed sectors overrun the FIFO. So the drain
//! is driven from [`pump`] once per frame rather than from the request that
//! happened to start the fetch: a caller that stops asking for a tile, or asks
//! for a different one, cannot strand sectors in the drive. That is precisely
//! the failure the ring version fell into.
//!
//! # Why one tile per seek, and not a window of them
//!
//! The obvious optimisation is to fetch a *window* of contiguous sectors under
//! one seek: a PS1 seek costs ~150 ms, while sixteen sectors take 213 ms to
//! stream, so a four-tile TIM window looks like it should amortise the seek four
//! ways. It was built and measured, and it was worse.
//!
//! Two reasons, both measured on the emulator rather than argued:
//!
//! - The window rarely got reused. 306 seeks served 339 tiles -- 1.13 tiles per
//!   seek, against the 4 a window should yield. Ordering the decode queue by disc
//!   position instead of screen position, to force sequential requests, moved
//!   neither number. The camera-visible list is rebuilt each frame and the VRAM
//!   pool thrashes, so consecutive requests genuinely are not adjacent.
//! - It cost 32 KB of RAM against 8 KB, and it was the only variant that ever
//!   dropped sectors on the drive: a sixteen-sector stream outlives the two-slot
//!   FIFO if a frame does not drain it, and the emulator reported
//!   `cd-sectors-dropped=44`. The four-sector stream did not.
//!
//! So the window is sized to one tile. The shape is kept -- a fetch covers a
//! range of sectors and a tile is served from its offset in it -- because that
//! is the only part that generalises, and widening `SECTORS_PER_FETCH` is then a
//! one-line change to try again once the queue can be made sequential.
//!
//! The real limit turned out not to be the disc at all: the run reports 40,212
//! cache hits against 42,043 placeholders, so half of what is on screen each
//! frame is not resident in a 64-slot VRAM pool covering ~52 visible tiles. The
//! texture fill rate is bounded by VRAM capacity, and widening the read made that
//! worse rather than better.
//!
//! # Where the sectors land
//!
//! A private buffer, not the decoders' `TileScratch`. TileScratch is still live
//! for the incremental decode path, which must not have a half-read tile appear
//! under it.

/// Words one sector is.
const SECTOR_WORDS: usize = 512;

/// Sectors fetched under one seek: exactly one tile's worth.
///
/// See the module notes for the measurements behind this being one tile rather
/// than a wider window. Widening it to 16 was built and was worse: fewer tiles
/// per seek, four times the RAM, and the drive dropped sectors.
const WINDOW_SECTORS: u32 = 4;

/// The window. Sectors land here in arrival order, and a tile is served from
/// the offset its LBA falls at.
static mut WINDOW: [u32; WINDOW_SECTORS as usize * SECTOR_WORDS] =
    [0; WINDOW_SECTORS as usize * SECTOR_WORDS];

/// Frames a fetch may stay open before it is treated as stalled.
///
/// A full window is 213 ms of streaming at 75 sectors/s, so this has to clear
/// that comfortably while still catching a drive that has stopped.
const STALL_FRAMES: u32 = 120;

/// Sectors still ahead of the drive in an open window, for the heartbeat.
static mut WINDOW_REMAINING: u32 = 0;

/// Seeks issued. Tiles served without one are `LANDED - SEEKED`, which is the
/// only measure of whether the window is being reused at all.
static mut SEEKED: u32 = 0;

/// Which uncompressed format asked for a tile.
///
/// Kept for telemetry and for sizing a tile inside the window; the fetch itself
/// is told how many sectors it wants.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Codec {
    Tim,
    Tim4,
    Tim8,
}

/// Sectors a codec's tile spans.
pub const fn sectors_for(codec: Codec) -> u32 {
    match codec {
        Codec::Tim => 4,
        Codec::Tim4 => 1,
        Codec::Tim8 => 2,
    }
}

/// A window being filled, across the frames it takes to arrive.
#[derive(Copy, Clone)]
struct Fetch {
    /// The drive stream is open.
    active: bool,
    /// LBA of window sector 0.
    base_lba: u32,
    /// Sectors collected.
    have: u32,
    /// Sectors the window covers.
    total: u32,
    /// Frame the seek was issued on, for the stall timeout.
    requested_frame: u32,
}

const fn no_fetch() -> Fetch {
    Fetch {
        active: false,
        base_lba: 0,
        have: 0,
        total: 0,
        requested_frame: 0,
    }
}

static mut PENDING: Fetch = no_fetch();

/// Tiles handed back from the window.
static mut LANDED: u32 = 0;

/// Fetches abandoned without the window filling, on a drive fault or on nothing
/// arriving in [`STALL_FRAMES`] frames.
///
/// Not a normal outcome: the stream is short and the frame pump drains it every
/// frame, so a stall means the drive desynchronised rather than that it was
/// slow.
static mut STALLED: u32 = 0;

/// Requests that found their tile still being fetched, so the frame drew the
/// placeholder and will ask again.
///
/// This is the visible cost of the asynchronous read, and the number to watch:
/// it is high for the first few hundred frames as the visible ground is filled,
/// and should fall towards zero once the camera is over ground already covered.
static mut WAITED: u32 = 0;

/// Outcome of an asynchronous tile fetch.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FetchStatus {
    /// Pixels are ready in the window buffer and can be uploaded to VRAM.
    Ready(*const u32),
    /// Tile is currently being streamed from the disc across frames.
    Pending,
    /// Fetch failed to start or stalled/faulted.
    Failed,
}

/// The window buffer, as a pixel base plus its length in words.
fn window() -> *const u32 {
    core::ptr::addr_of!(WINDOW) as *const u32
}

/// Fills the window from whatever the drive has produced, and closes the stream
/// once it is whole.
///
/// Separated from [`fetch`] because the drive's clock does not stop for the
/// caller: see the module note on draining.
fn step(reader: &mut crate::cd_fs::DiscReader, current_frame: u32) {
    let job = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    if !job.active {
        return;
    }
    if current_frame.wrapping_sub(job.requested_frame) > STALL_FRAMES {
        reader.stop_stream();
        *job = no_fetch();
        unsafe { STALLED = STALLED.wrapping_add(1) };
        unsafe { WINDOW_REMAINING = 0 };
        crate::drive::release(crate::drive::Owner::Texture);
        return;
    }

    // Drain until the drive says it has nothing more. Taking only one sector per
    // pass would leave the second one to be overrun.
    let base = window();
    while job.have < job.total {
        let at = job.have as usize * SECTOR_WORDS;
        // Safety: `have < total <= WINDOW_SECTORS`, so the 512 words written
        // start inside the window and end at or before its end. The pointer is
        // rebuilt each pass so no borrow outlives the call that filled it.
        let dst = unsafe { &mut *((base as *mut u32).add(at) as *mut [u32; SECTOR_WORDS]) };
        match reader.try_tile_sector(dst) {
            crate::cd_fs::SectorPoll::Filled => job.have += 1,
            crate::cd_fs::SectorPoll::Empty => break,
            crate::cd_fs::SectorPoll::Fault => {
                reader.stop_stream();
                *job = no_fetch();
                unsafe { STALLED = STALLED.wrapping_add(1) };
                unsafe { WINDOW_REMAINING = 0 };
                crate::drive::release(crate::drive::Owner::Texture);
                return;
            }
        }
    }

    unsafe { WINDOW_REMAINING = job.total - job.have };
    if job.have >= job.total {
        // The window is whole, so the drive has nothing left to give it.
        // Closing here is what keeps the stream bounded; leaving it open would
        // let the drive run on into the next window's sectors.
        reader.stop_stream();
        job.active = false;
        crate::drive::release(crate::drive::Owner::Texture);
    }
}

/// Advances the fetch in flight. Call once per frame.
///
/// Cheap when nothing is open, which is the case for every non-TIM frame.
/// Without it a window the caller has stopped asking about would sit in the
/// drive and overrun it.
pub fn pump(reader: &mut crate::cd_fs::DiscReader, current_frame: u32) {
    step(reader, current_frame);
}

/// Requests one tile, opening a fetch if none is in progress.
///
/// Returns `FetchStatus::Ready` when pixels are in hand, `FetchStatus::Pending`
/// when sectors are being collected or another fetch is in flight, or
/// `FetchStatus::Failed` on permanent error or drive fault.
///
/// Never blocks. The cost on the frame that opens a fetch is the seek command
/// and nothing more; every frame after that is one `try_read_sector` per sector
/// the drive produced.
///
/// `available` is how many sectors may be taken from `tile_lba` before the end
/// of the contiguous run it belongs to. The caller knows that bound -- it is the
/// end of the block -- and passing it keeps a fetch near the end of a block from
/// streaming into whatever the cooker laid down next.
pub fn fetch(
    reader: &mut crate::cd_fs::DiscReader,
    codec: Codec,
    tile_lba: u32,
    available: u32,
    current_frame: u32,
) -> FetchStatus {
    step(reader, current_frame);
    let need = sectors_for(codec);
    let job = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };

    // Already covered? Only if the whole tile has arrived, not merely been
    // asked for -- a half-read tile is not pixels.
    let end = tile_lba + need;
    if job.total >= need && tile_lba >= job.base_lba && end <= job.base_lba + job.have {
        let at = (tile_lba - job.base_lba) as usize * SECTOR_WORDS;
        unsafe { LANDED = LANDED.wrapping_add(1) };
        return FetchStatus::Ready(unsafe { (window() as *mut u32).add(at) });
    }

    // A window the caller did not ask for. Leave it entirely alone.
    //
    // It is tempting to abandon it here, on the reasoning that a fetch with
    // nothing collected yet is cheaper to cancel than to wait for. That is
    // wrong for this caller: the render path offers up to `PREFETCH_MAX`
    // candidates per frame, so every candidate after the first is by definition
    // a request for a tile outside the window. Cancelling on a mismatch
    // therefore means each candidate tore down the stream the previous one had
    // just opened, in the same frame, before the drive delivered anything -- no
    // tile ever finished. Measured at one tile per thirteen frames.
    //
    // The drive cannot be steered by whoever asks loudest. One window in flight
    // is the whole concurrency limit, and a caller wanting a different tile
    // waits its turn.
    if job.active {
        unsafe { WAITED = WAITED.wrapping_add(1) };
        return FetchStatus::Pending;
    }

    if available < need {
        return FetchStatus::Failed;
    }

    if !crate::drive::claim(crate::drive::Owner::Texture) {
        unsafe { WAITED = WAITED.wrapping_add(1) };
        return FetchStatus::Pending;
    }

    // Nothing open and the tile is not covered. A finished window is just cached
    // sectors now -- keyed by LBA, with no owner -- so it is free to overwrite.
    // Stream exactly one tile's sectors and close immediately when they land.
    let total = need.min(available);
    unsafe { SEEKED = SEEKED.wrapping_add(1) };
    if !reader.begin_tile_stream(tile_lba) {
        crate::drive::release(crate::drive::Owner::Texture);
        return FetchStatus::Failed;
    }
    *job = Fetch {
        active: true,
        base_lba: tile_lba,
        have: 0,
        total,
        requested_frame: current_frame,
    };
    // Sectors the drive has already buffered can be taken on this same frame, so
    // a window that opens late does not cost a further frame.
    step(reader, current_frame);
    let job = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    if job.have >= need && tile_lba >= job.base_lba {
        let at = (tile_lba - job.base_lba) as usize * SECTOR_WORDS;
        unsafe { LANDED = LANDED.wrapping_add(1) };
        FetchStatus::Ready(unsafe { (window() as *mut u32).add(at) })
    } else {
        unsafe { WAITED = WAITED.wrapping_add(1) };
        FetchStatus::Pending
    }
}

/// Abandons any fetch in flight, closing the drive stream with it.
///
/// Called on a circuit or format switch: the window belonged to a block that is
/// no longer being read, and leaving its stream open would have the drive still
/// pushing sectors nobody is collecting.
pub fn cancel() {
    crate::gpu::tracktex::disc_reader().stop_stream();
    unsafe { *core::ptr::addr_of_mut!(PENDING) = no_fetch() };
    unsafe { WINDOW_REMAINING = 0 };
    crate::drive::release(crate::drive::Owner::Texture);
}

/// Whether an asynchronous fetch is currently in flight on the CD drive.
pub fn is_in_flight() -> bool {
    unsafe { (*core::ptr::addr_of!(PENDING)).active }
}

/// Tiles handed back for upload.
pub fn landed() -> u32 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(LANDED)) }
}

/// Fetches that gave up waiting for the drive.
pub fn stalled() -> u32 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(STALLED)) }
}

/// Requests that found their tile still being fetched.
pub fn waited() -> u32 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(WAITED)) }
}

/// Seeks issued. Compared against [`landed`] to see whether windows are being
/// reused or every tile is paying its own seek.
pub fn seeks() -> u32 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(SEEKED)) }
}

/// Sectors the open window still expects, for the heartbeat.
pub fn outstanding() -> u32 {
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(WINDOW_REMAINING)) }
}
