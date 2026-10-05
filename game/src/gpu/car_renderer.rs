//! Rotated Polygon Vehicle and Aerodynamic Aero Renderer for PlayStation 1.
//!
//! Renders high-speed race cars with true 64-directional hardware polygon rotation:
//! - Rotated aerodynamic drop shadow
//! - 4 corner tire/wheel pods with steering angle
//! - Wide-body silhouette with tapered nose and flared rear haunches
//! - Twin contrasting racing stripes
//! - Tinted cockpit canopy / windshield glass
//! - Elevated rear aero spoiler wing
//! - Halogen headlights and glowing crimson taillights
//! - Twin pulsing nitro boost flame plumes

use crate::gpu::camera::Camera;
use crate::gpu::car_geometry::{make_quad, make_rect_at, transform_pt, Rot};
use arduracer_core::VehicleState;
use psx_gpu as gpu;
use psx_gpu::material::BlendMode;

/// How the car is composited. A ghost is a memory of a lap, so it blends with
/// the road instead of covering it (TASK-1205).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CarLayer {
    Solid,
    Ghost,
}

impl CarLayer {
    fn of(is_ghost: bool) -> Self {
        if is_ghost {
            CarLayer::Ghost
        } else {
            CarLayer::Solid
        }
    }

    /// Blends the ghost into the road; solid cars overwrite.
    pub const fn blend(self) -> BlendMode {
        match self {
            CarLayer::Solid => BlendMode::Opaque,
            CarLayer::Ghost => BlendMode::Average,
        }
    }
}

#[inline(always)]
fn draw_line(p0: (i16, i16), p1: (i16, i16), col: (u8, u8, u8), layer: CarLayer) {
    if layer == CarLayer::Ghost {
        gpu::draw_line_mono_blended(p0, p1, col, layer.blend());
    } else {
        gpu::draw_line_mono(p0.0, p0.1, p1.0, p1.1, col.0, col.1, col.2);
    }
}

#[inline(always)]
fn draw_quad(verts: [(i16, i16); 4], r: u8, g: u8, b: u8, layer: CarLayer) {
    gpu::draw_quad_flat_blended(verts, r, g, b, layer.blend());
}

/// Renders a vehicle with hardware-rotated polygons, drop shadow, and visual effects.
pub fn render_car(
    car: &VehicleState,
    camera: &Camera,
    draw_y: i16,
    is_ghost: bool,
    primary_color: (u8, u8, u8),
) {
    let (cx, cy) = camera.world_to_screen(car.position, draw_y);

    // Skip if offscreen
    if !(-40..=360).contains(&cx) || !(-40..=280).contains(&cy) {
        return;
    }

    let layer = CarLayer::of(is_ghost);
    let rot = Rot::from_bams(car.visual_angle);

    // 1. Dynamic Rotated Drop Shadow (projected +3, +3)
    if !is_ghost {
        let shadow = make_quad(cx + 3, cy + 3, 7, 8, 12, 12, rot);
        draw_quad(shadow, 12, 14, 18, layer);
    }

    // 2. Tires / Wheel Pods (4 Black rubber rectangles at the 4 corners)
    if !is_ghost {
        // Rear wheels
        let r_left = make_rect_at(cx, cy, -7, -6, 2, 4, rot);
        let r_right = make_rect_at(cx, cy, 7, -6, 2, 4, rot);
        draw_quad(r_left, 18, 18, 22, layer);
        draw_quad(r_right, 18, 18, 22, layer);

        // Front wheels
        let f_left = make_rect_at(cx, cy, -7, 7, 2, 4, rot);
        let f_right = make_rect_at(cx, cy, 7, 7, 2, 4, rot);
        draw_quad(f_left, 18, 18, 22, layer);
        draw_quad(f_right, 18, 18, 22, layer);
    }

    // 3. Main Aerodynamic Car Body
    let (r, g, b) = if is_ghost {
        (80, 160, 240) // Translucent glowing cyan ghost
    } else {
        primary_color
    };

    // Body: Tapered front hood (w=6) widening to muscular rear haunches (w=7), length ±12
    let body = make_quad(cx, cy, 6, 7, 12, 12, rot);
    draw_quad(body, r, g, b, layer);

    // 4. Dual Center Racing Stripes (Contrasting White or Golden highlight)
    if !is_ghost {
        let stripe_color = if r > 200 && g < 100 {
            (255, 255, 255) // White stripes on red/crimson cars
        } else {
            (255, 220, 40) // Neon gold stripes on blue/green/dark cars
        };
        let stripe_l = make_quad(cx, cy, 1, 1, 12, 12, rot);
        draw_quad(
            stripe_l,
            stripe_color.0,
            stripe_color.1,
            stripe_color.2,
            layer,
        );
    }

    // 5. Cockpit Canopy / Tinted Windshield Glass
    let glass_color = if is_ghost {
        (160, 220, 255)
    } else {
        (25, 38, 55) // Deep polarized reflective glass
    };
    // `make_quad` negates `l_rear` itself, so this must be positive. Passing a
    // negative length cancelled the negation and put all four vertices on the
    // same row, giving the canopy zero area at every heading (TASK-1203).
    let glass = make_quad(cx, cy, 4, 5, 4, 7, rot);
    draw_quad(glass, glass_color.0, glass_color.1, glass_color.2, layer);

    // Windshield front glare reflection line
    if !is_ghost {
        let p_glare_l = transform_pt(cx, cy, -3, 3, rot.cos_raw, rot.sin_raw);
        let p_glare_r = transform_pt(cx, cy, 3, 3, rot.cos_raw, rot.sin_raw);
        draw_line(p_glare_l, p_glare_r, (120, 160, 200), layer);
    }

    // 6. Rear Aero Wing / Downforce Spoiler
    if !is_ghost {
        let wing = make_rect_at(cx, cy, 0, -11, 8, 2, rot);
        let wing_r = (r as u16 * 180 / 255) as u8;
        let wing_g = (g as u16 * 180 / 255) as u8;
        let wing_b = (b as u16 * 180 / 255) as u8;
        draw_quad(wing, wing_r, wing_g, wing_b, layer);
    }

    // 7. Headlights (Front nose corners: bright halogen amber-white)
    let hl_l = transform_pt(cx, cy, -4, 11, rot.cos_raw, rot.sin_raw);
    let hl_r = transform_pt(cx, cy, 4, 11, rot.cos_raw, rot.sin_raw);
    let hl_l2 = transform_pt(cx, cy, -5, 10, rot.cos_raw, rot.sin_raw);
    let hl_r2 = transform_pt(cx, cy, 5, 10, rot.cos_raw, rot.sin_raw);
    draw_line(hl_l, hl_l2, (255, 250, 190), layer);
    draw_line(hl_r, hl_r2, (255, 250, 190), layer);

    // 8. Taillights (Rear corners: bright crimson red)
    let tl_l = transform_pt(cx, cy, -5, -12, rot.cos_raw, rot.sin_raw);
    let tl_r = transform_pt(cx, cy, 5, -12, rot.cos_raw, rot.sin_raw);
    let tl_l2 = transform_pt(cx, cy, -6, -11, rot.cos_raw, rot.sin_raw);
    let tl_r2 = transform_pt(cx, cy, 6, -11, rot.cos_raw, rot.sin_raw);
    draw_line(tl_l, tl_l2, (255, 30, 30), layer);
    draw_line(tl_r, tl_r2, (255, 30, 30), layer);

    // 9. Nitro Boost Exhaust Flame Jet
    if car.boost_ticks > 0 {
        // Twin animated flame cones behind exhausts (-3, -12) and (3, -12)
        let flame_len = 8 + ((car.boost_ticks % 4) as i32 * 2);
        let flame_tip_l = transform_pt(cx, cy, -3, -12 - flame_len, rot.cos_raw, rot.sin_raw);
        let flame_base_l1 = transform_pt(cx, cy, -4, -12, rot.cos_raw, rot.sin_raw);
        let flame_base_l2 = transform_pt(cx, cy, -2, -12, rot.cos_raw, rot.sin_raw);
        gpu::draw_tri_flat([flame_tip_l, flame_base_l1, flame_base_l2], 0, 220, 255);

        let flame_tip_r = transform_pt(cx, cy, 3, -12 - flame_len, rot.cos_raw, rot.sin_raw);
        let flame_base_r1 = transform_pt(cx, cy, 2, -12, rot.cos_raw, rot.sin_raw);
        let flame_base_r2 = transform_pt(cx, cy, 4, -12, rot.cos_raw, rot.sin_raw);
        gpu::draw_tri_flat([flame_tip_r, flame_base_r1, flame_base_r2], 255, 140, 20);
    }
}
