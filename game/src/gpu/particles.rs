//! Fixed-capacity hardware particle engine for smoke, sparks, and skidmarks.
//!
//! Pre-allocates all particle slots in static memory to guarantee zero heap
//! fragmentation on 2 MB PlayStation 1 hardware.

use crate::gpu::camera::Camera;
use arduracer_core::{Fixed, Vec2};
use psx_gpu as gpu;

pub const MAX_PARTICLES: usize = 64;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ParticleType {
    #[default]
    None,
    /// White/grey tire smoke emitted during drift.
    TireSmoke,
    /// Bright yellow/orange sparks from wall scrape or underbody bottoming out.
    Sparks,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Particle {
    pub ptype: ParticleType,
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: u8,
    pub max_life: u8,
}

pub struct ParticleSystem {
    pub pool: [Particle; MAX_PARTICLES],
}

impl ParticleSystem {
    pub const fn new() -> Self {
        ParticleSystem {
            pool: [Particle {
                ptype: ParticleType::None,
                pos: Vec2::ZERO,
                vel: Vec2::ZERO,
                life: 0,
                max_life: 0,
            }; MAX_PARTICLES],
        }
    }

    /// Emits a puff of tire smoke at a world position.
    pub fn emit_smoke(&mut self, pos: Vec2) {
        for p in self.pool.iter_mut() {
            if p.ptype == ParticleType::None {
                p.ptype = ParticleType::TireSmoke;
                p.pos = pos;
                p.vel = Vec2::ZERO;
                p.life = 20;
                p.max_life = 20;
                break;
            }
        }
    }

    /// Emits a shower of wall scraping sparks.
    pub fn emit_sparks(&mut self, pos: Vec2, normal: Vec2) {
        let spark_dirs = [
            Vec2::new(normal.y, -normal.x),
            Vec2::new(-normal.y, normal.x),
            normal,
        ];
        for dir in spark_dirs {
            for p in self.pool.iter_mut() {
                if p.ptype == ParticleType::None {
                    p.ptype = ParticleType::Sparks;
                    p.pos = pos;
                    p.vel = dir.scale(Fixed::from_raw(2048));
                    p.life = 12;
                    p.max_life = 12;
                    break;
                }
            }
        }
    }

    /// Updates all active particles by one 60Hz tick.
    pub fn tick(&mut self) {
        for p in self.pool.iter_mut() {
            if p.ptype != ParticleType::None {
                p.pos = p.pos + p.vel;
                if p.life > 0 {
                    p.life -= 1;
                }
                if p.life == 0 {
                    p.ptype = ParticleType::None;
                }
            }
        }
    }

    /// Renders active particles within the camera viewport.
    pub fn render(&self, camera: &Camera, draw_y: i16) {
        for p in self.pool.iter() {
            if p.ptype == ParticleType::None {
                continue;
            }
            let (sx, sy) = camera.world_to_screen(p.pos, draw_y);
            if sx < -8 || sx > 328 || sy < -8 || sy > 248 {
                continue;
            }

            match p.ptype {
                ParticleType::TireSmoke => {
                    // Expanding fading smoke puff (3x3 to 5x5)
                    let size = if p.life < 10 { 5 } else { 3 };
                    gpu::fill_rect(
                        (sx - size / 2) as u16,
                        (sy - size / 2) as u16,
                        size as u16,
                        size as u16,
                        180,
                        185,
                        190,
                    );
                }
                ParticleType::Sparks => {
                    // 2x2 bright yellow spark pixel
                    gpu::fill_rect((sx - 1) as u16, (sy - 1) as u16, 2, 2, 255, 220, 60);
                }
                ParticleType::None => {}
            }
        }
    }
}
