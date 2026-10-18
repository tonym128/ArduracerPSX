//! 3D Roadside Props, TrackMania Checkpoint Arches & Start Gantry Truss (`NO1-031`).
//!
//! Provides hardware-accelerated 3D roadside objects and TrackMania gate structures:
//! - **TrackMania Holographic Checkpoint Arches**: Neon-illuminated arches over each checkpoint.
//!   Pulsing cyan (`(0, 210, 255)`) when pending, glowing emerald green (`(40, 240, 110)`) once passed.
//! - **Overhead Start/Finish Gantry Truss**: Overhead gantry structure with 3-2-1-GO light clusters
//!   synced to [`StartSequence`].
//! - **Deterministic Biome Roadside Props**: Low-poly 3D props (Speedway palm trees, Neon City
//!   cyber-pylons, Canyon mesas, Alpine pines, Obsidian obelisks, Stadium floodlight towers)
//!   placed along route margins with zero floating artifacts.

use crate::gpu::camera::{Camera, M7_Z_FAR, M7_Z_NEAR};
use crate::gpu::gte_proj::load_m7_camera_gte;
use arduracer_core::math::{cos, sin};
use arduracer_core::timing::{CheckpointGate, StartPhase, StartSequence};
use arduracer_core::{Fixed, TrackDef, Vec2, FP_SHIFT};
use psx_gpu as gpu;

/// Renders all 3D roadside props, TrackMania checkpoint arches, and the start gantry (`NO1-031`).
pub fn render_track_props(
    camera: &Camera,
    track: &TrackDef,
    circuit_id: usize,
    checkpoint_mask: u64,
    start_seq: Option<&StartSequence>,
    ticks: u32,
) {
    if !camera.mode7 {
        return;
    }

    // Ensure GTE registers are loaded with the active camera transform
    load_m7_camera_gte(camera);

    let sky = crate::gpu::palette::sky_palette_for_circuit(circuit_id);
    let biome_style = sky.biome_style;

    // 1. Render Start / Finish Gantry Truss
    render_start_gantry(camera, &track.start_gate, start_seq, ticks);

    // 2. Render TrackMania Holographic Checkpoint Arches
    for (i, gate) in track.checkpoints.iter().enumerate() {
        if !gate.is_active() {
            continue;
        }
        let cleared = (checkpoint_mask & (1u64 << i)) != 0;
        render_checkpoint_arch(camera, gate, cleared, ticks);
    }

    // 3. Render Deterministic Roadside Props along track route
    render_biome_props(camera, track, biome_style, ticks);
}

/// Computes the world center position of a checkpoint or start gate.
#[inline]
fn gate_world_center(gate: &CheckpointGate) -> Vec2 {
    let cx = (gate.x as i32 * 64) + (gate.width as i32 * 32);
    let cy = (gate.y as i32 * 64) + (gate.height as i32 * 32);
    Vec2 {
        x: Fixed::from_int(cx),
        y: Fixed::from_int(cy),
    }
}

/// Fast frustum and depth check: returns projected screen coordinates `(sx, sy)` and `z_q4`.
///
/// The projection itself belongs to [`Camera`], so props land in exactly the
/// same place the software track tiles do in every split layout. This used to
/// re-derive it from the full-screen constants (`320 / 2`, `M7_HORIZON`), which
/// put arches and gantries off-centre in every 2P viewport.
#[inline]
fn project_prop_point(camera: &Camera, pos: Vec2, height_wu: i32) -> Option<((i16, i16), i32)> {
    let (right, z_q4) = camera.view_rq4_zq4(pos);
    if !((M7_Z_NEAR << 4)..=(M7_Z_FAR << 4)).contains(&z_q4) {
        return None;
    }
    // Lateral culling cone
    if (right.abs() * 5) > (z_q4 * 7) + (32 << 4) {
        return None;
    }
    let (sx, sy) = camera.project_cam_elevated(right, z_q4, height_wu);
    // Cull against this viewport, not the whole screen, with a margin so a prop
    // straddling the edge still contributes its visible part.
    let half_w = camera.viewport_w as i32 / 2 + 40;
    let lo_x = camera.center_x as i32 - half_w;
    let hi_x = camera.center_x as i32 + half_w;
    let hi_y = camera.viewport_h as i32 + 40;
    if (sx as i32) < lo_x || (sx as i32) > hi_x || (sy as i32) < -40 || (sy as i32) > hi_y {
        return None;
    }
    Some(((sx, sy), z_q4))
}

/// Renders the 3D Start / Finish Gantry Truss with 3-2-1-GO light clusters (`NO1-031`).
fn render_start_gantry(
    camera: &Camera,
    gate: &CheckpointGate,
    start_seq: Option<&StartSequence>,
    ticks: u32,
) {
    if !gate.is_active() {
        return;
    }
    let center = gate_world_center(gate);
    let is_wide = gate.width >= gate.height;
    let span_wu = if is_wide {
        gate.width as i32 * 32
    } else {
        gate.height as i32 * 32
    };

    let (dx_wu, dy_wu) = if is_wide { (span_wu, 0) } else { (0, span_wu) };

    let left_pos = Vec2::new(
        center.x - Fixed::from_int(dx_wu),
        center.y - Fixed::from_int(dy_wu),
    );
    let right_pos = Vec2::new(
        center.x + Fixed::from_int(dx_wu),
        center.y + Fixed::from_int(dy_wu),
    );

    // Left column: base and top (H = 26)
    let Some((l_base, _)) = project_prop_point(camera, left_pos, 0) else {
        return;
    };
    let Some((l_top, z_l)) = project_prop_point(camera, left_pos, 26) else {
        return;
    };
    let Some((r_base, _)) = project_prop_point(camera, right_pos, 0) else {
        return;
    };
    let Some((r_top, _)) = project_prop_point(camera, right_pos, 26) else {
        return;
    };

    // Columns: dark metallic steel
    gpu::draw_line_mono(l_base.0, l_base.1, l_top.0, l_top.1, 75, 82, 95);
    gpu::draw_line_mono(l_base.0 + 1, l_base.1, l_top.0 + 1, l_top.1, 110, 120, 135);
    gpu::draw_line_mono(r_base.0, r_base.1, r_top.0, r_top.1, 75, 82, 95);
    gpu::draw_line_mono(r_base.0 + 1, r_base.1, r_top.0 + 1, r_top.1, 110, 120, 135);

    // Overhead crossbar truss
    gpu::draw_line_mono(l_top.0, l_top.1, r_top.0, r_top.1, 130, 140, 155);
    gpu::draw_line_mono(l_top.0, l_top.1 - 2, r_top.0, r_top.1 - 2, 85, 95, 110);

    // 3-2-1-GO Overhead Start Light Pods
    let phase = start_seq.map(|s| s.phase()).unwrap_or(StartPhase::Racing);
    for pod in 0..3 {
        let t_num = pod + 1;
        let p_x = l_top.0 + ((r_top.0 - l_top.0) * t_num) / 4;
        let p_y = l_top.1 + ((r_top.1 - l_top.1) * t_num) / 4 - 1;

        let col = match phase {
            StartPhase::Grid => (40, 40, 45), // Off
            StartPhase::Lit(n) => {
                if (pod as u8) < n {
                    (255, 30, 20) // Vibrant illuminated Red
                } else {
                    (50, 20, 20) // Dim unlit Red
                }
            }
            StartPhase::Go => (30, 255, 60), // All Green GO!
            StartPhase::Racing => {
                if (ticks & 32) != 0 {
                    (20, 220, 70)
                } else {
                    (240, 240, 255)
                }
            }
        };

        let radius = if z_l < (140 << 4) { 2 } else { 1 };
        gpu::draw_rect_flat(
            p_x - radius,
            p_y - radius,
            (radius * 2 + 1) as u16,
            (radius * 2 + 1) as u16,
            col.0,
            col.1,
            col.2,
        );
    }
}

/// Renders a TrackMania Holographic Checkpoint Arch (`NO1-031`).
fn render_checkpoint_arch(camera: &Camera, gate: &CheckpointGate, cleared: bool, ticks: u32) {
    let center = gate_world_center(gate);
    let is_wide = gate.width >= gate.height;
    let span_wu = if is_wide {
        gate.width as i32 * 32
    } else {
        gate.height as i32 * 32
    };

    let (dx_wu, dy_wu) = if is_wide { (span_wu, 0) } else { (0, span_wu) };

    let left_pos = Vec2::new(
        center.x - Fixed::from_int(dx_wu),
        center.y - Fixed::from_int(dy_wu),
    );
    let right_pos = Vec2::new(
        center.x + Fixed::from_int(dx_wu),
        center.y + Fixed::from_int(dy_wu),
    );

    let Some((l_base, _)) = project_prop_point(camera, left_pos, 0) else {
        return;
    };
    let Some((l_top, z_l)) = project_prop_point(camera, left_pos, 22) else {
        return;
    };
    let Some((r_base, _)) = project_prop_point(camera, right_pos, 0) else {
        return;
    };
    let Some((r_top, _)) = project_prop_point(camera, right_pos, 22) else {
        return;
    };

    // Color: Glowing Emerald Green when cleared; pulsing Cyan when pending
    let neon_col = if cleared {
        (35, 245, 115)
    } else {
        let pulse = ((ticks >> 2) & 7) as u8 * 6;
        (pulse, 200 + pulse.min(50), 255)
    };

    // Upright pylon beams
    gpu::draw_line_mono(
        l_base.0, l_base.1, l_top.0, l_top.1, neon_col.0, neon_col.1, neon_col.2,
    );
    gpu::draw_line_mono(
        r_base.0, r_base.1, r_top.0, r_top.1, neon_col.0, neon_col.1, neon_col.2,
    );

    // Overhead holographic arch beam
    gpu::draw_line_mono(
        l_top.0, l_top.1, r_top.0, r_top.1, neon_col.0, neon_col.1, neon_col.2,
    );

    // Chevron chevron arrow / pulsing beacon on arch crossbar
    let mid_x = (l_top.0 + r_top.0) / 2;
    let mid_y = (l_top.1 + r_top.1) / 2;
    let arrow_size = if z_l < (140 << 4) { 4 } else { 2 };
    let chevron_col = if cleared {
        (220, 255, 230)
    } else {
        (200, 250, 255)
    };

    gpu::draw_tri_flat(
        [
            (mid_x - arrow_size, mid_y - arrow_size),
            (mid_x + arrow_size, mid_y - arrow_size),
            (mid_x, mid_y + 1),
        ],
        chevron_col.0,
        chevron_col.1,
        chevron_col.2,
    );
}

/// Renders deterministic roadside props along the track edges (`NO1-031`).
fn render_biome_props(camera: &Camera, track: &TrackDef, biome_style: u8, _ticks: u32) {
    let n = if !track.route.is_empty() {
        track.route.len()
    } else {
        track.route_len()
    };
    if n < 4 {
        return;
    }

    let node_at = |idx: usize| -> Vec2 {
        if !track.route.is_empty() {
            track.route[idx % n]
        } else {
            TrackDef::gate_centre(&track.route_node(idx % n))
        }
    };

    // Step every 4 route nodes to maintain 60 FPS while populating roadside
    for i in (0..n).step_by(4) {
        let p0 = node_at(i);
        let p1 = node_at(i + 1);
        let seg_h = arduracer_core::heading_towards(p0, p1);

        // Place props along both left and right outer margins (+24 and -24 WU offset)
        let lat_x = (cos(seg_h).raw() * 24) >> FP_SHIFT;
        let lat_y = (sin(seg_h).raw() * 24) >> FP_SHIFT;

        let left_pos = Vec2::new(p0.x - Fixed::from_int(lat_x), p0.y - Fixed::from_int(lat_y));
        let right_pos = Vec2::new(p0.x + Fixed::from_int(lat_x), p0.y + Fixed::from_int(lat_y));

        for &pos in &[left_pos, right_pos] {
            match biome_style {
                0 => draw_palm_tree(camera, pos),
                1 => draw_cyber_pylon(camera, pos),
                2 => draw_canyon_spire(camera, pos),
                3 => draw_alpine_pine(camera, pos),
                4 => draw_obsidian_obelisk(camera, pos),
                _ => draw_stadium_floodlight(camera, pos),
            }
        }
    }
}

/// Palm Tree: slender brown trunk with green palm frond canopy.
fn draw_palm_tree(camera: &Camera, pos: Vec2) {
    let Some((base, _)) = project_prop_point(camera, pos, 0) else {
        return;
    };
    let Some((crown, z)) = project_prop_point(camera, pos, 20) else {
        return;
    };
    // Trunk
    gpu::draw_line_mono(base.0, base.1, crown.0, crown.1, 115, 78, 48);
    // Frond canopy
    let rad = if z < (140 << 4) { 6 } else { 3 };
    gpu::draw_tri_flat(
        [
            (crown.0 - rad, crown.1),
            (crown.0 + rad, crown.1),
            (crown.0, crown.1 - rad),
        ],
        38,
        145,
        52,
    );
    gpu::draw_tri_flat(
        [
            (crown.0 - rad + 1, crown.1 + 1),
            (crown.0 + rad - 1, crown.1 + 1),
            (crown.0, crown.1 - rad - 2),
        ],
        55,
        175,
        65,
    );
}

/// Cyber Pylon: high-tech neon beacon column.
fn draw_cyber_pylon(camera: &Camera, pos: Vec2) {
    let Some((base, _)) = project_prop_point(camera, pos, 0) else {
        return;
    };
    let Some((top, z)) = project_prop_point(camera, pos, 18) else {
        return;
    };
    gpu::draw_line_mono(base.0, base.1, top.0, top.1, 45, 52, 70);
    gpu::draw_line_mono(base.0 + 1, base.1, top.0 + 1, top.1, 0, 215, 255);
    let rad = if z < (140 << 4) { 2 } else { 1 };
    gpu::draw_rect_flat(
        top.0 - rad,
        top.1 - rad,
        (rad * 2 + 1) as u16,
        (rad * 2 + 1) as u16,
        255,
        30,
        160,
    );
}

/// Canyon Rock Spire / Mesa pyramid.
fn draw_canyon_spire(camera: &Camera, pos: Vec2) {
    let Some((base, z)) = project_prop_point(camera, pos, 0) else {
        return;
    };
    let Some((peak, _)) = project_prop_point(camera, pos, 24) else {
        return;
    };
    let w = if z < (140 << 4) { 8 } else { 4 };
    gpu::draw_tri_flat(
        [(base.0 - w, base.1), (base.0 + w, base.1), (peak.0, peak.1)],
        140,
        65,
        38,
    );
    gpu::draw_line_mono(base.0, base.1, peak.0, peak.1, 185, 95, 55);
}

/// Alpine Pine: triangular tiered evergreen.
fn draw_alpine_pine(camera: &Camera, pos: Vec2) {
    let Some((base, _)) = project_prop_point(camera, pos, 0) else {
        return;
    };
    let Some((mid, z)) = project_prop_point(camera, pos, 12) else {
        return;
    };
    let Some((top, _)) = project_prop_point(camera, pos, 22) else {
        return;
    };
    // Trunk
    gpu::draw_line_mono(base.0, base.1, mid.0, mid.1, 80, 55, 38);
    // Lower boughs
    let w1 = if z < (140 << 4) { 7 } else { 4 };
    gpu::draw_tri_flat(
        [
            (mid.0 - w1, mid.1 + 2),
            (mid.0 + w1, mid.1 + 2),
            (top.0, top.1 + 4),
        ],
        24,
        82,
        46,
    );
    // Upper crown
    let w2 = if z < (140 << 4) { 5 } else { 3 };
    gpu::draw_tri_flat(
        [
            (mid.0 - w2, top.1 + 5),
            (mid.0 + w2, top.1 + 5),
            (top.0, top.1),
        ],
        36,
        115,
        62,
    );
}

/// Obsidian Cyber Obelisk: dark monolith with cyan line.
fn draw_obsidian_obelisk(camera: &Camera, pos: Vec2) {
    let Some((base, z)) = project_prop_point(camera, pos, 0) else {
        return;
    };
    let Some((top, _)) = project_prop_point(camera, pos, 22) else {
        return;
    };
    let w = if z < (140 << 4) { 3 } else { 2 };
    gpu::draw_quad_flat(
        [
            (base.0 - w, base.1),
            (base.0 + w, base.1),
            (top.0 - w, top.1),
            (top.0 + w, top.1),
        ],
        22,
        26,
        36,
    );
    gpu::draw_line_mono(base.0, base.1, top.0, top.1, 0, 210, 255);
}

/// Stadium Floodlight Tower: tall lattice mast with luminous floodlight head.
fn draw_stadium_floodlight(camera: &Camera, pos: Vec2) {
    let Some((base, _)) = project_prop_point(camera, pos, 0) else {
        return;
    };
    let Some((top, z)) = project_prop_point(camera, pos, 30) else {
        return;
    };
    // Lattice mast
    gpu::draw_line_mono(base.0, base.1, top.0, top.1, 95, 105, 120);
    gpu::draw_line_mono(base.0 + 1, base.1, top.0 + 1, top.1, 140, 150, 165);
    // Floodlight lamp bank
    let w = if z < (140 << 4) { 5 } else { 3 };
    gpu::draw_rect_flat(top.0 - w, top.1 - 2, (w * 2 + 1) as u16, 4, 250, 250, 220);
}
