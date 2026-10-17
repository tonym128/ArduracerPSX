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

use arduracer_core::{math::cos, math::sin, Fixed, Vec2, FP_ONE, FP_SHIFT};

/// Returned for points a Mode 7 view cannot show; outside every layer's bounds test.
pub const M7_OFFSCREEN: (i16, i16) = (-2000, -2000);

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
/// How fast the position eases toward its target (~1/10th per 60 Hz tick) for silky smooth tracking.
const POSITION_EASE: i32 = 420;

/// How fast the look-ahead *direction* eases toward the direction of travel (~1/8th per 60 Hz tick).
/// Silky smooth damping prevents jarring 18-pixel swings on corner entry.
const LEAD_DIR_EASE: i32 = 512;

/// Speed below which the direction of travel is noise rather than intent, in raw
/// fixed-point units. ~18 world units/second, which is walking pace.
///
/// Under it the velocity vector points at whatever the tyres last did -- a wall
/// scrape, a steering correction, the creep of a wheel off line -- not at where
/// the car is going. Below this the lead direction is held instead of chased.
const LEAD_DIR_HOLD_SPEED: i32 = 1_200;

// ---------------------------------------------------------------------------
// Mode 7 chase view
// ---------------------------------------------------------------------------
//
// The ground plane is rotated so the car's heading points up the screen and
// projected with a pinhole camera behind the car: `y = HORIZON + K / z`,
// `x = 160 + right * FOCAL / z`, with `z = forward + M7_Z0`. At the car
// (forward ~ 0) the scale is FOCAL / Z0 = 1 pixel per world unit, so sprites
// keep their top-down size there and shrink with distance.

/// Screen row of the horizon.
pub const M7_HORIZON: i32 = 60;
/// Depth of the camera pivot behind the focus point, in world units.
pub const M7_Z0: i32 = 120;
/// Focal length in pixels. `FOCAL == Z0` makes the scale 1.0 at the pivot.
const M7_FOCAL: i32 = 120;
/// Camera height times focal length: the ground row at depth z is `HORIZON + K / z`.
const M7_K: i32 = 15_600;
/// Nearest depth projected. Clamping here keeps the divide bounded.
pub const M7_Z_NEAR: i32 = 70;
/// Farthest depth that is drawn. Bounds the number of streamed tiles.
pub const M7_Z_FAR: i32 = 240;
/// The heading eases by 1/this of the remaining angle each tick.
const M7_HEADING_EASE_DIV: i32 = 6;
/// Position ease in chase mode. Stiffer than top-down because there is no
/// look-ahead to hide the lag: the car must stay near its screen anchor.
const M7_POSITION_EASE: i32 = 1_600;

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
    /// True for the Mode 7 behind-the-car view; false for top-down views.
    pub mode7: bool,
    /// True for the rotating top-down view where the stage rotates under the car.
    pub rotating: bool,
    /// Smoothed view heading in BAMS (4096 = 360 degrees), 0 = up the screen.
    pub heading: u16,
    /// Cached `sin(heading)` / `cos(heading)` as raw Q20.12.
    hsin: i32,
    hcos: i32,
}

/// The direction the look-ahead starts at: up the screen, so the car renders
/// below centre rather than dead on it.
const LEAD_DIR_INITIAL: Vec2 = Vec2 {
    x: Fixed::ZERO,
    y: Fixed(-FP_ONE),
};

impl Default for Camera {
    fn default() -> Self {
        Camera::new(Vec2::ZERO)
    }
}

impl Camera {
    pub fn new(initial_pos: Vec2) -> Self {
        Camera {
            pos: initial_pos,
            zoom: Fixed::from_raw(ZOOM_REST),
            lead_dir: LEAD_DIR_INITIAL,
            mode7: false,
            rotating: false,
            heading: 0,
            hsin: 0,
            hcos: FP_ONE,
        }
    }

    /// A camera in the Mode 7 chase view, starting aligned to `heading`.
    pub fn new_chase(initial_pos: Vec2, heading: u16) -> Self {
        let mut c = Camera::new(initial_pos);
        c.mode7 = true;
        c.set_heading(heading);
        c
    }

    /// A camera in the top-down rotating view, starting aligned to `heading`.
    pub fn new_rotating(initial_pos: Vec2, heading: u16) -> Self {
        let mut c = Camera::new(initial_pos);
        c.rotating = true;
        c.set_heading(heading);
        c
    }

    /// Snaps the view heading (BAMS) and refreshes the cached trig.
    pub fn set_heading(&mut self, heading: u16) {
        self.heading = heading & 0x0FFF;
        self.hsin = sin(self.heading).raw();
        self.hcos = cos(self.heading).raw();
    }

    /// Eases the view heading toward `target` (BAMS) the short way round.
    pub fn follow_heading(&mut self, target: u16) {
        let mut diff = (target & 0x0FFF) as i32 - self.heading as i32;
        if diff > 2048 {
            diff -= 4096;
        } else if diff < -2048 {
            diff += 4096;
        }
        // Always make progress on a small residual so it cannot stall.
        let mut step = diff / M7_HEADING_EASE_DIV;
        if step == 0 {
            step = diff.signum();
        }
        let h = (self.heading as i32 + step) & 0x0FFF;
        self.set_heading(h as u16);
    }

    /// View heading sine in Q20.12 (FP_ONE = 4096).
    #[inline]
    pub fn hsin(&self) -> i32 {
        self.hsin
    }

    /// View heading cosine in Q20.12 (FP_ONE = 4096).
    #[inline]
    pub fn hcos(&self) -> i32 {
        self.hcos
    }

    /// Camera-space `(right, depth)` of a world point, both in Q4 world units
    /// (1/16 unit). Depth includes the pivot offset, so it is the `z` of the
    /// projection and is not clamped. Offsets are clamped so the `i32`
    /// products below cannot overflow.
    #[inline]
    pub fn view_rq4_zq4(&self, world_pos: Vec2) -> (i32, i32) {
        let dx = ((world_pos.x.raw() - self.pos.x.raw()) >> 8).clamp(-32_000, 32_000);
        let dy = ((world_pos.y.raw() - self.pos.y.raw()) >> 8).clamp(-32_000, 32_000);
        self.view_from_offset_q4(dx, dy)
    }

    /// As [`Camera::view_rq4_zq4`] but from an offset already in Q4 units.
    #[inline]
    pub fn view_from_offset_q4(&self, dx: i32, dy: i32) -> (i32, i32) {
        // forward = dx*sin(h) - dy*cos(h); right = dx*cos(h) + dy*sin(h)
        let fwd = (dx * self.hsin - dy * self.hcos) >> 12;
        let right = (dx * self.hcos + dy * self.hsin) >> 12;
        (right, fwd + (M7_Z0 << 4))
    }

    /// Projects a camera-space `(right, depth)` pair (Q4) to a screen pixel.
    #[inline]
    pub fn project_rz(right_q4: i32, z_q4: i32) -> (i16, i16) {
        let z = z_q4.max(M7_Z_NEAR << 4);
        let sy = M7_HORIZON + (M7_K * 16) / z;
        let sx = SCREEN_W as i32 / 2 + (right_q4 * M7_FOCAL) / z;
        (clamp_i16(sx.clamp(-4000, 4000)), clamp_i16(sy.min(4000)))
    }

    /// Perspective scale at a world point as Q12 (4096 = 1 pixel per unit).
    /// Returns 0 for points behind the near plane or beyond the far plane.
    #[inline]
    pub fn m7_scale_q12(&self, world_pos: Vec2) -> i32 {
        let (_, z_q4) = self.view_rq4_zq4(world_pos);
        if !((M7_Z_NEAR << 4)..=(M7_Z_FAR << 4)).contains(&z_q4) {
            return 0;
        }
        // FOCAL * 16 / z_q4 is pixels per unit; scale to Q12.
        (M7_FOCAL * 16 * 4096) / z_q4
    }

    /// Half-extents of the visible world rectangle, in world units, taking the
    /// current zoom into account.
    ///
    /// The renderer needs this to work out which tiles can possibly be on
    /// screen; anything outside this rectangle cannot be drawn.
    pub fn visible_half_extents(&self) -> (i32, i32) {
        // `zoom` is a plain ratio, so the world extent grows as it shrinks.
        let z = self.zoom.raw().max(1);
        if self.rotating {
            // For a rotating 2D viewport, diagonal half-screen radius is sqrt(160^2 + 120^2) = 200 pixels.
            // With safety margin for rounding, 205 pixels covers all rotations.
            let rad = div_ceil(205 * FP_ONE, z);
            return (rad, rad);
        }
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
        if self.mode7 {
            let (right, z_q4) = self.view_rq4_zq4(world_pos);
            if !((M7_Z_NEAR << 4)..=(M7_Z_FAR << 4)).contains(&z_q4) {
                return M7_OFFSCREEN;
            }
            return Self::project_rz(right, z_q4);
        }
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
        let z = self.zoom.raw() as i64;
        let mut dx_raw = (world_pos.x - self.pos.x).raw() as i64;
        let mut dy_raw = (world_pos.y - self.pos.y).raw() as i64;

        if self.rotating {
            // Affine rotation by camera heading so car faces up the screen:
            // screen right = dx*cos(h) + dy*sin(h)
            // screen forward (up) = dx*sin(h) - dy*cos(h) => screen y offset is -forward
            let right = (dx_raw * self.hcos as i64 + dy_raw * self.hsin as i64) >> FP_SHIFT;
            let fwd = (dx_raw * self.hsin as i64 - dy_raw * self.hcos as i64) >> FP_SHIFT;
            dx_raw = right;
            dy_raw = -fwd;
        }

        let dx = dx_raw * z;
        let dy = dy_raw * z;
        // Symmetric half-pixel rounding before the 24-bit shift eliminates
        // directional subpixel jitter and truncation asymmetry.
        const HALF_PX: i64 = 1 << (FP_SHIFT + FP_SHIFT - 1);
        let rel_x = ((dx + HALF_PX) >> (FP_SHIFT + FP_SHIFT)) as i32;
        let rel_y = ((dy + HALF_PX) >> (FP_SHIFT + FP_SHIFT)) as i32;
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
        if self.mode7 {
            // Chase view: sit on the car (it is the pivot of the projection)
            // and let the perspective supply the look-ahead.
            let diff_x = target_pos.x - self.pos.x;
            let diff_y = target_pos.y - self.pos.y;
            self.pos.x = (self.pos.x + diff_x * Fixed::from_raw(M7_POSITION_EASE))
                .clamp(Fixed::ZERO, Fixed::from_int(world_w.max(0)));
            self.pos.y = (self.pos.y + diff_y * Fixed::from_raw(M7_POSITION_EASE))
                .clamp(Fixed::ZERO, Fixed::from_int(world_h.max(0)));
            let _ = (target_velocity, speed);
            return;
        }

        if self.rotating {
            // Top-down rotating view: camera stays centered on the vehicle
            // with lookahead shifted along the vehicle's forward vector.
            let speed_ratio = clamp_i32(
                (speed.raw() as i64 * FP_ONE as i64) / ZOOM_FULL_SPEED as i64,
                0,
                FP_ONE,
            );
            let lead = LOOKAHEAD_REST + ((LOOKAHEAD_FAST - LOOKAHEAD_REST) * speed_ratio) / FP_ONE;
            // Lead along car's forward heading vector: sin(h) in X, -cos(h) in Y
            let lead_x = (self.hsin * lead) >> FP_SHIFT;
            let lead_y = (-self.hcos * lead) >> FP_SHIFT;
            let desired_pos = Vec2 {
                x: target_pos.x + Fixed::from_raw(lead_x << FP_SHIFT),
                y: target_pos.y + Fixed::from_raw(lead_y << FP_SHIFT),
            };

            let diff_x = desired_pos.x - self.pos.x;
            let diff_y = desired_pos.y - self.pos.y;
            self.pos.x = self.pos.x + diff_x * Fixed::from_raw(POSITION_EASE);
            self.pos.y = self.pos.y + diff_y * Fixed::from_raw(POSITION_EASE);

            let target_zoom = ZOOM_REST - ((ZOOM_REST - ZOOM_FAST) * speed_ratio) / FP_ONE;
            let zoom_diff = target_zoom - self.zoom.raw();
            self.zoom = Fixed::from_raw(self.zoom.raw() + (zoom_diff * ZOOM_EASE) / FP_ONE);

            self.clamp_to_bounds(world_w, world_h);
            return;
        }
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

    /// Updates the camera as a dynamic **Trackside Broadcast TV Camera** (`NO1-027`).
    ///
    /// Selects the nearest route/checkpoint vantage point ahead of `target_pos`,
    /// stands slightly off the racing line, and pans in Mode 7 perspective as the
    /// vehicle approaches and sweeps past the camera pod.
    pub fn update_broadcast_tv(
        &mut self,
        track: &arduracer_core::TrackDef,
        target_pos: Vec2,
        target_heading: u16,
    ) {
        self.mode7 = true;
        self.rotating = false;

        let n = if !track.route.is_empty() {
            track.route.len()
        } else {
            track.route_len()
        };

        let mut vantage = target_pos;
        if n > 0 {
            let node_at = |idx: usize| -> Vec2 {
                if !track.route.is_empty() {
                    track.route[idx % n]
                } else {
                    arduracer_core::TrackDef::gate_centre(&track.route_node(idx % n))
                }
            };
            let mut best_idx = 0usize;
            let mut best_d2 = i64::MAX;
            for i in 0..n {
                let p = node_at(i);
                let dx = (p.x.raw() - target_pos.x.raw()) >> 8;
                let dy = (p.y.raw() - target_pos.y.raw()) >> 8;
                let d2 = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                if d2 < best_d2 {
                    best_d2 = d2;
                    best_idx = i;
                }
            }
            // Pick the node 1 step ahead of the closest node as the broadcast camera stand
            let cam_node = node_at((best_idx + 1) % n);
            let next_node = node_at((best_idx + 2) % n);
            let seg_h = arduracer_core::heading_towards(cam_node, next_node);
            // Offset 24 units to the outside of the track segment
            let side_x = (cos(seg_h).raw() * 24) >> FP_SHIFT;
            let side_y = (sin(seg_h).raw() * 24) >> FP_SHIFT;
            vantage = Vec2::new(
                cam_node.x + Fixed::from_int(side_x),
                cam_node.y + Fixed::from_int(side_y),
            );
        }

        let aim_heading = if (target_pos - vantage).length().to_int() < 8 {
            target_heading
        } else {
            arduracer_core::heading_towards(vantage, target_pos)
        };
        self.set_heading(aim_heading);

        // Keep `target_pos` inside the Mode 7 frustum depth [85..185] while panning:
        // Since `z = fwd + M7_Z0 (120)`, placing `self.pos` at `target_pos - aim_dir * (depth - 120)`
        // puts `target_pos` at exact camera depth `depth`.
        let raw_dist = (target_pos - vantage).length().to_int().clamp(85, 185);
        let fwd_offset = raw_dist - M7_Z0;
        let world_w = track.world_width().max(0);
        let world_h = track.world_height().max(0);
        let pivot_x = target_pos.x - Fixed::from_raw(self.hsin * fwd_offset);
        let pivot_y = target_pos.y + Fixed::from_raw(self.hcos * fwd_offset);
        self.pos.x = pivot_x.clamp(Fixed::ZERO, Fixed::from_int(world_w));
        self.pos.y = pivot_y.clamp(Fixed::ZERO, Fixed::from_int(world_h));
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
