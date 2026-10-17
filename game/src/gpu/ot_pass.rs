//! 512-Bucket GPU Z-Ordering Table (OT) & DMA Channel 2 Linked-List Pass (`NO1-029`).
//!
//! Provides $O(1)$ bucket depth-sorting for 3D scene primitives (3D roadside props,
//! checkpoint arches, start gantry, 3D car meshes, bridges) and submits the entire
//! sorted linked list to GP0 via hardware DMA Channel 2 (`D2_CHCR = 0x01000401`).

use crate::gpu::camera::{M7_Z_FAR, M7_Z_NEAR};
use psx_gpu::{
    material::{BlendMode, TextureMaterial},
    ot::OrderingTable,
    prim::{LineMono, QuadFlat, QuadGouraud, QuadGouraudBlended, TriFlat, TriGouraud},
};
use psx_hw::gpu::{gp0, pack_color, pack_vertex};

/// Number of Z-buckets in the hardware Ordering Table.
pub const OT_BUCKETS: usize = 512;

/// Capacity of the static primitive packet arena in 32-bit words (`2,048` words = 8 KiB).
const OT_ARENA_WORDS: usize = 2048;

#[repr(C, align(4))]
struct OtState {
    ot: OrderingTable<OT_BUCKETS>,
    arena: [u32; OT_ARENA_WORDS],
    cursor: usize,
    active: bool,
}

static mut OT_STATE: OtState = OtState {
    ot: OrderingTable::new(),
    arena: [0; OT_ARENA_WORDS],
    cursor: 0,
    active: false,
};

/// Resets the 512-bucket Ordering Table and packet arena for a new 3D/2.5D scene pass.
pub fn begin_ot_pass() {
    let state = unsafe { &mut *core::ptr::addr_of_mut!(OT_STATE) };
    state.ot.clear();
    state.cursor = 0;
    state.active = true;
}

/// Submits all queued packets in the 512-bucket Ordering Table via DMA Channel 2
/// linked-list mode (farthest slot `511` to nearest slot `0`) and waits for completion.
pub fn flush_ot_pass() {
    let state = unsafe { &mut *core::ptr::addr_of_mut!(OT_STATE) };
    if !state.active {
        return;
    }
    if state.cursor > 0 {
        state.ot.submit_async();
        psx_gpu::submit_linked_list_wait();
        state.ot.clear();
        state.cursor = 0;
    }
    state.active = false;
}

/// Maps a Mode 7 camera-space depth `z_q4` into an Ordering Table bucket `0..512`.
///
/// Larger `z_q4` (farther from the camera) maps to higher bucket indices (`..511`),
/// which the DMA walker visits first; smaller `z_q4` (closer to the camera) maps to
/// lower bucket indices (`0..`), which draw last on top of distant geometry.
#[inline]
pub fn m7_depth_to_ot_slot(z_q4: i32) -> usize {
    let near = M7_Z_NEAR << 4;
    let far = M7_Z_FAR << 4;
    let clamped = z_q4.clamp(near, far);
    let span = (far - near).max(1);
    (((clamped - near) * (OT_BUCKETS as i32 - 1)) / span) as usize
}

/// Maps a top-down screen Y coordinate (`-32..=272`) into an Ordering Table bucket `0..512`
/// so objects higher on screen (smaller `y`) draw before objects lower on screen (larger `y`).
#[inline]
pub fn screen_y_to_ot_slot(sy: i16) -> usize {
    let clamped = (sy as i32).clamp(-32, 272);
    let inv = 272 - clamped; // larger inv = higher on screen = drawn first
    ((inv * (OT_BUCKETS as i32 - 1)) / 304) as usize
}

/// Allocates `total_words` (`1 + data_words`) from the static packet arena, flushing
/// the current OT via DMA first if the arena is full.
#[inline]
fn alloc_packet_words(z_slot: usize, data_words: u8) -> Option<*mut u32> {
    let total = 1 + data_words as usize;
    let state = unsafe { &mut *core::ptr::addr_of_mut!(OT_STATE) };
    if !state.active {
        return None;
    }
    if state.cursor + total > OT_ARENA_WORDS {
        // Flush current chain via DMA Channel 2 and reuse the arena
        state.ot.submit_async();
        psx_gpu::submit_linked_list_wait();
        state.ot.clear();
        state.cursor = 0;
    }
    let ptr = unsafe { state.arena.as_mut_ptr().add(state.cursor) };
    state.cursor += total;
    let slot = z_slot.min(OT_BUCKETS - 1);
    unsafe {
        state.ot.insert(slot, ptr, data_words);
    }
    Some(ptr)
}

/// Inserts a flat-shaded quad into the Ordering Table at `z_slot` (or draws immediately if inactive).
#[inline]
pub fn draw_quad_flat_ot(z_slot: usize, verts: [(i16, i16); 4], r: u8, g: u8, b: u8) {
    if let Some(ptr) = alloc_packet_words(z_slot, QuadFlat::WORDS) {
        let pkt = QuadFlat::new(verts, r, g, b);
        unsafe {
            *ptr.add(1) = pkt.color_cmd;
            *ptr.add(2) = pkt.v0;
            *ptr.add(3) = pkt.v1;
            *ptr.add(4) = pkt.v2;
            *ptr.add(5) = pkt.v3;
        }
    } else {
        psx_gpu::draw_quad_flat(verts, r, g, b);
    }
}

/// Inserts a blended flat-shaded quad into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_quad_flat_blended_ot(
    z_slot: usize,
    verts: [(i16, i16); 4],
    r: u8,
    g: u8,
    b: u8,
    blend_mode: BlendMode,
) {
    if !blend_mode.is_translucent() {
        draw_quad_flat_ot(z_slot, verts, r, g, b);
        return;
    }
    if let Some(ptr) = alloc_packet_words(z_slot, 6) {
        let draw_mode = TextureMaterial::blended(0, 0, (r, g, b), blend_mode).draw_mode_word();
        let cmd = gp0::polygon_opcode(false, true, false, true, false) | pack_color(r, g, b);
        unsafe {
            *ptr.add(1) = draw_mode;
            *ptr.add(2) = cmd;
            *ptr.add(3) = pack_vertex(verts[0].0, verts[0].1);
            *ptr.add(4) = pack_vertex(verts[1].0, verts[1].1);
            *ptr.add(5) = pack_vertex(verts[2].0, verts[2].1);
            *ptr.add(6) = pack_vertex(verts[3].0, verts[3].1);
        }
    } else {
        psx_gpu::draw_quad_flat_blended(verts, r, g, b, blend_mode);
    }
}

/// Inserts a Gouraud-shaded quad into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_quad_gouraud_ot(z_slot: usize, verts: [(i16, i16); 4], colors: [(u8, u8, u8); 4]) {
    if let Some(ptr) = alloc_packet_words(z_slot, QuadGouraud::WORDS) {
        let pkt = QuadGouraud::new(verts, colors);
        unsafe {
            *ptr.add(1) = pkt.color0_cmd;
            *ptr.add(2) = pkt.v0;
            *ptr.add(3) = pkt.color1;
            *ptr.add(4) = pkt.v1;
            *ptr.add(5) = pkt.color2;
            *ptr.add(6) = pkt.v2;
            *ptr.add(7) = pkt.color3;
            *ptr.add(8) = pkt.v3;
        }
    } else {
        psx_gpu::draw_tri_gouraud(
            [verts[0], verts[1], verts[2]],
            [colors[0], colors[1], colors[2]],
        );
        psx_gpu::draw_tri_gouraud(
            [verts[1], verts[2], verts[3]],
            [colors[1], colors[2], colors[3]],
        );
    }
}

/// Inserts a blended Gouraud-shaded quad into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_quad_gouraud_blended_ot(
    z_slot: usize,
    verts: [(i16, i16); 4],
    colors: [(u8, u8, u8); 4],
    blend_mode: BlendMode,
) {
    if !blend_mode.is_translucent() {
        draw_quad_gouraud_ot(z_slot, verts, colors);
        return;
    }
    if let Some(ptr) = alloc_packet_words(z_slot, QuadGouraudBlended::WORDS) {
        let pkt = QuadGouraudBlended::new(verts, colors, blend_mode);
        unsafe {
            *ptr.add(1) = pkt.draw_mode;
            *ptr.add(2) = pkt.color0_cmd;
            *ptr.add(3) = pkt.v0;
            *ptr.add(4) = pkt.color1;
            *ptr.add(5) = pkt.v1;
            *ptr.add(6) = pkt.color2;
            *ptr.add(7) = pkt.v2;
            *ptr.add(8) = pkt.color3;
            *ptr.add(9) = pkt.v3;
        }
    } else {
        psx_gpu::draw_tri_gouraud_blended(
            [verts[0], verts[1], verts[2]],
            [colors[0], colors[1], colors[2]],
            blend_mode,
        );
        psx_gpu::draw_tri_gouraud_blended(
            [verts[1], verts[2], verts[3]],
            [colors[1], colors[2], colors[3]],
            blend_mode,
        );
    }
}

/// Inserts a flat-shaded triangle into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_tri_flat_ot(z_slot: usize, verts: [(i16, i16); 3], r: u8, g: u8, b: u8) {
    if let Some(ptr) = alloc_packet_words(z_slot, TriFlat::WORDS) {
        let pkt = TriFlat::new(verts, r, g, b);
        unsafe {
            *ptr.add(1) = pkt.color_cmd;
            *ptr.add(2) = pkt.v0;
            *ptr.add(3) = pkt.v1;
            *ptr.add(4) = pkt.v2;
        }
    } else {
        psx_gpu::draw_tri_flat(verts, r, g, b);
    }
}

/// Inserts a Gouraud-shaded triangle into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_tri_gouraud_ot(z_slot: usize, verts: [(i16, i16); 3], colors: [(u8, u8, u8); 3]) {
    if let Some(ptr) = alloc_packet_words(z_slot, TriGouraud::WORDS) {
        let pkt = TriGouraud::new(verts, colors);
        unsafe {
            *ptr.add(1) = pkt.color0_cmd;
            *ptr.add(2) = pkt.v0;
            *ptr.add(3) = pkt.color1;
            *ptr.add(4) = pkt.v1;
            *ptr.add(5) = pkt.color2;
            *ptr.add(6) = pkt.v2;
        }
    } else {
        psx_gpu::draw_tri_gouraud(verts, colors);
    }
}

/// Inserts a blended Gouraud-shaded triangle into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_tri_gouraud_blended_ot(
    z_slot: usize,
    verts: [(i16, i16); 3],
    colors: [(u8, u8, u8); 3],
    blend_mode: BlendMode,
) {
    if !blend_mode.is_translucent() {
        draw_tri_gouraud_ot(z_slot, verts, colors);
        return;
    }
    if let Some(ptr) = alloc_packet_words(z_slot, 7) {
        let draw_mode = TextureMaterial::blended(0, 0, colors[0], blend_mode).draw_mode_word();
        let op = gp0::polygon_opcode(true, false, false, true, false);
        unsafe {
            *ptr.add(1) = draw_mode;
            *ptr.add(2) = op | pack_color(colors[0].0, colors[0].1, colors[0].2);
            *ptr.add(3) = pack_vertex(verts[0].0, verts[0].1);
            *ptr.add(4) = pack_color(colors[1].0, colors[1].1, colors[1].2);
            *ptr.add(5) = pack_vertex(verts[1].0, verts[1].1);
            *ptr.add(6) = pack_color(colors[2].0, colors[2].1, colors[2].2);
            *ptr.add(7) = pack_vertex(verts[2].0, verts[2].1);
        }
    } else {
        psx_gpu::draw_tri_gouraud_blended(verts, colors, blend_mode);
    }
}

/// Inserts a monochrome line into the Ordering Table at `z_slot`.
#[inline]
pub fn draw_line_mono_ot(z_slot: usize, p0: (i16, i16), p1: (i16, i16), r: u8, g: u8, b: u8) {
    if let Some(ptr) = alloc_packet_words(z_slot, LineMono::WORDS) {
        let pkt = LineMono::new(p0.0, p0.1, p1.0, p1.1, r, g, b);
        unsafe {
            *ptr.add(1) = pkt.color_cmd;
            *ptr.add(2) = pkt.v0;
            *ptr.add(3) = pkt.v1;
        }
    } else {
        psx_gpu::draw_line_mono(p0.0, p0.1, p1.0, p1.1, r, g, b);
    }
}
