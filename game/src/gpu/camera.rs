//! Dynamic 2.5D Overhead Camera Controller.
//!
//! Follows the player vehicle with:
//! - Speed-dependent zoom, actually applied in [`Camera::world_to_screen`]
//! - Forward look-ahead along the vehicle's heading, so the car sits behind
//!   screen centre and the driver sees where they are going
//! - Clamping to the circuit bounds, so the view never leaves the track
//! - Smooth exponential tracking (lerp) eliminating harsh jitter
//!
//! Two things feed the follow, and both are filtered rather than sampled: the
//! camera *position* eases toward the car, and the look-ahead *direction* eases
//! toward the direction of travel. Filtering only the position is not enough --
//! the lead is long enough that a flickering direction puts a jittered target in
//! front of the lerp, which then tracks the jitter faithfully.
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

/// How fast the look-ahead *direction* eases toward the direction of travel,
/// 1/4th of the gap per 60 Hz tick.
///
/// Faster than [`POSITION_EASE`] would leave the lead swinging the full
/// `LOOKAHEAD_FAST` units on a hard corner entry, which is the wobble this
/// filter exists to stop; slower would add a visible lag into the corner, which
/// is the one thing the lead is for.
const LEAD_DIR_EASE: i32 = 1_024;

/// Speed below which the direction of travel is noise rather than intent, in raw
/// fixed-point units. ~18 world units/second, which is walking pace.
///
/// Under it the velocity vector points at whatever the tyres last did -- a wall
/// scrape, a steering correction, the creep of a wheel off line -- not at where
/// the car is going. Below this the lead direction is held instead of chased.
const LEAD_DIR_HOLD_SPEED: i32 = 1_200;

/// Camera state in world coordinates.
#[derive(Copy, Clone, Debug)]
pub struct Camera {
    /// World position of the camera center (Fixed point Q20.12).
    pub pos: Vec2,
    /// Zoom factor (1.0 = standard, < 1.0 = zoomed out).
    pub zoom: Fixed,
    /// Smoothed unit direction the look-ahead lead is applied along.
    ///
    /// Filter state rather than an input: it is only meaningful relative to the
    /// previous tick's velocity, so it is not public. Starts pointing up the
    /// screen, which is the standstill default -- a stationary car sits low.
    lead_dir: Vec2,
}

/// The direction the look-ahead starts at: up the screen, so the car renders
/// below centre rather than dead on it.
const LEAD_DIR_INITIAL: Vec2 = Vec2 {
    x: Fixed::ZERO,
    y: Fixed(-FP_ONE),
};

impl Default for Camera {
    fn default() -> Self {
        Camera {
            pos: Vec2::ZERO,
            zoom: Fixed::from_raw(ZOOM_REST),
            lead_dir: LEAD_DIR_INITIAL,
        }
    }
}

impl Camera {
    pub fn new(initial_pos: Vec2) -> Self {
        Camera {
            pos: initial_pos,
            zoom: Fixed::from_raw(ZOOM_REST),
            lead_dir: LEAD_DIR_INITIAL,
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
        let dir = self.lead_direction(target_velocity, speed);
        let desired_pos = Vec2 {
            x: target_pos.x + dir.x * Fixed::from_int(lead),
            y: target_pos.y + dir.y * Fixed::from_int(lead),
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

    /// Smoothed direction the look-ahead is applied along, and updates it.
    ///
    /// The lead is `LOOKAHEAD_FAST` units long at speed, so a direction that
    /// flickers frame to frame does not average out -- it swings the target
    /// position by its whole length, and the position lerp then faithfully
    /// chases a target that keeps jumping. Filtering the direction is what makes
    /// the follow smooth; filtering only the position cannot be, because the
    /// jitter is already inside the target by the time the lerp sees it.
    fn lead_direction(&mut self, velocity: Vec2, speed: Fixed) -> Vec2 {
        // Below walking pace the velocity vector points at tyre noise rather than
        // at where the car is going, so hold the last good heading. Holding is
        // what keeps a car sitting still from wandering, and what keeps a car
        // pinned against a barrier from having its lead slosh about.
        if speed.raw() >= LEAD_DIR_HOLD_SPEED {
            let len = velocity.length();
            if len.raw() != 0 {
                let want = velocity.scale(Fixed::ONE / len);
                self.lead_dir.x =
                    self.lead_dir.x + (want.x - self.lead_dir.x) * Fixed::from_raw(LEAD_DIR_EASE);
                self.lead_dir.y =
                    self.lead_dir.y + (want.y - self.lead_dir.y) * Fixed::from_raw(LEAD_DIR_EASE);
            }
        }
        self.lead_dir
    }

    /// Keeps the visible rectangle inside the circuit.
    ///
    /// Where the circuit is narrower than the view, the camera centres on that
    /// axis instead of clamping to an edge, so it never sits off-world.
    pub fn clamp_to_bounds(&mut self, world_w: i32, world_h: i32) {
        let (half_w, half_h) = self.visible_half_extents();
        self.pos.x = clamp_axis(self.pos.x, half_w, world_w);
        self.pos.y = clamp_axis(self.pos.y, half_h, world_h);
    }
}

/// Clamps a camera axis so the visible rectangle stays within `0..world`.
///
/// If the world is smaller than the view on this axis the camera centres on the
/// world rather than clamping, otherwise it would sit at a corner and show more
/// out-of-bounds on the other side.
///
/// The clamp is on the Q20.12 value, deliberately. This used to round-trip
/// through `to_int()` -- `Fixed::from_int(cam.to_int().clamp(lo, hi))` -- which
/// quantised the camera to whole world units *every* frame, whether or not the
/// clamp had anything to do. A world unit is a screen pixel at rest zoom and the
/// camera covers several units per frame at speed, so the camera advanced in
/// whole-pixel steps while the car moved smoothly: the car's screen position
/// juddered by a pixel every frame, which is what made the stage feel clunky
/// rather than the follow feeling soft. The arithmetic shift also floors, so the
/// snap was asymmetric about the origin.
fn clamp_axis(cam: Fixed, half: i32, world: i32) -> Fixed {
    if world <= 2 * half {
        return Fixed::from_int(world / 2);
    }
    cam.clamp(Fixed::from_int(half), Fixed::from_int(world - half))
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
