//! Hardware GTE (COP2) 3D & Mode 7 Projection Engine (`NO1-028`).
//!
//! Uses the PlayStation 1's Geometry Transformation Engine (Coprocessor 2) for:
//! - Camera rotation & translation matrix setup (`RT`, `TR`, `OFX`, `OFY`, `H`)
//! - Perspective transformation of single 3D vertices (`RTPS`) and vertex triples (`RTPT`)
//! - Hardware backface culling (`NCLIP`) and 3-vertex depth averaging (`AVSZ3`)
//! - Hardware depth-cueing fog (`DPCS`)
//!
//! Coordinate convention in Mode 7 camera space (Q4 world units = 1/16 world unit):
//! - `X` = lateral offset right of camera center (`dx_q4`)
//! - `Y` = vertical offset (`-height_q4`, since screen Y grows downward and camera sits
//!   at height `M7_CAM_HEIGHT_Q4 = 2080` above the ground plane)
//! - `Z` = forward offset ahead of the camera pivot (`dy_q4`, with `TR.z = M7_Z0 << 4 = 1920`)
//!
//! With `H = 120`, `OFX = 160 << 16`, `OFY = 60 << 16`, a ground point (`height_q4 = 0`)
//! projects to `sx = 160 + right_q4 * 120 / z_q4`, `sy = 60 + 2080 * 120 / z_q4`, matching
//! [`Camera::project_rz`] pixel-for-pixel while accelerating 3D elevated meshes in hardware.

use crate::gpu::camera::{Camera, M7_Z0, M7_Z_FAR, M7_Z_NEAR, SCREEN_W};
use arduracer_core::Vec2;
use psx_gte::{
    math::{Mat3I16, Vec3I16, Vec3I32},
    mfc2, mtc2, ops, scene,
};

/// Camera height above the ground plane in Q4 world units (`130 * 16 = 2080`).
/// Chosen so `M7_CAM_HEIGHT_Q4 * H = 2080 * 120 = 249,600 = M7_K * 16`.
pub const M7_CAM_HEIGHT_Q4: i32 = 2_080;

/// Focal distance `H` in pixels.
pub const M7_GTE_H: u16 = 120;

/// Loads the Mode 7 camera transform (heading yaw, jump pitch and speed-FOV
/// warp) into the hardware GTE control registers.
///
/// Deliberately mirrors [`Camera::project_cam`] exactly: the ground tiles,
/// sprites and particles go through the software path, so any extra term here
/// (such as the corner-banking roll) would slide 3D meshes off their shadows.
/// Banking roll is therefore applied to the sky backdrop only.
pub fn load_m7_camera_gte(camera: &Camera) {
    let h_sin = camera.hsin().clamp(-4096, 4096) as i16;
    let h_cos = camera.hcos().clamp(-4096, 4096) as i16;

    // Input vertex V = (dx_q4, -height_q4, dy_q4) relative to camera.pos:
    //   X_cam = right_q4 = (dx_q4 * h_cos + dy_q4 * h_sin) >> 12
    //   Y_cam = down_q4  = -height_q4 (+ camera height via TR.y)
    //   Z_cam = fwd_q4   = (dx_q4 * h_sin - dy_q4 * h_cos) >> 12 (+ pivot via TR.z)
    let rot = Mat3I16 {
        m: [
            [h_cos, 0, h_sin],
            [0, 4096, 0],
            [h_sin, 0, (-(h_cos as i32)).clamp(-4096, 4096) as i16],
        ],
    };
    scene::load_rotation(&rot);
    scene::load_translation(Vec3I32::new(0, M7_CAM_HEIGHT_Q4, M7_Z0 << 4));

    let horizon_y = camera.m7_horizon().clamp(24, 110);
    scene::set_screen_offset((SCREEN_W as i32 / 2) << 16, horizon_y << 16);

    let h_plane = camera.m7_focal().clamp(96, 140) as u16;
    scene::set_projection_plane(h_plane);
    scene::set_avsz_weights(0x555, 0x400);
}

/// Projects a single 3D world point `(world_pos, height_wu)` through the GTE (`RTPS`).
///
/// Assumes [`load_m7_camera_gte`] has already been called for the current camera.
/// Returns `Some(((sx, sy), z_q4))` if the point lies within `[M7_Z_NEAR..=M7_Z_FAR]`.
#[inline]
pub fn project_world_3d_gte(
    camera: &Camera,
    world_pos: Vec2,
    height_wu: i32,
) -> Option<((i16, i16), i32)> {
    let dx_q4 = ((world_pos.x.raw() - camera.pos.x.raw()) >> 8).clamp(-16_000, 16_000) as i16;
    let dy_q4 = ((world_pos.y.raw() - camera.pos.y.raw()) >> 8).clamp(-16_000, 16_000) as i16;
    let neg_h_q4 = (-(height_wu.clamp(-256, 512) * 16)) as i16;

    let p = scene::project_vertex_scheduled(Vec3I16::new(dx_q4, neg_h_q4, dy_q4));
    let z_q4 = p.sz as i32;
    if !((M7_Z_NEAR << 4)..=(M7_Z_FAR << 4)).contains(&z_q4) {
        return None;
    }
    Some(((p.sx, p.sy), z_q4))
}

/// Projects a single Q4 offset `(dx_q4, dy_q4)` on the ground plane (`height = 0`)
/// using the hardware GTE (`RTPS`).
#[inline]
pub fn project_offset_ground_gte(dx_q4: i32, dy_q4: i32) -> ((i16, i16), i32) {
    let v = Vec3I16::new(
        dx_q4.clamp(-16_000, 16_000) as i16,
        0,
        dy_q4.clamp(-16_000, 16_000) as i16,
    );
    let p = scene::project_vertex_scheduled(v);
    ((p.sx, p.sy), p.sz as i32)
}

/// Projects three 3D world vertices `[v0, v1, v2]` in a single hardware `RTPT` batch.
///
/// Each input is `(world_pos, height_wu)`. Returns the three projected `(sx, sy)` pairs
/// and the hardware-averaged `OTZ` depth in Q4 units (`SZ / 3`), or `None` if all vertices
/// fall outside the Mode 7 near/far frustum.
#[inline]
pub fn project_tri_3d_gte(
    camera: &Camera,
    verts: [(Vec2, i32); 3],
) -> Option<([(i16, i16); 3], i32)> {
    let to_gte = |(pos, h_wu): (Vec2, i32)| -> Vec3I16 {
        let dx = ((pos.x.raw() - camera.pos.x.raw()) >> 8).clamp(-16_000, 16_000) as i16;
        let dy = ((pos.y.raw() - camera.pos.y.raw()) >> 8).clamp(-16_000, 16_000) as i16;
        let ny = (-(h_wu.clamp(-256, 512) * 16)) as i16;
        Vec3I16::new(dx, ny, dy)
    };

    let p = scene::project_triangle_scheduled(to_gte(verts[0]), to_gte(verts[1]), to_gte(verts[2]));
    let z0 = p[0].sz as i32;
    let z1 = p[1].sz as i32;
    let z2 = p[2].sz as i32;
    let zmin = z0.min(z1).min(z2);
    let zmax = z0.max(z1).max(z2);
    if zmax < (M7_Z_NEAR << 4) || zmin > (M7_Z_FAR << 4) {
        return None;
    }
    let avg_z = scene::average_z_triangle() as i32;
    Some((
        [(p[0].sx, p[0].sy), (p[1].sx, p[1].sy), (p[2].sx, p[2].sy)],
        avg_z.max(zmin),
    ))
}

/// Computes the signed 2D screen-space cross product of the most recently projected
/// triangle in the GTE SXY FIFO using the hardware `NCLIP` instruction (`MAC0`).
#[inline]
pub fn nclip_last_triangle() -> i32 {
    unsafe { ops::nclip() };
    mfc2!(24) as i32
}

/// Depth-cues an RGB color toward the current GTE far color (`FC`) using the hardware
/// `DPCS` instruction and an explicit fog factor `fog_q12` (`0` = no fog, `4096` = full fog).
#[inline]
pub fn depth_cue_color_gte(color: (u8, u8, u8), fog_q12: i32) -> (u8, u8, u8) {
    let ir0 = fog_q12.clamp(0, 4096) as u32;
    let rgbc = (color.0 as u32) | ((color.1 as u32) << 8) | ((color.2 as u32) << 16);
    mtc2!(6, rgbc);
    mtc2!(8, ir0);
    unsafe { ops::dpcs() };
    let out = mfc2!(22);
    (
        (out & 0xFF) as u8,
        ((out >> 8) & 0xFF) as u8,
        ((out >> 16) & 0xFF) as u8,
    )
}
