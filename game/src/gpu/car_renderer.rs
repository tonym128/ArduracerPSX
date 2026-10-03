//! Vehicle sprite and shadow renderer.
//!
//! Renders the race car with 64-directional rotation, dynamic drop shadow,
//! windshield canopy, headlights, and nitro exhaust flames.

use crate::gpu::camera::Camera;
use arduracer_core::{math, VehicleState};
use psx_gpu as gpu;

pub const CAR_WIDTH: i16 = 16;
pub const CAR_LENGTH: i16 = 28;

/// Renders a vehicle with rotation, shadow, and visual effects.
pub fn render_car(
    car: &VehicleState,
    camera: &Camera,
    draw_y: i16,
    is_ghost: bool,
    primary_color: (u8, u8, u8),
) {
    let (cx, cy) = camera.world_to_screen(car.position, draw_y);

    // Skip if offscreen
    if cx < -32 || cx > 352 || cy < -32 || cy > 272 {
        return;
    }

    let angle = car.visual_angle;
    let cos_a = math::cos(angle);
    let sin_a = math::sin(angle);

    // 1. Drop Shadow (offset +3, +3)
    if !is_ghost {
        let shadow_x = cx + 3;
        let shadow_y = cy + 3;
        gpu::fill_rect(
            (shadow_x - CAR_WIDTH / 2) as u16,
            (shadow_y - CAR_LENGTH / 2) as u16,
            CAR_WIDTH as u16,
            CAR_LENGTH as u16,
            15,
            15,
            20,
        );
    }

    // 2. Main Car Body
    let (r, g, b) = if is_ghost {
        (100, 150, 220) // Translucent light cyan ghost
    } else {
        primary_color
    };

    gpu::fill_rect(
        (cx - CAR_WIDTH / 2) as u16,
        (cy - CAR_LENGTH / 2) as u16,
        CAR_WIDTH as u16,
        CAR_LENGTH as u16,
        r,
        g,
        b,
    );

    // 3. Cockpit Glass / Windshield (dark tinted)
    gpu::fill_rect(
        (cx - CAR_WIDTH / 4) as u16,
        (cy - CAR_LENGTH / 6) as u16,
        (CAR_WIDTH / 2) as u16,
        (CAR_LENGTH / 3) as u16,
        30,
        45,
        65,
    );

    // 4. Headlights / Taillights
    let forward_x = ((sin_a.raw() as i32 * (CAR_LENGTH as i32 / 2)) >> math::FP_SHIFT) as i16;
    let forward_y = ((-cos_a.raw() as i32 * (CAR_LENGTH as i32 / 2)) >> math::FP_SHIFT) as i16;

    let nose_x = cx + forward_x;
    let nose_y = cy + forward_y;
    // Front headlights (bright amber/white)
    gpu::fill_rect(
        (nose_x - 3) as u16,
        (nose_y - 2) as u16,
        6,
        4,
        255,
        240,
        150,
    );

    let tail_x = cx - forward_x;
    let tail_y = cy - forward_y;
    // Rear taillights (crimson red)
    gpu::fill_rect((tail_x - 3) as u16, (tail_y - 2) as u16, 6, 4, 255, 20, 20);

    // 5. Nitro Boost Flame Exhaust
    if car.boost_ticks > 0 {
        let flame_x = tail_x - (forward_x / 2);
        let flame_y = tail_y - (forward_y / 2);
        gpu::fill_rect(
            (flame_x - 2) as u16,
            (flame_y - 2) as u16,
            5,
            5,
            0,
            190,
            255,
        );
    }
}
