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
//!
//! # Vertex order is load-bearing
//!
//! Both helpers below emit their four vertices in **"Z" order** -- top-left,
//! top-right, bottom-left, bottom-right -- and that is not a cosmetic choice.
//! `psx_gpu`'s quad primitives rasterise as `tri(v0, v1, v2)` **plus**
//! `tri(v1, v2, v3)`: the shared edge is `v1-v2`, so `v1` and `v2` must be
//! *diagonally opposite* corners. Handed a perimeter ordering
//! (`v0, v1, v2, v3` = TL, TR, BR, BL) the second triangle comes out as
//! `{TR, BR, BL}` where the complement of the first needs `{TL, BR, BL}`, and
//! the pair leaves the `v0` corner of the quad unpainted -- a wedge-shaped hole
//! that reads as a missing flank on the car.
//!
//! `make_quad` and `make_rect_at` were briefly "corrected" into perimeter
//! order on the theory that the original ordering was a self-intersecting
//! bowtie. It was not: for a convex quad `tri(v0,v1,v2) + tri(v1,v2,v3)`
//! tiles Z-ordered corners exactly, and perimeter order is precisely the
//! ordering the primitive cannot fill. The `twice_area` regression test added
//! at the time did not catch it either -- the shoelace sum of a quad with one
//! corner missing is still comfortably positive, so "area > 0" passed on a
//! shape that was a fifth short. [`quad_tiles_its_interior`] and
//! [`quad_covers_its_interior`] are the checks that actually discriminate.

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
    // Z order: front-left, front-right, rear-left, rear-right. `v1` (front-right)
    // and `v2` (rear-left) are diagonally opposite, which is what the primitive's
    // `tri(v0,v1,v2) + tri(v1,v2,v3)` split needs to tile the trapezoid. See the
    // module docs.
    [
        transform_pt(cx, cy, -w_front, l_front, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, w_front, l_front, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, -w_rear, -l_rear, rot.cos_raw, rot.sin_raw),
        transform_pt(cx, cy, w_rear, -l_rear, rot.cos_raw, rot.sin_raw),
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
    // Z order, as in `make_quad`: top-left, top-right, bottom-left, bottom-right.
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

/// Twice the *signed* area of a triangle: the cross product of its two edge
/// vectors. Sign gives the winding; magnitude is twice the area.
fn signed_twice_area(t: [(i64, i64); 3]) -> i64 {
    let (a, b, c) = (t[0], t[1], t[2]);
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Twice the area the hardware paints for a four-vertex quad.
///
/// This is the sum of the two triangles the primitive emits,
/// `tri(v0, v1, v2)` and `tri(v1, v2, v3)`, rather than a shoelace sum around
/// the polygon. The shoelace formula is only meaningful for a perimeter
/// traversal, and these helpers emit Z order on purpose, so summing around the
/// polygon would report near zero for a perfectly good quad.
///
/// Zero means every vertex is collinear, which for these quads means the shape
/// has collapsed to a line and cannot rasterise to anything visible.
///
/// Replaces the old `twice_area`, whose shoelace sum was blind to the
/// regression: it reported a healthy area for the notched shape. Note this
/// measure is *also* blind to it -- see [`quad_tiles_its_interior`] -- so it
/// answers "is any of this visible at all", not "is it the right shape".
pub fn painted_twice_area(pts: &[(i16, i16); 4]) -> i64 {
    let v: [(i64, i64); 4] = [
        (pts[0].0 as i64, pts[0].1 as i64),
        (pts[1].0 as i64, pts[1].1 as i64),
        (pts[2].0 as i64, pts[2].1 as i64),
        (pts[3].0 as i64, pts[3].1 as i64),
    ];
    signed_twice_area([v[0], v[1], v[2]]).abs() + signed_twice_area([v[1], v[2], v[3]]).abs()
}

/// Whether `p` lies inside triangle `t` (edges count as inside).
///
/// A plain barycentric sign test. The question being asked is which of two
/// triangles a primitive covers, not how the hardware rounds edges, so this
/// does not try to model the rasteriser's fill rules.
fn inside_tri(p: (i64, i64), t: [(i64, i64); 3]) -> bool {
    let sign = |a: (i64, i64), b: (i64, i64), c: (i64, i64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let d0 = sign(t[0], t[1], p);
    let d1 = sign(t[1], t[2], p);
    let d2 = sign(t[2], t[0], p);
    let neg = d0 < 0 || d1 < 0 || d2 < 0;
    let pos = d0 > 0 || d1 > 0 || d2 > 0;
    !(neg && pos)
}

/// Whether the PSX quad split tiles `pts` exactly, with no gap and no overlap.
///
/// The hardware draws a four-vertex quad as `tri(v0, v1, v2)` plus
/// `tri(v1, v2, v3)`, sharing the `v1`-`v2` edge. That tiles the quad exactly
/// **iff `v1` and `v2` are diagonally opposite corners**. Two ways to get it
/// wrong:
///
/// - If `v1`-`v2` is a *side* rather than a diagonal, the two triangles sit on
///   the same side of it: they overlap, and the wedge they jointly miss at the
///   `v0` corner is never painted. On the car that reads as a hole through the
///   body.
/// - If the quad is degenerate (an edge collapsed), one triangle has zero area.
///
/// Both collapse to one test: the two signed areas must have *opposite* signs
/// and neither may be zero. Same-sign means the shared edge is a side and the
/// fill overlaps; zero means a triangle has no area to contribute.
///
/// Note that no *area* measure can see any of this, including
/// [`painted_twice_area`]. Perimeter order trades the overlap for the gap, so
/// the painted total is identical to a correct Z ordering's, and the shoelace sum
/// around the polygon is identical too. Both reported a healthy positive number
/// for a quad that was missing a fifth of its interior. Only the shape of the
/// fill distinguishes them.
pub fn quad_tiles_its_interior(pts: &[(i16, i16); 4]) -> bool {
    let v: [(i64, i64); 4] = [
        (pts[0].0 as i64, pts[0].1 as i64),
        (pts[1].0 as i64, pts[1].1 as i64),
        (pts[2].0 as i64, pts[2].1 as i64),
        (pts[3].0 as i64, pts[3].1 as i64),
    ];
    let a = signed_twice_area([v[0], v[1], v[2]]);
    let b = signed_twice_area([v[1], v[2], v[3]]);
    a != 0 && b != 0 && (a < 0) != (b < 0)
}

/// Whether a sample grid over the quad's interior lands inside the two
/// triangles the primitive emits.
///
/// An independent restatement of [`quad_tiles_its_interior`] that reasons about
/// *coverage* rather than orientation, so the two cannot share a mistake.
///
/// "Interior" needs no hull computation: the union of the four triangles formed
/// by omitting one corner at a time is exactly the convex hull of the four
/// points (each lies inside the hull, and together they cover it). So a sample
/// is interior iff it is in at least one of those four.
pub fn quad_covers_its_interior(pts: &[(i16, i16); 4]) -> bool {
    let v: [(i64, i64); 4] = [
        (pts[0].0 as i64, pts[0].1 as i64),
        (pts[1].0 as i64, pts[1].1 as i64),
        (pts[2].0 as i64, pts[2].1 as i64),
        (pts[3].0 as i64, pts[3].1 as i64),
    ];
    // The two triangles the quad primitive emits.
    let drawn = [[v[0], v[1], v[2]], [v[1], v[2], v[3]]];
    // The four triangles whose union is the convex hull.
    let hull = [
        [v[1], v[2], v[3]],
        [v[0], v[2], v[3]],
        [v[0], v[1], v[3]],
        [v[0], v[1], v[2]],
    ];
    let (min_x, max_x) = (
        [v[0].0, v[1].0, v[2].0, v[3].0]
            .iter()
            .min()
            .copied()
            .unwrap_or(0),
        [v[0].0, v[1].0, v[2].0, v[3].0]
            .iter()
            .max()
            .copied()
            .unwrap_or(0),
    );
    let (min_y, max_y) = (
        [v[0].1, v[1].1, v[2].1, v[3].1]
            .iter()
            .min()
            .copied()
            .unwrap_or(0),
        [v[0].1, v[1].1, v[2].1, v[3].1]
            .iter()
            .max()
            .copied()
            .unwrap_or(0),
    );
    let mut y = min_y;
    while y <= max_y {
        let mut x = min_x;
        while x <= max_x {
            let p = (x, y);
            if hull.iter().any(|t| inside_tri(p, *t)) && !drawn.iter().any(|t| inside_tri(p, *t)) {
                return false;
            }
            x += 1;
        }
        y += 1;
    }
    true
}
