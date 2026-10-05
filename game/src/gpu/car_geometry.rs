//! Hardware-free polygon geometry helpers for the vehicle renderer.
//!
//! Split out of `car_renderer.rs` so `tools/test_ui` can exercise the vertex
//! maths on the host. `car_renderer.rs` itself draws through `psx_gpu`, which
//! writes MMIO and cannot be linked into a host test; these helpers are pure
//! integer arithmetic over a Q20.12 rotation.
//!
//! This split exists because of TASK-1203: the cockpit canopy passed a
//! negative `l_rear` to `make_quad`, which negates that argument internally, so
//! the two length arguments cancelled and every vertex landed on the same row.
//! The "Tinted Cockpit Canopy" feature had measured zero area at 0, 90, 180 and
//! 270 degrees and nothing noticed, because nothing could check.

use arduracer_core::{math::cos, math::sin};

/// Rotates a local-space point into screen space about `(cx, cy)`.
///
/// `cos_raw` / `sin_raw` are Q20.12.
#[inline(always)]
pub fn transform_pt(cx: i16, cy: i16, u: i32, v: i32, cos_raw: i32, sin_raw: i32) -> (i16, i16) {
    let x = cx as i32 + ((u * cos_raw + v * sin_raw) >> 12);
    let y = cy as i32 + ((u * sin_raw - v * cos_raw) >> 12);
    (x as i16, y as i16)
}

/// A cached heading, as the raw Q20.12 sine/cosine the vertex transform wants.
///
/// Bundled because every geometry helper needs both halves and passing them as
/// two scalars pushes `make_quad`/`make_rect_at` over clippy's argument limit.
#[derive(Copy, Clone, Debug)]
pub struct Rot {
    pub cos_raw: i32,
    pub sin_raw: i32,
}

impl Rot {
    #[inline(always)]
    pub fn from_bams(angle: u16) -> Self {
        Rot {
            cos_raw: cos(angle).raw() as i32,
            sin_raw: sin(angle).raw() as i32,
        }
    }
}

/// A tapered quad: `l_rear` is negated internally, so **both** length arguments
/// must be positive. A negative value does not mirror the shape, it collapses
/// it onto the front row and yields zero area.
#[inline(always)]
pub fn make_quad(
    cx: i16,
    cy: i16,
    w_front: i32,
    w_rear: i32,
    l_front: i32,
    l_rear: i32,
    rot: Rot,
) -> [(i16, i16); 4] {
    debug_assert!(
        l_front >= 0 && l_rear >= 0,
        "make_quad negates l_rear internally; a negative length collapses the quad"
    );
    // Perimeter order, not bowtie order. Emitting front-left, front-right,
    // rear-left, rear-right makes the polygon self-intersect, so the SPU fills
    // a figure-of-eight rather than the trapezoid and the shoelace area sums to
    // zero. Traverse the outline instead: front-left, front-right, rear-right,
    // rear-left.
    [
        transform_pt(cx, cy, -w_front, l_front, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, w_front, l_front, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, w_rear, -l_rear, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, -w_rear, -l_rear, rot.cos_raw, rot.sin_raw),
    ]
}

/// An axis-aligned-in-local-space rectangle centred on `(center_u, center_v)`.
#[inline(always)]
pub fn make_rect_at(
    cx: i16,
    cy: i16,
    center_u: i32,
    center_v: i32,
    half_w: i32,
    half_l: i32,
    rot: Rot,
) -> [(i16, i16); 4] {
    // Perimeter order for the same reason as `make_quad`: top-left, top-right,
    // bottom-right, bottom-left.
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
            center_u + half_w,
            center_v - half_l,
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
    ]
}

/// Twice the signed area of a polygon by the shoelace formula.
///
/// Zero means every vertex is collinear, which for these quads means the shape
/// has collapsed to a line and cannot rasterise to anything visible.
pub fn twice_area(pts: &[(i16, i16); 4]) -> i64 {
    let mut acc: i64 = 0;
    for i in 0..4 {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[(i + 1) % 4];
        acc += x0 as i64 * y1 as i64 - x1 as i64 * y0 as i64;
    }
    acc.abs()
}
