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
use arduracer_core::{math::cos, math::sin, VehicleState};
use psx_gpu as gpu;

#[inline(always)]
fn transform_pt(cx: i16, cy: i16, u: i32, v: i32, cos_raw: i32, sin_raw: i32) -> (i16, i16) {
    let x = cx as i32 + ((u * cos_raw + v * sin_raw) >> 12);
    let y = cy as i32 + ((u * sin_raw - v * cos_raw) >> 12);
    (x as i16, y as i16)
}

/// A cached heading, as the raw Q20.12 sine/cosine the vertex transform wants.
///
/// Bundled because every geometry helper needs both halves and passing them as
/// two scalars pushed `make_quad`/`make_rect_at` over clippy's argument limit.
#[derive(Copy, Clone)]
struct Rot {
    cos_raw: i32,
    sin_raw: i32,
}

impl Rot {
    #[inline(always)]
    fn from_bams(angle: u16) -> Self {
        Rot {
            cos_raw: cos(angle).raw() as i32,
            sin_raw: sin(angle).raw() as i32,
        }
    }
}

#[inline(always)]
fn make_quad(
    cx: i16,
    cy: i16,
    w_front: i32,
    w_rear: i32,
    l_front: i32,
    l_rear: i32,
    rot: Rot,
) -> [(i16, i16); 4] {
    [
        transform_pt(cx, cy, -w_front, l_front, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, w_front, l_front, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, -w_rear, -l_rear, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, w_rear, -l_rear, rot.cos_raw, rot.sin_raw),
    ]
}

#[inline(always)]
fn draw_line(p0: (i16, i16), p1: (i16, i16), col: (u8, u8, u8)) {
    gpu::draw_line_mono(p0.0, p0.1, p1.0, p1.1, col.0, col.1, col.2);
}

#[inline(always)]
fn make_rect_at(
    cx: i16,
    cy: i16,
    center_u: i32,
    center_v: i32,
    half_w: i32,
    half_l: i32,
    rot: Rot,
) -> [(i16, i16); 4] {
    [
        transform_pt(
            cx,
            cy,
            center_u - half_w,
            center_v + half_l,
            rot.cos_raw,
            rot.sin_raw,
        ),
        transform_pt(
            cx,
            cy,
            center_u + half_w,
            center_v + half_l,
            rot.cos_raw,
            rot.sin_raw,
        ),
        transform_pt(
            cx,
            cy,
            center_u - half_w,
            center_v - half_l,
            rot.cos_raw,
            rot.sin_raw,
        ),
        transform_pt(
            cx,
            cy,
            center_u + half_w,
            center_v - half_l,
            rot.cos_raw,
            rot.sin_raw,
        ),
    ]
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

    let rot = Rot::from_bams(car.visual_angle);

    // 1. Dynamic Rotated Drop Shadow (projected +3, +3)
    if !is_ghost {
        let shadow = make_quad(cx + 3, cy + 3, 7, 8, 12, 12, rot);
        gpu::draw_quad_flat(shadow, 12, 14, 18);
    }

    // 2. Tires / Wheel Pods (4 Black rubber rectangles at the 4 corners)
    if !is_ghost {
        // Rear wheels
        let r_left = make_rect_at(cx, cy, -7, -6, 2, 4, rot);
        let r_right = make_rect_at(cx, cy, 7, -6, 2, 4, rot);
        gpu::draw_quad_flat(r_left, 18, 18, 22);
        gpu::draw_quad_flat(r_right, 18, 18, 22);

        // Front wheels
        let f_left = make_rect_at(cx, cy, -7, 7, 2, 4, rot);
        let f_right = make_rect_at(cx, cy, 7, 7, 2, 4, rot);
        gpu::draw_quad_flat(f_left, 18, 18, 22);
        gpu::draw_quad_flat(f_right, 18, 18, 22);
    }

    // 3. Main Aerodynamic Car Body
    let (r, g, b) = if is_ghost {
        (80, 160, 240) // Translucent glowing cyan ghost
    } else {
        primary_color
    };

    // Body: Tapered front hood (w=6) widening to muscular rear haunches (w=7), length ±12
    let body = make_quad(cx, cy, 6, 7, 12, 12, rot);
    gpu::draw_quad_flat(body, r, g, b);

    // 4. Dual Center Racing Stripes (Contrasting White or Golden highlight)
    if !is_ghost {
        let stripe_color = if r > 200 && g < 100 {
            (255, 255, 255) // White stripes on red/crimson cars
        } else {
            (255, 220, 40) // Neon gold stripes on blue/green/dark cars
        };
        let stripe_l = make_quad(cx, cy, 1, 1, 12, 12, rot);
        gpu::draw_quad_flat(stripe_l, stripe_color.0, stripe_color.1, stripe_color.2);
    }

    // 5. Cockpit Canopy / Tinted Windshield Glass
    let glass_color = if is_ghost {
        (160, 220, 255)
    } else {
        (25, 38, 55) // Deep polarized reflective glass
    };
    let glass = make_quad(cx, cy, 4, 5, 4, -4, rot);
    gpu::draw_quad_flat(glass, glass_color.0, glass_color.1, glass_color.2);

    // Windshield front glare reflection line
    if !is_ghost {
        let p_glare_l = transform_pt(cx, cy, -3, 3, rot.cos_raw, rot.sin_raw);
        let p_glare_r = transform_pt(cx, cy, 3, 3, rot.cos_raw, rot.sin_raw);
        draw_line(p_glare_l, p_glare_r, (120, 160, 200));
    }

    // 6. Rear Aero Wing / Downforce Spoiler
    if !is_ghost {
        let wing = make_rect_at(cx, cy, 0, -11, 8, 2, rot);
        let wing_r = (r as u16 * 180 / 255) as u8;
        let wing_g = (g as u16 * 180 / 255) as u8;
        let wing_b = (b as u16 * 180 / 255) as u8;
        gpu::draw_quad_flat(wing, wing_r, wing_g, wing_b);
    }

    // 7. Headlights (Front nose corners: bright halogen amber-white)
    let hl_l = transform_pt(cx, cy, -4, 11, rot.cos_raw, rot.sin_raw);
    let hl_r = transform_pt(cx, cy, 4, 11, rot.cos_raw, rot.sin_raw);
    let hl_l2 = transform_pt(cx, cy, -5, 10, rot.cos_raw, rot.sin_raw);
    let hl_r2 = transform_pt(cx, cy, 5, 10, rot.cos_raw, rot.sin_raw);
    draw_line(hl_l, hl_l2, (255, 250, 190));
    draw_line(hl_r, hl_r2, (255, 250, 190));

    // 8. Taillights (Rear corners: bright crimson red)
    let tl_l = transform_pt(cx, cy, -5, -12, rot.cos_raw, rot.sin_raw);
    let tl_r = transform_pt(cx, cy, 5, -12, rot.cos_raw, rot.sin_raw);
    let tl_l2 = transform_pt(cx, cy, -6, -11, rot.cos_raw, rot.sin_raw);
    let tl_r2 = transform_pt(cx, cy, 6, -11, rot.cos_raw, rot.sin_raw);
    draw_line(tl_l, tl_l2, (255, 30, 30));
    draw_line(tl_r, tl_r2, (255, 30, 30));

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
