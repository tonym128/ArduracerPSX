//! Dynamic 2.5D Overhead Camera Controller.
//!
//! Follows the player vehicle with:
//! - Speed-dependent zoom, actually applied in [`Camera::world_to_screen`]
//! - Forward look-ahead along the vehicle's heading, so the car sits behind
//!   screen centre and the driver sees where they are going
//! - Clamping to the circuit bounds, so the view never leaves the track
//! - Smooth exponential tracking (lerp) eliminating harsh jitter
//!
//! Hardware-free: depends only on `arduracer-core`, so `tools/test_ui` exercises
//! it on the host.

use arduracer_core::{Fixed, Vec2, FP_ONE, FP_SHIFT};

pub const SCREEN_W: i16 = 320;
pub const SCREEN_H: i16 = 240;

/// Zoom at a standstill: 1 world unit = 1 pixel.
const ZOOM_REST: i32 = FP_ONE;
/// Zoom at top speed. Below 1.0 means a wider field of view, so the car appears
/// smaller and more of the corner is visible ahead.
const ZOOM_FAST: i32 = 3_200; // ~0.78 => ~28% wider at speed
/// Speed at which the zoom bottoms out, in raw fixed-point units.
const ZOOM_FULL_SPEED: i32 = 14_000;
/// How fast the zoom eases toward its target (1/16th of the gap per tick).
const ZOOM_EASE: i32 = 256;

/// Look-ahead at a standstill, in world units. Small but non-zero, so the car is
/// never dead-centre when stationary.
const LOOKAHEAD_REST: i32 = 10;
/// Look-ahead at top speed, in world units (~4.5 car lengths).
const LOOKAHEAD_FAST: i32 = 72;
/// How fast the position eases toward its target (1/8th per 60 Hz tick).
const POSITION_EASE: i32 = 512;

/// Camera state in world coordinates.
#[derive(Copy, Clone, Debug)]
pub struct Camera {
    /// World position of the camera center (Fixed point Q20.12).
    pub pos: Vec2,
    /// Zoom factor (1.0 = standard, < 1.0 = zoomed out).
    pub zoom: Fixed,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            pos: Vec2::ZERO,
            zoom: Fixed::from_raw(ZOOM_REST),
        }
    }
}

impl Camera {
    pub fn new(initial_pos: Vec2) -> Self {
        Camera {
            pos: initial_pos,
            zoom: Fixed::from_raw(ZOOM_REST),
        }
    }

    /// Half-extents of the visible world rectangle, in world units, taking the
    /// current zoom into account.
    ///
    /// The renderer needs this to work out which tiles can possibly be on
    /// screen; anything outside this rectangle cannot be drawn.
    pub fn visible_half_extents(&self) -> (i32, i32) {
        // `zoom` is a plain ratio, so the world extent grows as it shrinks.
        let z = self.zoom.raw().max(1);
        // Rounded *up*: the tile renderer uses this to choose which tiles to
        // draw, so under-covering would leave a gap of background at the screen
        // edge. One pixel of overdraw is the cheaper error.
        let half_w = div_ceil((SCREEN_W as i32 / 2) * FP_ONE, z);
        let half_h = div_ceil((SCREEN_H as i32 / 2) * FP_ONE, z);
        (half_w, half_h)
    }

    /// World point -> screen pixel, applying zoom.
    ///
    /// This is the single projection for the whole game: cars, particles,
    /// skidmarks and the tile renderer all go through here (or through
    /// [`Camera::world_offset`]), so zoom cannot drift between layers.
    ///
    /// The `draw_y` offset this used to take has been removed. It was threaded
    /// through fifteen call sites and discarded at every one of them:
    /// double-buffering is handled by the deferred `FrameBuffer` swap, so there is
    /// no vertical offset for a renderer to apply. Leaving the parameter in place
    /// invited a future renderer to honour it and silently double-offset
    /// everything (TASK-1216).
    pub fn world_to_screen(&self, world_pos: Vec2) -> (i16, i16) {
        let off = self.world_offset(world_pos);
        // Saturating, not wrapping: `world_offset` clamps each axis to `i16`, so
        // a point far off-screen would overflow on the addition to screen centre.
        (
            add_i16_saturating(SCREEN_W / 2, off.0),
            add_i16_saturating(SCREEN_H / 2, off.1),
        )
    }

    /// World point -> pixel offset from screen centre, with zoom applied.
    ///
    /// Returned in whole pixels; the fractional part is dropped, which at these
    /// zoom levels is at most one pixel of jitter.
    pub fn world_offset(&self, world_pos: Vec2) -> (i16, i16) {
        // `raw()` is Q20.12 and `z` is Q20.12, so the product needs two shifts to
        // come back to pixels.
        let z = self.zoom.raw();
        let rel_x =
            (((world_pos.x - self.pos.x).raw() as i64 * z as i64) >> FP_SHIFT >> FP_SHIFT) as i32;
        let rel_y =
            (((world_pos.y - self.pos.y).raw() as i64 * z as i64) >> FP_SHIFT >> FP_SHIFT) as i32;
        (clamp_i16(rel_x), clamp_i16(rel_y))
    }

    /// Updates camera position and zoom to track a vehicle.
    ///
    /// `world_w` / `world_h` are the circuit's bounds in world units; the camera
    /// is clamped so the view rectangle stays inside them.
    pub fn update(
        &mut self,
        target_pos: Vec2,
        target_velocity: Vec2,
        speed: Fixed,
        world_w: i32,
        world_h: i32,
    ) {
        // Look-ahead leads along the direction of travel. Driven by heading
        // rather than velocity so it survives being slow or pinned against a
        // wall, where a velocity-derived lead collapses to nothing and the car
        // sits dead centre with no warning of what is ahead.
        let speed_ratio = clamp_i32(
            (speed.raw() as i64 * FP_ONE as i64) / ZOOM_FULL_SPEED as i64,
            0,
            FP_ONE,
        );
        let lead = LOOKAHEAD_REST + ((LOOKAHEAD_FAST - LOOKAHEAD_REST) * speed_ratio) / FP_ONE;
        let dir = forward_dir(target_pos, self.pos, target_velocity, speed_ratio);
        let desired_pos = Vec2 {
            x: target_pos.x + Fixed::from_int(dir.0 * lead / 1000),
            y: target_pos.y + Fixed::from_int(dir.1 * lead / 1000),
        };

        // Smooth exponential tracking.
        let diff_x = desired_pos.x - self.pos.x;
        let diff_y = desired_pos.y - self.pos.y;
        self.pos.x = self.pos.x + diff_x * Fixed::from_raw(POSITION_EASE);
        self.pos.y = self.pos.y + diff_y * Fixed::from_raw(POSITION_EASE);

        // Zoom out with speed, so the faster you go the more corner you can see.
        let target_zoom = ZOOM_REST - ((ZOOM_REST - ZOOM_FAST) * speed_ratio) / FP_ONE;
        let zoom_diff = target_zoom - self.zoom.raw();
        self.zoom = Fixed::from_raw(self.zoom.raw() + (zoom_diff * ZOOM_EASE) / FP_ONE);

        self.clamp_to_bounds(world_w, world_h);
    }

    /// Keeps the visible rectangle inside the circuit.
    ///
    /// Where the circuit is narrower than the view, the camera centres on that
    /// axis instead of clamping to an edge, so it never sits off-world.
    pub fn clamp_to_bounds(&mut self, world_w: i32, world_h: i32) {
        let (half_w, half_h) = self.visible_half_extents();
        self.pos.x = clamp_axis(self.pos.x.to_int(), half_w, world_w);
        self.pos.y = clamp_axis(self.pos.y.to_int(), half_h, world_h);
    }
}

/// Clamps a camera axis so the visible rectangle stays within `0..world`.
///
/// If the world is smaller than the view on this axis the camera centres on the
/// world rather than clamping, otherwise it would sit at a corner and show more
/// out-of-bounds on the other side.
fn clamp_axis(cam: i32, half: i32, world: i32) -> Fixed {
    if world <= 2 * half {
        return Fixed::from_int(world / 2);
    }
    let lo = half;
    let hi = world - half;
    Fixed::from_int(cam.clamp(lo, hi))
}

/// Unit forward direction for the camera lead, in whole-number world units.
///
/// Uses velocity when the car is actually moving, because that reflects where
/// the car is *going* mid-drift; falls back to the camera's existing offset for
/// the heading when stationary. The result is scaled by 1000 to stay integral.
fn forward_dir(target: Vec2, cam: Vec2, velocity: Vec2, _speed_ratio: i32) -> (i32, i32) {
    const SCALE: i32 = 1000;
    if velocity.length_squared().raw() > 0 {
        let len = velocity.length().raw();
        if len != 0 {
            return (
                (velocity.x.raw() * SCALE) / len,
                (velocity.y.raw() * SCALE) / len,
            );
        }
    }
    // Stationary: keep the last heading by leaning away from where the camera
    // already sits, so the car settles slightly low on screen at the grid.
    let dx = target.x.raw() - cam.x.raw();
    let dy = target.y.raw() - cam.y.raw();
    if dx == 0 && dy == 0 {
        return (0, -SCALE);
    }
    // Approximate normalisation on the dominant axis: exact trig is wasted here
    // because the magnitude is immediately scaled by `lead` again.
    let ax = dx.abs();
    let ay = dy.abs();
    if ax >= ay {
        (if dx < 0 { -SCALE } else { SCALE }, 0)
    } else {
        (0, if dy < 0 { -SCALE } else { SCALE })
    }
}

/// Integer division rounding away from zero for positive inputs.
fn div_ceil(a: i32, b: i32) -> i32 {
    if b <= 0 {
        return a;
    }
    (a + b - 1) / b
}

fn clamp_i32(v: i64, lo: i32, hi: i32) -> i32 {
    if v < lo as i64 {
        lo
    } else if v > hi as i64 {
        hi
    } else {
        v as i32
    }
}

fn clamp_i16(v: i32) -> i16 {
    clamp_i32(v as i64, i16::MIN as i32, i16::MAX as i32) as i16
}

/// Adds to an `i16` without wrapping.
///
/// `world_offset` clamps each axis to `i16`, so adding screen centre back on
/// could overflow for a point far off-screen -- and in release mode a wrapped
/// coordinate draws a car in the opposite corner instead of culling it.
fn add_i16_saturating(a: i16, b: i16) -> i16 {
    clamp_i32(a as i64 + b as i64, i16::MIN as i32, i16::MAX as i32) as i16
}
