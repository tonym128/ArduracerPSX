//! Skidmark renderer: projects the hardware-free ring in `effects_sim` and
//! issues GP0 draws.
//!
//! The simulation lives in `effects_sim.rs` so `tools/test_ui` can check it;
//! this file only decides pixels.

use crate::gpu::camera::Camera;
use crate::gpu::effects_sim::SkidmarkBuffer;
use psx_gpu as gpu;

/// Fresh rubber, and the faded colour it fades to.
const FRESH: (u8, u8, u8) = (18, 18, 22);
const FADED: (u8, u8, u8) = (28, 30, 36);

/// Half-extent of one stamp, in pixels.
const STAMP_HALF: i16 = 1;
const STAMP_SIZE: u16 = (STAMP_HALF * 2 + 1) as u16;

impl SkidmarkBuffer {
    /// Renders skidmark stamps to screen relative to the camera.
    pub fn render(&self, camera: &Camera, draw_y: i16) {
        for (m, faded) in self.visible() {
            let (lx, ly) = camera.world_to_screen(m.left_pos, draw_y);
            let (rx, ry) = camera.world_to_screen(m.right_pos, draw_y);
            let colour = if faded { FADED } else { FRESH };

            if (-8..328).contains(&lx) && (-8..248).contains(&ly) {
                gpu::draw_rect_flat(
                    lx - STAMP_HALF,
                    ly - STAMP_HALF,
                    STAMP_SIZE,
                    STAMP_SIZE,
                    colour.0,
                    colour.1,
                    colour.2,
                );
            }
            if (-8..328).contains(&rx) && (-8..248).contains(&ry) {
                gpu::draw_rect_flat(
                    rx - STAMP_HALF,
                    ry - STAMP_HALF,
                    STAMP_SIZE,
                    STAMP_SIZE,
                    colour.0,
                    colour.1,
                    colour.2,
                );
            }
        }
    }
}
