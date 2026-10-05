//! Skidmark renderer: projects the hardware-free ring in `effects_sim` and
//! issues GP0 draws.
//!
//! The simulation lives in `effects_sim.rs` so `tools/test_ui` can check it;
//! this file only decides pixels.

use crate::gpu::camera::Camera;
use crate::gpu::effects_sim::SkidmarkBuffer;
use psx_gpu as gpu;
use psx_gpu::material::BlendMode;

/// Fresh rubber, and the colour it fades toward.
const FRESH: (u8, u8, u8) = (18, 18, 22);
const FADED: (u8, u8, u8) = (28, 30, 36);

/// Draws one stamp with a fade that tracks `life`.
///
/// The fade used to be a single-frame flash between two opaque colours, because
/// the ring was overwritten at ~53 % of its nominal life so almost no mark ever
/// reached the faded branch, *and* because nothing could blend (TASK-1205).
fn stamp(x: i16, y: i16, faded: bool) {
    let colour = if faded { FADED } else { FRESH };
    // A quad rather than a 3x3 rect, so the stamp can be blended: an aged mark
    // now thins toward the road instead of jumping.
    let verts = [
        (x - STAMP_HALF, y - STAMP_HALF),
        (x + STAMP_HALF, y - STAMP_HALF),
        (x + STAMP_HALF, y + STAMP_HALF),
        (x - STAMP_HALF, y + STAMP_HALF),
    ];
    gpu::draw_quad_flat_blended(verts, colour.0, colour.1, colour.2, BlendMode::Average);
}

/// Half-extent of one stamp, in pixels.
const STAMP_HALF: i16 = 1;

impl SkidmarkBuffer {
    /// Renders skidmark stamps to screen relative to the camera.
    pub fn render(&self, camera: &Camera) {
        for (m, faded) in self.visible() {
            let (lx, ly) = camera.world_to_screen(m.left_pos);
            let (rx, ry) = camera.world_to_screen(m.right_pos);
            if (-8..328).contains(&lx) && (-8..248).contains(&ly) {
                stamp(lx, ly, faded);
            }
            if (-8..328).contains(&rx) && (-8..248).contains(&ry) {
                stamp(rx, ry, faded);
            }
        }
    }
}
