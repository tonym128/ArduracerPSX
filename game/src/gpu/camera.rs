//! Dynamic 2.5D Overhead Camera Controller.
//!
//! Follows the player vehicle smoothly with:
//! - Speed-dependent zoom (widers field of view as vehicle approaches top speed)
//! - Forward lookahead along the vehicle's heading vector
//! - Smooth exponential smoothing (lerp) eliminating harsh jitter

use arduracer_core::{Fixed, Vec2, FP_ONE};

pub const SCREEN_W: i16 = 320;
pub const SCREEN_H: i16 = 240;

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
            zoom: Fixed::ONE,
        }
    }
}

impl Camera {
    pub fn new(initial_pos: Vec2) -> Self {
        Camera {
            pos: initial_pos,
            zoom: Fixed::ONE,
        }
    }

    /// Updates camera position to track vehicle with forward lookahead.
    pub fn update(&mut self, target_pos: Vec2, target_velocity: Vec2, speed: Fixed) {
        // Lookahead vector: lead camera in the direction of motion
        let lookahead = target_velocity.scale(Fixed::from_raw(1600)); // ~39% of velocity
        let desired_pos = target_pos + lookahead;

        // Smooth exponential tracking (factor 0.125 = 1/8th per 60Hz tick)
        let diff_x = desired_pos.x - self.pos.x;
        let diff_y = desired_pos.y - self.pos.y;
        self.pos.x = self.pos.x + diff_x * Fixed::from_raw(512);
        self.pos.y = self.pos.y + diff_y * Fixed::from_raw(512);

        // Dynamic zoom based on speed: zoom widens at high speeds for greater anticipation
        let speed_ratio = (speed.raw() as i64 * 4096) / 14000;
        let target_zoom = Fixed::from_raw((4096 - (speed_ratio * 400 / 4096)) as i32); // up to ~10% zoom out
        let zoom_diff = target_zoom - self.zoom;
        self.zoom = self.zoom + zoom_diff * Fixed::from_raw(256);
    }

    /// Transforms a world position into screen coordinates (0..320, 0..240).
    pub fn world_to_screen(&self, world_pos: Vec2, draw_offset_y: i16) -> (i16, i16) {
        let rel_x = (world_pos.x - self.pos.x).raw() / FP_ONE;
        let rel_y = (world_pos.y - self.pos.y).raw() / FP_ONE;

        let screen_x = (SCREEN_W / 2) + (rel_x as i16);
        let screen_y = (SCREEN_H / 2) + (rel_y as i16) + draw_offset_y;
        (screen_x, screen_y)
    }
}
