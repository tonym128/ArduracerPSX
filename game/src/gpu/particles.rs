//! Particle renderer: projects the hardware-free pool in `effects_sim` and
//! issues GP0 draws.
//!
//! The simulation lives in `effects_sim.rs` so `tools/test_ui` can check it;
//! this file only decides pixels.

use crate::gpu::camera::Camera;
use crate::gpu::effects_sim::{ParticleSystem, ParticleType};
use psx_gpu as gpu;

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
                    // Grows and darkens as it ages, driven by `age_fraction`
                    // rather than absolute `life` bands. Three steps, because
                    // semi-transparency is not available (TASK-1205) and the
                    // fade has to be carried by brightness instead.
                    let (half, level) = Self::smoke_appearance(age);
                    let (r, g, b) = (230 - level, 235 - level, 240 - level);
                    gpu::draw_rect_flat(
                        sx - half,
                        sy - half,
                        (half * 2 + 1) as u16,
                        (half * 2 + 1) as u16,
                        r,
                        g,
                        b,
                    );
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

    /// Maps an age fraction to a quad half-extent and a brightness drop.
    ///
    /// Split out so the mapping is checkable arithmetic rather than buried
    /// `if` chains in the draw loop.
    pub fn smoke_appearance(age: u8) -> (i16, u8) {
        match age {
            0..=63 => (SMOKE_SMALL, 0),
            64..=159 => (SMOKE_SMALL + 1, 40),
            _ => (SMOKE_LARGE, 95),
        }
    }
}
