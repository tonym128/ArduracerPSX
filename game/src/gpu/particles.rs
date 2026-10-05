//! Particle renderer: projects the hardware-free pool in `effects_sim` and
//! issues GP0 draws.
//!
//! The simulation lives in `effects_sim.rs` so `tools/test_ui` can check it;
//! this file only decides pixels.

use crate::gpu::camera::Camera;
use crate::gpu::effects_sim::{Particle, ParticleSystem, ParticleType};
use psx_gpu as gpu;
use psx_gpu::material::BlendMode;

/// Smoke quad half-extent at full life and at birth, in pixels.
const SMOKE_LARGE: i16 = 4;
const SMOKE_SMALL: i16 = 1;

impl ParticleSystem {
    /// Renders active particles within the camera viewport.
    pub fn render(&self, camera: &Camera, draw_y: i16) {
        for (p, age) in self.visible() {
            let (sx, sy) = camera.world_to_screen(p.pos, draw_y);
            if !(-16..=336).contains(&sx) || !(-16..=256).contains(&sy) {
                continue;
            }

            match p.ptype {
                ParticleType::TireSmoke => {
                    // Grows as it ages and *fades out*. The fade used to be
                    // faked by stepping through three discrete opaque greys,
                    // because nothing in the renderer could blend; with
                    // `draw_quad_flat_blended` there it is a real gradient
                    // (TASK-1205).
                    let half = Self::smoke_half_extent(age);
                    // Older smoke is dimmer as well as larger, so the two cues
                    // reinforce each other.
                    let level = (age as u16 * 110) / 255;
                    let (r, g, b) = (230 - level as u8, 235 - level as u8, 240 - level as u8);
                    let verts = [
                        (sx - half, sy - half),
                        (sx + half, sy - half),
                        (sx + half, sy + half),
                        (sx - half, sy + half),
                    ];
                    gpu::draw_quad_flat_blended(verts, r, g, b, BlendMode::Average);
                }
                ParticleType::Sparks => {
                    let (r, g, b) = if age < 128 {
                        (255, 240, 80) // Brilliant yellow-white
                    } else {
                        (255, 120, 20) // Cooling ember
                    };
                    gpu::draw_rect_flat(sx - 1, sy - 1, 2, 2, r, g, b);
                }
                ParticleType::None => {}
            }
        }
    }

    /// Maps an age fraction to the smoke quad's half-extent, in pixels.
    ///
    /// Split out so the mapping is checkable arithmetic rather than buried in
    /// the draw loop. Linear in age, so the puff expands smoothly instead of
    /// jumping between the three sizes the old opaque fade cycled through.
    pub fn smoke_half_extent(age: u8) -> i16 {
        SMOKE_SMALL
            + ((SMOKE_LARGE - SMOKE_SMALL) as u32 * age as u32 / Particle::ONE as u32) as i16
    }
}
