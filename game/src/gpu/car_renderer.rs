//! 3D Gouraud-Shaded Low-Poly Mesh & Multi-Part Vehicle Renderer (`NO1-030`, `NO1-033`).
//!
//! Renders high-speed race cars in both Mode 7 3D perspective (using hardware GTE `RTPS`
//! projection and Gouraud shading) and 64-directional Overhead/Rotating views:
//! - Rotated aerodynamic drop shadow
//! - 4 corner tire/wheel pods with live front-wheel steering deflection (`steered_wheel_rot`)
//! - Sculpted 3D wedge nose, raised sidepods, and Gouraud-shaded upper deck
//! - True twin contrasting racing stripes (`make_twin_stripes`)
//! - Raised 3D tinted cockpit canopy with sky-reflection gradient
//! - Elevated 3D rear aero spoiler wing with vertical endplates
//! - Dynamic suspension pitch & cornering roll (`body_roll_pitch_px`)
//! - Additive (`BlendMode::Add`) dual headlight cones in tunnels and night stages (`NO1-033`)
//! - Illuminated crimson brake lights + additive glow under braking/drifting
//! - Twin pulsing Nitro (cyan) and Reactor Super Boost (crimson-gold) flame plumes

use crate::gpu::camera::{Camera, M7_OFFSCREEN};
use crate::gpu::car_geometry::{
    body_roll_pitch_px, make_quad, make_rect_at, make_twin_stripes, steered_wheel_rot,
    transform_pt, Rot,
};
use crate::gpu::gte_proj::{load_m7_camera_gte, project_world_3d_gte};
use arduracer_core::{
    math::{cos, sin},
    Fixed, Vec2, VehicleState,
};
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
fn scale_rgb(c: (u8, u8, u8), num: u16) -> (u8, u8, u8) {
    (
        ((c.0 as u16 * num) / 256).min(255) as u8,
        ((c.1 as u16 * num) / 256).min(255) as u8,
        ((c.2 as u16 * num) / 256).min(255) as u8,
    )
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

#[inline(always)]
fn draw_quad_gouraud(verts: [(i16, i16); 4], colors: [(u8, u8, u8); 4], layer: CarLayer) {
    let blend = layer.blend();
    gpu::draw_tri_gouraud_blended(
        [verts[0], verts[1], verts[2]],
        [colors[0], colors[1], colors[2]],
        blend,
    );
    gpu::draw_tri_gouraud_blended(
        [verts[1], verts[2], verts[3]],
        [colors[1], colors[2], colors[3]],
        blend,
    );
}

/// Transforms a local `(u, v)` vehicle offset (in world units) into world `Vec2`.
#[inline(always)]
fn car_local_to_world(pos: Vec2, cos_raw: i32, sin_raw: i32, u: i32, v: i32) -> Vec2 {
    // Heading 0 = North (-Y): forward (+v) is (+sin, -cos), right (+u) is (+cos, +sin).
    let wx = pos.x.raw() + u * cos_raw + v * sin_raw;
    let wy = pos.y.raw() + u * sin_raw - v * cos_raw;
    Vec2::new(Fixed::from_raw(wx), Fixed::from_raw(wy))
}

/// Renders additive dual headlight cones ahead of the vehicle on night stages and inside tunnels (`NO1-033`).
fn render_headlight_cones(car: &VehicleState, camera: &Camera, cx: i16, body_cy: i16, rot: Rot) {
    if camera.mode7 {
        let h_cos = cos(car.visual_angle).raw();
        let h_sin = sin(car.visual_angle).raw();
        let z_base = car.z.to_int().clamp(0, 64);

        for side in [-1i32, 1i32] {
            let u0 = side * 4;
            let p_origin = car_local_to_world(car.position, h_cos, h_sin, u0, 12);
            let p_outer = car_local_to_world(car.position, h_cos, h_sin, side * 16, 54);
            let p_inner = car_local_to_world(car.position, h_cos, h_sin, side * 2, 54);

            let s0 = camera.world_to_screen(p_origin);
            let s1 = camera.world_to_screen(p_outer);
            let s2 = camera.world_to_screen(p_inner);
            if s0 != M7_OFFSCREEN && s1 != M7_OFFSCREEN && s2 != M7_OFFSCREEN {
                let lift = (z_base / 2) as i16;
                gpu::draw_tri_gouraud_blended(
                    [(s0.0, s0.1 - lift), s1, s2],
                    [(145, 140, 100), (0, 0, 0), (0, 0, 0)],
                    BlendMode::Add,
                );
            }
        }
    } else {
        for side in [-1i32, 1i32] {
            let p0 = transform_pt(cx, body_cy, side * 4, 12, rot.cos_raw, rot.sin_raw);
            let p1 = transform_pt(cx, body_cy, side * 15, 48, rot.cos_raw, rot.sin_raw);
            let p2 = transform_pt(cx, body_cy, side * 2, 48, rot.cos_raw, rot.sin_raw);
            gpu::draw_tri_gouraud_blended(
                [p0, p1, p2],
                [(135, 130, 95), (0, 0, 0), (0, 0, 0)],
                BlendMode::Add,
            );
        }
    }
}

/// Renders a GTE-transformed, Gouraud-shaded 3D low-poly car mesh in Mode 7 chase view (`NO1-030`).
fn render_car_mode7_3d(
    car: &VehicleState,
    camera: &Camera,
    is_ghost: bool,
    primary_color: (u8, u8, u8),
) {
    load_m7_camera_gte(camera);

    let h_cos = cos(car.visual_angle).raw();
    let h_sin = sin(car.visual_angle).raw();
    let base_z = car.z.to_int().clamp(0, 64);
    let braking = car.is_drifting
        || car.is_reversing
        || (car.speed.raw() > 1200 && car.engine_rpm < 2400 && car.boost_ticks == 0);
    let (roll_u, pitch_v) = body_roll_pitch_px(
        car.speed.raw(),
        car.slide_bam() as i16,
        car.vz.raw(),
        braking,
    );

    let proj = |u: i32, v: i32, h: i32| -> Option<(i16, i16)> {
        let wpos = car_local_to_world(car.position, h_cos, h_sin, u, v);
        // Apply subtle roll/pitch to elevated vertices above the wheel plane
        let dh = if h > 1 {
            (u * roll_u) / 8 + (v * pitch_v) / 12
        } else {
            0
        };
        project_world_3d_gte(camera, wpos, base_z + h + dh).map(|(pt, _)| pt)
    };

    let Some(center_ground) = proj(0, 0, 0) else {
        return;
    };
    if !(-60..=380).contains(&center_ground.0) || !(-60..=300).contains(&center_ground.1) {
        return;
    }

    let layer = CarLayer::of(is_ghost);

    // 1. Ground Drop Shadow (projected flat at height 0)
    if !is_ghost {
        if let (Some(fl), Some(fr), Some(rl), Some(rr)) = (
            proj(-7, 12, 0),
            proj(7, 12, 0),
            proj(-8, -12, 0),
            proj(8, -12, 0),
        ) {
            draw_quad([fl, fr, rl, rr], 10, 12, 16, layer);
        }
    }

    // 2. Night / Tunnel Additive Headlight Cones (NO1-033)
    if !is_ghost && crate::gpu::tracktex::is_night_circuit() {
        render_headlight_cones(
            car,
            camera,
            center_ground.0,
            center_ground.1,
            Rot::from_bams(0),
        );
    }

    let base_col = if is_ghost {
        (
            primary_color.0 / 2 + 50,
            primary_color.1 / 2 + 60,
            primary_color.2 / 2 + 80,
        )
    } else {
        primary_color
    };
    let dark_col = scale_rgb(base_col, 145);
    let mid_col = scale_rgb(base_col, 210);
    let bright_col = (
        base_col.0.saturating_add(35),
        base_col.1.saturating_add(35),
        base_col.2.saturating_add(35),
    );

    // 3. Four 3D Corner Wheel Pods (with steered front wheels)
    if !is_ghost {
        let steer_u = ((car.slide_bam() as i16) as i32 / 12).clamp(-2, 2);
        for &(wu, wv, is_front) in &[(-7, -7, false), (7, -7, false), (-7, 7, true), (7, 7, true)] {
            let du = if is_front { steer_u } else { 0 };
            if let (Some(tl), Some(tr), Some(bl), Some(br)) = (
                proj(wu - 1 + du, wv + 3, 3),
                proj(wu + 1 + du, wv + 3, 3),
                proj(wu - 1 - du, wv - 3, 0),
                proj(wu + 1 - du, wv - 3, 0),
            ) {
                draw_quad([tl, tr, bl, br], 20, 22, 26, layer);
            }
        }
    }

    // 4. Sculpted 3D Sidepods (Left & Right vertical flanks, h = 1..4)
    if let (Some(l_ft), Some(l_rt), Some(l_fb), Some(l_rb)) = (
        proj(-6, 11, 3),
        proj(-7, -11, 4),
        proj(-6, 11, 1),
        proj(-7, -11, 1),
    ) {
        draw_quad_gouraud(
            [l_ft, l_rt, l_fb, l_rb],
            [mid_col, bright_col, dark_col, dark_col],
            layer,
        );
    }
    if let (Some(r_ft), Some(r_rt), Some(r_fb), Some(r_rb)) = (
        proj(6, 11, 3),
        proj(7, -11, 4),
        proj(6, 11, 1),
        proj(7, -11, 1),
    ) {
        draw_quad_gouraud(
            [r_ft, r_rt, r_fb, r_rb],
            [mid_col, bright_col, dark_col, dark_col],
            layer,
        );
    }

    // 5. 3D Wedge Nose & Upper Body Deck (h = 2 at nose rising to h = 4 at rear haunches)
    if let (Some(fl), Some(fr), Some(rl), Some(rr)) = (
        proj(-6, 12, 2),
        proj(6, 12, 2),
        proj(-7, -12, 4),
        proj(7, -12, 4),
    ) {
        draw_quad_gouraud(
            [fl, fr, rl, rr],
            [bright_col, bright_col, base_col, base_col],
            layer,
        );
    }

    // 6. True Twin Racing Stripes along the 3D Hood & Deck (NO1-030)
    if !is_ghost {
        let stripe_col = if base_col.0 > 200 && base_col.1 < 100 {
            (250, 250, 255)
        } else {
            (255, 220, 45)
        };
        for &su in &[-2i32, 2i32] {
            if let (Some(sfl), Some(sfr), Some(srl), Some(srr)) = (
                proj(su - 1, 11, 3),
                proj(su + 1, 11, 3),
                proj(su - 1, -11, 4),
                proj(su + 1, -11, 4),
            ) {
                draw_quad(
                    [sfl, sfr, srl, srr],
                    stripe_col.0,
                    stripe_col.1,
                    stripe_col.2,
                    layer,
                );
            }
        }
    }

    // 7. 3D Raised Cockpit Canopy (Windshield, Roof, Rear Engine Deck, Side Glass)
    let glass_top = if is_ghost {
        (180, 235, 255)
    } else {
        (75, 115, 165)
    };
    let glass_bot = if is_ghost {
        (120, 190, 240)
    } else {
        (22, 34, 52)
    };
    // Front sloped windshield (v = 5..1, h = 3..7)
    if let (Some(w_fl), Some(w_fr), Some(w_rl), Some(w_rr)) =
        (proj(-4, 5, 3), proj(4, 5, 3), proj(-3, 0, 7), proj(3, 0, 7))
    {
        draw_quad_gouraud(
            [w_fl, w_fr, w_rl, w_rr],
            [glass_bot, glass_bot, glass_top, glass_top],
            layer,
        );
    }
    // Rear sloped canopy / engine cover (v = 0..-7, h = 7..4)
    if let (Some(c_fl), Some(c_fr), Some(c_rl), Some(c_rr)) = (
        proj(-3, 0, 7),
        proj(3, 0, 7),
        proj(-5, -7, 4),
        proj(5, -7, 4),
    ) {
        draw_quad_gouraud(
            [c_fl, c_fr, c_rl, c_rr],
            [glass_top, glass_top, dark_col, dark_col],
            layer,
        );
    }

    // 8. 3D Rear Tail Fascia & Illuminated Brake Lights
    if let (Some(t_tl), Some(t_tr), Some(t_bl), Some(t_br)) = (
        proj(-7, -12, 4),
        proj(7, -12, 4),
        proj(-7, -12, 1),
        proj(7, -12, 1),
    ) {
        draw_quad_gouraud(
            [t_tl, t_tr, t_bl, t_br],
            [dark_col, dark_col, (16, 18, 24), (16, 18, 24)],
            layer,
        );
    }
    let brake_col = if braking {
        (255, 55, 45)
    } else {
        (190, 24, 28)
    };
    for &bu in &[-5i32, 5i32] {
        if let (Some(bl_tl), Some(bl_tr), Some(bl_bl), Some(bl_br)) = (
            proj(bu - 1, -12, 4),
            proj(bu + 1, -12, 4),
            proj(bu - 1, -12, 2),
            proj(bu + 1, -12, 2),
        ) {
            draw_quad(
                [bl_tl, bl_tr, bl_bl, bl_br],
                brake_col.0,
                brake_col.1,
                brake_col.2,
                layer,
            );
            if braking && !is_ghost {
                gpu::draw_quad_flat_blended(
                    [
                        (bl_tl.0 - 1, bl_tl.1 - 1),
                        (bl_tr.0 + 1, bl_tr.1 - 1),
                        (bl_bl.0 - 1, bl_bl.1 + 1),
                        (bl_br.0 + 1, bl_br.1 + 1),
                    ],
                    180,
                    30,
                    20,
                    BlendMode::Add,
                );
            }
        }
    }

    // 9. Elevated 3D Rear Aero Wing & Endplates (h = 8)
    if !is_ghost {
        let wing_hi = bright_col;
        let wing_lo = scale_rgb(base_col, 165);
        if let (Some(w_fl), Some(w_fr), Some(w_rl), Some(w_rr)) = (
            proj(-8, -9, 7),
            proj(8, -9, 7),
            proj(-8, -12, 9),
            proj(8, -12, 9),
        ) {
            draw_quad_gouraud(
                [w_fl, w_fr, w_rl, w_rr],
                [wing_lo, wing_lo, wing_hi, wing_hi],
                layer,
            );
        }
        // Wing vertical endplates at u = -8 and u = +8
        for &eu in &[-8i32, 8i32] {
            if let (Some(ep_ft), Some(ep_rt), Some(ep_fb), Some(ep_rb)) = (
                proj(eu, -9, 8),
                proj(eu, -12, 9),
                proj(eu, -9, 4),
                proj(eu, -12, 4),
            ) {
                draw_quad(
                    [ep_ft, ep_rt, ep_fb, ep_rb],
                    dark_col.0,
                    dark_col.1,
                    dark_col.2,
                    layer,
                );
            }
        }
    }

    // 10. 3D Nitro / Reactor Super Boost Exhaust Plumes
    if car.boost_ticks > 0 || car.super_boost_ticks > 0 {
        let pulse = (car.boost_ticks.max(car.super_boost_ticks) % 4) as i32 * 2;
        let flame_len = 9 + pulse;
        let (core_col, tip_col) = if car.super_boost_ticks > 0 {
            ((255, 230, 80), (255, 45, 20))
        } else {
            ((180, 250, 255), (0, 160, 255))
        };
        for &fu in &[-3i32, 3i32] {
            if let (Some(tip), Some(b1), Some(b2)) = (
                proj(fu, -12 - flame_len, 2),
                proj(fu - 2, -12, 2),
                proj(fu + 2, -12, 2),
            ) {
                gpu::draw_tri_gouraud_blended(
                    [tip, b1, b2],
                    [tip_col, core_col, core_col],
                    BlendMode::Add,
                );
            }
        }
    }
}

/// Renders a vehicle with hardware-rotated polygons, drop shadow, and visual effects.
pub fn render_car(
    car: &VehicleState,
    camera: &Camera,
    is_ghost: bool,
    primary_color: (u8, u8, u8),
) {
    if camera.mode7 {
        render_car_mode7_3d(car, camera, is_ghost, primary_color);
        return;
    }

    let (cx, cy) = camera.world_to_screen(car.position);

    // Skip if offscreen
    if !(-40..=360).contains(&cx) || !(-40..=280).contains(&cy) {
        return;
    }

    let layer = CarLayer::of(is_ghost);
    let rel_angle = if camera.rotating {
        // Heading relative to the view heading so the player's car points up the screen
        car.visual_angle.wrapping_sub(camera.heading)
    } else {
        car.visual_angle
    };
    let rot = Rot::from_bams(rel_angle);
    let front_wheel_rot = steered_wheel_rot(rel_angle, car.slide_bam() as i16, 4096);

    let z_px = car.z.to_int().clamp(0, 32) as i16;
    let body_cy = cy.saturating_sub(z_px);
    let braking = car.is_drifting
        || car.is_reversing
        || (car.speed.raw() > 1200 && car.engine_rpm < 2400 && car.boost_ticks == 0);
    let (roll_u, pitch_v) = body_roll_pitch_px(
        car.speed.raw(),
        car.slide_bam() as i16,
        car.vz.raw(),
        braking,
    );
    let (upper_cx, upper_cy) = transform_pt(cx, body_cy, roll_u, pitch_v, rot.cos_raw, rot.sin_raw);

    // 0. Night / Tunnel Additive Headlight Cones (NO1-033)
    if !is_ghost && crate::gpu::tracktex::is_night_circuit() {
        render_headlight_cones(car, camera, cx, body_cy, rot);
    }

    // 1. Dynamic Rotated Drop Shadow (projected on the ground at +3 + z_px/2)
    if !is_ghost {
        let shadow_off = 3 + (z_px / 2);
        let shadow = make_quad(cx + shadow_off, cy + shadow_off, 7, 8, 12, 12, rot);
        draw_quad(shadow, 12, 14, 18, layer);
    }

    // 2. Tires / Wheel Pods (4 Black rubber rectangles at the 4 corners; front wheels steer!)
    if !is_ghost {
        // Rear wheels
        let r_left = make_rect_at(cx, body_cy, -7, -6, 2, 4, rot);
        let r_right = make_rect_at(cx, body_cy, 7, -6, 2, 4, rot);
        draw_quad(r_left, 18, 18, 22, layer);
        draw_quad(r_right, 18, 18, 22, layer);

        // Front wheels with live steering angle around each wheel hub
        let (fl_cx, fl_cy) = transform_pt(cx, body_cy, -7, 7, rot.cos_raw, rot.sin_raw);
        let (fr_cx, fr_cy) = transform_pt(cx, body_cy, 7, 7, rot.cos_raw, rot.sin_raw);
        let f_left = make_rect_at(fl_cx, fl_cy, 0, 0, 2, 4, front_wheel_rot);
        let f_right = make_rect_at(fr_cx, fr_cy, 0, 0, 2, 4, front_wheel_rot);
        draw_quad(f_left, 18, 18, 22, layer);
        draw_quad(f_right, 18, 18, 22, layer);
    }

    // 3. Main Aerodynamic Car Body (Gouraud-shaded nose-to-tail contour)
    let (r, g, b) = primary_color;
    let nose_col = (
        r.saturating_add(28),
        g.saturating_add(28),
        b.saturating_add(28),
    );
    let tail_col = scale_rgb((r, g, b), 185);

    // Body: Tapered front hood (w=6) widening to muscular rear haunches (w=7), length ±12
    let body = make_quad(cx, body_cy, 6, 7, 12, 12, rot);
    draw_quad_gouraud(body, [nose_col, nose_col, tail_col, tail_col], layer);

    // 4. True Twin Racing Stripes (Contrasting White or Golden highlight, NO1-030)
    if !is_ghost {
        let stripe_color = if r > 200 && g < 100 {
            (255, 255, 255) // White stripes on red/crimson cars
        } else {
            (255, 220, 40) // Neon gold stripes on blue/green/dark cars
        };
        let (stripe_l, stripe_r) = make_twin_stripes(upper_cx, upper_cy, rot);
        draw_quad(
            stripe_l,
            stripe_color.0,
            stripe_color.1,
            stripe_color.2,
            layer,
        );
        draw_quad(
            stripe_r,
            stripe_color.0,
            stripe_color.1,
            stripe_color.2,
            layer,
        );
    }

    // 5. Cockpit Canopy / Tinted Windshield Glass (shifted by roll/pitch)
    let glass_color = if is_ghost {
        (160, 220, 255)
    } else {
        (25, 38, 55) // Deep polarized reflective glass
    };
    let glass = make_quad(upper_cx, upper_cy, 4, 5, 4, 7, rot);
    draw_quad(glass, glass_color.0, glass_color.1, glass_color.2, layer);

    // Windshield front glare reflection line
    if !is_ghost {
        let p_glare_l = transform_pt(upper_cx, upper_cy, -3, 3, rot.cos_raw, rot.sin_raw);
        let p_glare_r = transform_pt(upper_cx, upper_cy, 3, 3, rot.cos_raw, rot.sin_raw);
        draw_line(p_glare_l, p_glare_r, (120, 160, 200), layer);
    }

    // 6. Rear Aero Wing / Downforce Spoiler + Endplates
    if !is_ghost {
        let wing = make_rect_at(upper_cx, upper_cy, 0, -11, 8, 2, rot);
        let wing_r = (r as u16 * 180 / 255) as u8;
        let wing_g = (g as u16 * 180 / 255) as u8;
        let wing_b = (b as u16 * 180 / 255) as u8;
        draw_quad(wing, wing_r, wing_g, wing_b, layer);
    }

    // 7. Headlights (Front nose corners: bright halogen amber-white)
    let hl_l = transform_pt(cx, body_cy, -4, 11, rot.cos_raw, rot.sin_raw);
    let hl_r = transform_pt(cx, body_cy, 4, 11, rot.cos_raw, rot.sin_raw);
    let hl_l2 = transform_pt(cx, body_cy, -5, 10, rot.cos_raw, rot.sin_raw);
    let hl_r2 = transform_pt(cx, body_cy, 5, 10, rot.cos_raw, rot.sin_raw);
    draw_line(hl_l, hl_l2, (255, 250, 190), layer);
    draw_line(hl_r, hl_r2, (255, 250, 190), layer);

    // 8. Taillights & Illuminated Brake Glow (Rear corners)
    let tl_col = if braking {
        (255, 70, 55)
    } else {
        (235, 25, 25)
    };
    let tl_l = transform_pt(cx, body_cy, -5, -12, rot.cos_raw, rot.sin_raw);
    let tl_r = transform_pt(cx, body_cy, 5, -12, rot.cos_raw, rot.sin_raw);
    let tl_l2 = transform_pt(cx, body_cy, -6, -11, rot.cos_raw, rot.sin_raw);
    let tl_r2 = transform_pt(cx, body_cy, 6, -11, rot.cos_raw, rot.sin_raw);
    draw_line(tl_l, tl_l2, tl_col, layer);
    draw_line(tl_r, tl_r2, tl_col, layer);
    if braking && !is_ghost {
        let b_l = make_rect_at(cx, body_cy, -5, -12, 2, 1, rot);
        let b_r = make_rect_at(cx, body_cy, 5, -12, 2, 1, rot);
        gpu::draw_quad_flat_blended(b_l, 200, 35, 25, BlendMode::Add);
        gpu::draw_quad_flat_blended(b_r, 200, 35, 25, BlendMode::Add);
    }

    // 9. Nitro / Reactor Super Boost Exhaust Flame Jet
    if car.boost_ticks > 0 || car.super_boost_ticks > 0 {
        let ticks = car.boost_ticks.max(car.super_boost_ticks);
        let flame_len = 8 + ((ticks % 4) as i32 * 2);
        let (col_l, col_r) = if car.super_boost_ticks > 0 {
            ((255, 70, 20), (255, 210, 40))
        } else {
            ((0, 220, 255), (255, 140, 20))
        };
        let flame_tip_l = transform_pt(cx, body_cy, -3, -12 - flame_len, rot.cos_raw, rot.sin_raw);
        let flame_base_l1 = transform_pt(cx, body_cy, -4, -12, rot.cos_raw, rot.sin_raw);
        let flame_base_l2 = transform_pt(cx, body_cy, -2, -12, rot.cos_raw, rot.sin_raw);
        gpu::draw_tri_flat(
            [flame_tip_l, flame_base_l1, flame_base_l2],
            col_l.0,
            col_l.1,
            col_l.2,
        );

        let flame_tip_r = transform_pt(cx, body_cy, 3, -12 - flame_len, rot.cos_raw, rot.sin_raw);
        let flame_base_r1 = transform_pt(cx, body_cy, 2, -12, rot.cos_raw, rot.sin_raw);
        let flame_base_r2 = transform_pt(cx, body_cy, 4, -12, rot.cos_raw, rot.sin_raw);
        gpu::draw_tri_flat(
            [flame_tip_r, flame_base_r1, flame_base_r2],
            col_r.0,
            col_r.1,
            col_r.2,
        );
    }
}
