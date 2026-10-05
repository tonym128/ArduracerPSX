//! Hardware-free state for particles and skidmarks.
//!
//! Split from the renderers (`particles.rs`, `skidmarks.rs`) so `tools/test_ui`
//! can exercise the simulation. The renderers only project through the camera
//! and issue GP0 draws; everything that decides *what exists and how it decays*
//! lives here, where it can be checked.
//!
//! This exists because of TASK-1206, where four separate faults made the
//! effects invisible:
//!
//! - Particles were drawn *before* the cars, so the opaque body quad painted
//!   over every puff emitted at the car's own position.
//! - Smoke was emitted with zero velocity at the car's exact centre, so a puff
//!   never drifted away from the thing that made it and never cleared the body.
//! - `max_life` was written in three places and read in none: the renderer
//!   branched on absolute `life` values, so a particle's appearance depended on
//!   which band its countdown happened to be in this frame rather than on how
//!   far through its life it was.
//! - The skidmark ring held 96 slots stamped with `life: 180`, so every mark was
//!   overwritten at ~53 % of its nominal life and the fade branch was
//!   unreachable in practice.
//!
//! The decay is expressed as a fraction of `max_life` here, so a particle looks
//! the same at the same *age* regardless of how long it was configured to live.

use arduracer_core::{math, Fixed, Vec2};

pub const MAX_PARTICLES: usize = 64;
pub const MAX_SKIDMARKS: usize = 96;

/// Frames a tire-smoke puff lives, and a spark.
pub const SMOKE_LIFE: u8 = 28;
pub const SPARK_LIFE: u8 = 14;

/// Frames a skidmark persists.
///
/// Must be no greater than `MAX_SKIDMARKS`, because the buffer is a ring
/// stamped at most once per frame: a longer life would mean marks are always
/// overwritten before they finish fading.
pub const SKIDMARK_LIFE: u8 = MAX_SKIDMARKS as u8;

/// Fraction of `max_life` remaining below which a skidmark reads as faded.
pub const SKIDMARK_FADE_FROM: u8 = 30;

/// Smoke's initial drift, in Q20.12 world units per frame (0x0800 = half a
/// world unit, roughly half a pixel at rest zoom).
///
/// The old code emitted smoke with `vel = ZERO`, so a puff sat exactly on the
/// car that made it and was painted over by the opaque body quad every frame.
const SMOKE_DRIFT: i32 = 0x0800;
/// Velocity lost per frame. Smoke slows as it disperses.
const SMOKE_DRAG_PERMILLE: i32 = 180;
/// Spark launch speed.
const SPARK_SPEED: i32 = 2048;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ParticleType {
    #[default]
    None,
    /// White/grey tire smoke emitted during drift.
    TireSmoke,
    /// Bright yellow/orange sparks from wall scrape or underbody bottoming out.
    Sparks,
}

impl ParticleType {
    /// Whether this particle is drawn at all.
    pub const fn is_live(self) -> bool {
        !matches!(self, ParticleType::None)
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Particle {
    pub ptype: ParticleType,
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: u8,
    pub max_life: u8,
}

impl Particle {
    /// How far through its life this particle is: 0 at birth, `ONE` at expiry.
    ///
    /// Returns `ONE` for a dead particle so callers never divide by zero, and
    /// clamps above so a life that somehow exceeded `max_life` cannot index
    /// past a lookup table.
    pub fn age_fraction(&self) -> u8 {
        if self.max_life == 0 {
            return Self::ONE;
        }
        let elapsed = self.max_life.saturating_sub(self.life);
        let scaled = (elapsed as u16 * Self::ONE as u16) / self.max_life as u16;
        scaled.min(Self::ONE as u16) as u8
    }

    /// One eighth of full life, as a percentage.
    pub const ONE: u8 = 255;
}

/// Fixed-capacity particle pool.
pub struct ParticleSystem {
    pub pool: [Particle; MAX_PARTICLES],
    /// Deterministic spread counter, so consecutive puffs do not all take the
    /// same direction without needing an RNG (and without needing a seed).
    spread: u8,
}

impl Default for ParticleSystem {
    fn default() -> Self {
        Self::new()
    }
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
            spread: 0,
        }
    }

    /// Claims the first free slot, or `None` when the pool is full.
    ///
    /// Returning the slot rather than silently breaking is what lets `emit_smoke`
    /// report whether the puff actually happened.
    fn claim(&mut self) -> Option<usize> {
        let idx = self.pool.iter().position(|p| !p.ptype.is_live())?;
        Some(idx)
    }

    /// Emits a puff of tire smoke that drifts away from the car.
    ///
    /// The old version placed the puff at the car's exact centre with zero
    /// velocity, so it sat inside the opaque body quad and never moved. It now
    /// starts slightly behind and beside the car and drifts, which both clears
    /// the body and reads as smoke.
    pub fn emit_smoke(&mut self, pos: Vec2) -> bool {
        let Some(idx) = self.claim() else {
            return false;
        };
        self.spread = self.spread.wrapping_add(37);
        // Alternate the lateral side so two wheels' worth of smoke does not
        // stack into one column.
        let side = if self.spread & 1 == 0 { 1 } else { -1 };
        let lateral = Fixed::from_raw(side * 0x1000);
        self.pool[idx] = Particle {
            ptype: ParticleType::TireSmoke,
            // Spawn behind and beside the car, not on its centre.
            pos: Vec2::new(
                pos.x + Fixed::from_raw(lateral.raw() / 2),
                pos.y + Fixed::from_raw(-SMOKE_DRIFT * 2),
            ),
            vel: Vec2::new(lateral, Fixed::from_raw(-SMOKE_DRIFT)),
            life: SMOKE_LIFE,
            max_life: SMOKE_LIFE,
        };
        true
    }

    /// Emits a shower of wall scraping sparks.
    pub fn emit_sparks(&mut self, pos: Vec2, normal: Vec2) -> u32 {
        let dirs = [
            Vec2::new(normal.y, -normal.x),
            Vec2::new(-normal.y, normal.x),
            normal,
        ];
        let mut emitted = 0;
        for dir in dirs {
            if let Some(idx) = self.claim() {
                self.pool[idx] = Particle {
                    ptype: ParticleType::Sparks,
                    pos,
                    vel: dir.scale(Fixed::from_raw(SPARK_SPEED)),
                    life: SPARK_LIFE,
                    max_life: SPARK_LIFE,
                };
                emitted += 1;
            }
        }
        emitted
    }

    /// Advances every live particle by one 60 Hz tick.
    pub fn tick(&mut self) {
        for p in self.pool.iter_mut() {
            if !p.ptype.is_live() {
                continue;
            }
            p.pos = p.pos + p.vel;
            if p.ptype == ParticleType::TireSmoke {
                // Bleed off speed so smoke decelerates as it disperses. Scaled
                // per-axis so a negative velocity decays toward zero instead of
                // accelerating away.
                p.vel.x = Fixed::from_raw((p.vel.x.raw() * (1000 - SMOKE_DRAG_PERMILLE)) / 1000);
                p.vel.y = Fixed::from_raw((p.vel.y.raw() * (1000 - SMOKE_DRAG_PERMILLE)) / 1000);
            }
            if p.life > 0 {
                p.life -= 1;
            }
            if p.life == 0 {
                p.ptype = ParticleType::None;
            }
        }
    }

    /// Number of live particles.
    pub fn live_count(&self) -> usize {
        self.pool.iter().filter(|p| p.ptype.is_live()).count()
    }

    /// Live particles paired with how far through their life each is.
    ///
    /// The renderer uses the fraction rather than raw `life` thresholds, so a
    /// puff looks the same at the same age whatever its configured lifetime.
    pub fn visible(&self) -> impl Iterator<Item = (&Particle, u8)> + '_ {
        self.pool
            .iter()
            .filter(|p| p.ptype.is_live())
            .map(|p| (p, p.age_fraction()))
    }
}

/// One tyre pair's mark, laid down per frame while sliding.
#[derive(Copy, Clone, Debug, Default)]
pub struct Skidmark {
    pub active: bool,
    pub left_pos: Vec2,
    pub right_pos: Vec2,
    pub life: u8,
}

impl Skidmark {
    /// `true` once the mark is old enough to render as faded rubber.
    pub const fn is_faded(&self) -> bool {
        self.life <= SKIDMARK_FADE_FROM
    }
}

/// Circular buffer of skidmarks.
pub struct SkidmarkBuffer {
    pub marks: [Skidmark; MAX_SKIDMARKS],
    pub head: usize,
}

impl Default for SkidmarkBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SkidmarkBuffer {
    pub const fn new() -> Self {
        SkidmarkBuffer {
            marks: [Skidmark {
                active: false,
                left_pos: Vec2::ZERO,
                right_pos: Vec2::ZERO,
                life: 0,
            }; MAX_SKIDMARKS],
            head: 0,
        }
    }

    /// Stamps a pair of tyre marks behind the rear wheels.
    pub fn emit(&mut self, pos: Vec2, angle: u16) {
        let cos_a = math::cos(angle);
        let sin_a = math::sin(angle);

        // Perpendicular to the heading: the wheel track half-width.
        let lateral_x = (cos_a * Fixed::from_int(7)).raw();
        let lateral_y = (sin_a * Fixed::from_int(7)).raw();

        // Rearward from the car centre.
        let rear_x = (-sin_a * Fixed::from_int(10)).raw();
        let rear_y = (cos_a * Fixed::from_int(10)).raw();

        let base_rear = Vec2::new(
            pos.x + Fixed::from_raw(rear_x),
            pos.y + Fixed::from_raw(rear_y),
        );

        self.marks[self.head] = Skidmark {
            active: true,
            left_pos: Vec2::new(
                base_rear.x - Fixed::from_raw(lateral_x),
                base_rear.y - Fixed::from_raw(lateral_y),
            ),
            right_pos: Vec2::new(
                base_rear.x + Fixed::from_raw(lateral_x),
                base_rear.y + Fixed::from_raw(lateral_y),
            ),
            life: SKIDMARK_LIFE,
        };
        self.head = (self.head + 1) % MAX_SKIDMARKS;
    }

    /// Ages every mark by one tick.
    pub fn tick(&mut self) {
        for m in self.marks.iter_mut() {
            if m.active {
                if m.life > 0 {
                    m.life -= 1;
                }
                if m.life == 0 {
                    m.active = false;
                }
            }
        }
    }

    pub fn live_count(&self) -> usize {
        self.marks.iter().filter(|m| m.active).count()
    }

    /// Live marks paired with whether each has reached its faded stage.
    pub fn visible(&self) -> impl Iterator<Item = (&Skidmark, bool)> + '_ {
        self.marks
            .iter()
            .filter(|m| m.active)
            .map(|m| (m, m.is_faded()))
    }
}
