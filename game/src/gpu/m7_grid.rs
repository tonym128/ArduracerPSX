//! Mode 7 ground as a fixed screen-space grid.
//!
//! Hardware-free, like `m7_mesh`, so `tools/test_ui` drives it through the
//! same software rasterizer on the host; `tracktex.rs` writes its polygons as
//! GPU packets.
//!
//! # Why a screen grid
//!
//! The ground's projection has no roll (banking only tilts the sky), so a
//! ground point at depth `z` always lands on row `horizon + K / z`, and a
//! column at a fixed screen `x` is a fixed ratio `right / z`. A grid of whole
//! scanlines and whole pixels is therefore a fixed set of ground directions in
//! view space: each frame only moves it across the world. A vertex costs no
//! projection at all -- its screen position is the grid -- and along a row its
//! texel coordinate advances by a constant step, so the per-vertex work is a
//! handful of additions. The per-tile mesh it replaces projected every point
//! through the GTE, culled and stitched per tile, and set every tile up again.
//!
//! # Why it needs the texture laid out by world position
//!
//! A cell spans several world tiles, and the GPU samples one texture page per
//! polygon. `tracktex` keeps each visible tile at a slot fixed by its world
//! position (tile `(x mod 8, y mod 8)`), so neighbouring tiles are neighbouring
//! texels in VRAM: a 256x256 page holds a 4x4 block of tiles, and a cell only
//! has to be cut where it crosses a page edge or the world's edge. Those are
//! 640 world units apart, so most cells are drawn whole.
//!
//! # Why rows are spaced by depth
//!
//! The GPU interpolates texels affinely, and the error grows with the depth a
//! polygon spans and its size on screen. Equal depth ratios per row keep the
//! error even from the bottom of the screen to the horizon; the host warp test
//! (`m7_mesh_tests`) chose the shape.

use crate::gpu::camera::{Camera, M7_CAM_HEIGHT_Q4, M7_Z0};

/// Most rows and columns a [`GridShape`] may ask for.
pub const MAX_ROWS: usize = 24;
pub const MAX_COLS: usize = 24;

/// Texels per texture page side, in the Q8 texel units [`GridVert`] uses.
pub const PAGE_Q8: i32 = 256 << 8;

/// Most vertices one piece of a cell may have after its cuts.
pub const MAX_PIECE: usize = 8;

/// The grid's layout.
#[derive(Clone, Copy, Debug)]
pub struct GridShape {
    /// Rows from the far plane to the bottom of the screen.
    pub rows: usize,
    /// Columns across the screen.
    pub cols: usize,
    /// How geometric (by depth) rather than uniform (by scanline) the row
    /// spacing is, 0..=256.
    pub geometric_q8: i32,
}

/// The shape the game draws; see the module docs and `m7_mesh_tests`.
///
/// Swept on the host over 24 poses (`grid_sweep`), against the per-tile mesh's
/// worst 7.3 px and 1.9% of pixels over 3 px from ~320 polygons:
///
/// | rows x cols | spacing   | worst  | > 3 px | polygons |
/// |-------------|-----------|--------|--------|----------|
/// | 8 x 8       | 3/4 depth | 3.3 px | 0.07%  | 90       |
/// | 10 x 8      | depth     | 2.5 px | 0      | 111      |
/// | 12 x 10     | depth     | 1.8 px | 0      | 158      |
///
/// Uniform (by scanline) spacing warps worst; depth spacing is best at every
/// size.
pub const SHAPE: GridShape = GridShape {
    rows: 10,
    cols: 8,
    geometric_q8: 256,
};

/// One grid point.
///
/// `u`/`v` are absolute texel coordinates across the circuit's texture in Q8;
/// `z` is view depth and `r` the lateral view offset, both Q4. All four are
/// affine in the ground position, which is what lets a cut point be found by
/// interpolating them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridVert {
    pub sx: i16,
    pub sy: i16,
    pub u: i32,
    pub v: i32,
    pub z: i32,
    pub r: i32,
}

/// Receives the grid's polygons.
pub trait GridSink {
    /// The next polygons lie in the row spanning depths `z_far..z_near` (Q4).
    fn row(&mut self, z_far: i32, z_near: i32);
    /// A polygon inside texture page `page`, in GPU quad order: top-left,
    /// top-right, bottom-left, bottom-right.
    fn quad(&mut self, page: (i32, i32), q: [&GridVert; 4]);
    /// A convex polygon inside texture page `page`, in winding order.
    fn fan(&mut self, page: (i32, i32), v: &[&GridVert]);
}

/// Per-row constants: the row's depth and its first vertex, plus the step from
/// one column to the next.
#[derive(Clone, Copy, Default)]
struct Row {
    sy: i16,
    z: i32,
    u0: i32,
    v0: i32,
    r0: i32,
    du: i32,
    dv: i32,
    dr: i32,
}

/// Everything about the grid that is fixed for one frame.
pub struct GridFrame {
    rows: [Row; MAX_ROWS + 1],
    nrows: usize,
    sx: [i16; MAX_COLS + 1],
    ncols: usize,
    /// Projection constants for cut points: horizon row, centre column,
    /// `M7_CAM_HEIGHT_Q4 * focal`, focal.
    horizon: i32,
    center_x: i32,
    k: i32,
    focal: i32,
    /// The circuit's texture extent (Q8 texels).
    world_q8: (i32, i32),
}

/// `base^(1/n)` in Q16 for `base` in Q16 (>= 1.0), by bisection.
fn nth_root_q16(base_q16: i64, n: u32) -> i64 {
    let (mut lo, mut hi) = (1i64 << 16, base_q16.max(1 << 16));
    for _ in 0..24 {
        let mid = (lo + hi) / 2;
        let mut p = 1i64 << 16;
        for _ in 0..n {
            p = (p * mid) >> 16;
            // Past the target: stop before the product overflows.
            if p >= base_q16 {
                break;
            }
        }
        if p < base_q16 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

impl GridFrame {
    /// Lays the grid over `camera`'s view of a circuit whose tiles are
    /// `tile_wu` world units and `tile_tex` texels a side, `world_tex` texels
    /// in all. Full-screen viewports only: the split-screen scale is not
    /// applied.
    pub fn new(
        camera: &Camera,
        shape: GridShape,
        tile_wu: i32,
        tile_tex: i32,
        world_tex: (i32, i32),
    ) -> Self {
        let rows_n = shape.rows.clamp(1, MAX_ROWS);
        let cols_n = shape.cols.clamp(1, MAX_COLS);
        let focal = camera.m7_focal().max(1);
        let horizon = camera.m7_horizon_row();
        let center_x = camera.center_x as i32;
        let k = M7_CAM_HEIGHT_Q4 * focal;
        let top = (camera.m7_track_top() as i32).max(horizon + 1);
        let bottom = (camera.center_y as i32 + camera.viewport_h as i32 / 2).max(top + 1);

        // Row offsets below the horizon: a blend of geometric (equal depth
        // ratios) and uniform spacing, rounded to whole scanlines.
        let d_top = (top - horizon) as i64;
        let d_bot = (bottom - horizon) as i64;
        let step = nth_root_q16((d_bot << 16) / d_top, rows_n as u32);
        let g = shape.geometric_q8.clamp(0, 256) as i64;
        let mut rows = [Row::default(); MAX_ROWS + 1];
        let mut nrows = 0;
        let mut geo = d_top << 16;
        let mut last = i32::MIN;
        for j in 0..=rows_n {
            let uni = (d_top << 16) + ((d_bot - d_top) << 16) * j as i64 / rows_n as i64;
            let d = if j == rows_n {
                d_bot << 16
            } else {
                (geo * g + uni * (256 - g)) >> 8
            };
            geo = (geo * step) >> 16;
            let sy = horizon + ((d + (1 << 15)) >> 16) as i32;
            let sy = if j == 0 { top } else { sy.clamp(top, bottom) };
            if sy <= last {
                continue;
            }
            last = sy;
            rows[nrows].sy = sy as i16;
            rows[nrows].z = k / (sy - horizon);
            nrows += 1;
        }

        let x0 = camera.center_x as i32 - camera.viewport_w as i32 / 2;
        let w = camera.viewport_w as i32;
        let mut sx = [0i16; MAX_COLS + 1];
        for (i, s) in sx.iter_mut().enumerate().take(cols_n + 1) {
            *s = (x0 + w * i as i32 / cols_n as i32) as i16;
        }

        // Texel coordinates of each row's ends, then an even step between.
        // 32-bit except one multiply: the R3000 has no 64-bit divide.
        let (hs, hc) = (camera.hsin(), camera.hcos());
        let (cam_qx, cam_qy) = (camera.pos.x.raw() >> 8, camera.pos.y.raw() >> 8);
        // Q4 world units to Q8 texels, as a Q16 factor.
        let scale_q16 = (((256 * tile_tex) as i64) << 16) / (16 * tile_wu.max(1)) as i64;
        let to_tex = |w_q4: i32| ((w_q4 as i64 * scale_q16) >> 16) as i32;
        let ground = |r: i32, z: i32| -> (i32, i32) {
            let fwd = z - (M7_Z0 << 4);
            let dx = (r * hc + fwd * hs) >> 12;
            let dy = (r * hs - fwd * hc) >> 12;
            (to_tex(cam_qx + dx), to_tex(cam_qy + dy))
        };
        let n = cols_n as i32;
        for row in rows.iter_mut().take(nrows) {
            let z = row.z;
            let r_at = |s: i16| ((s as i32 - center_x) * z) / focal;
            let (ra, rb) = (r_at(sx[0]), r_at(sx[cols_n]));
            let (ua, va) = ground(ra, z);
            let (ub, vb) = ground(rb, z);
            row.u0 = ua;
            row.v0 = va;
            row.r0 = ra;
            row.du = (ub - ua) / n;
            row.dv = (vb - va) / n;
            row.dr = (rb - ra) / n;
        }

        Self {
            rows,
            nrows,
            sx,
            ncols: cols_n,
            horizon,
            center_x,
            k,
            focal,
            world_q8: (world_tex.0 << 8, world_tex.1 << 8),
        }
    }

    /// Rows the grid actually has (rounding can merge some).
    pub fn rows(&self) -> usize {
        self.nrows.saturating_sub(1)
    }

    /// The vertex at column `i` of row `j`.
    #[inline(always)]
    fn vert(&self, j: usize, i: usize) -> GridVert {
        let row = &self.rows[j];
        let k = i as i32;
        GridVert {
            sx: self.sx[i],
            sy: row.sy,
            u: row.u0 + row.du * k,
            v: row.v0 + row.dv * k,
            z: row.z,
            r: row.r0 + row.dr * k,
        }
    }

    /// Emits every polygon of the grid, far row first.
    pub fn emit<S: GridSink>(&self, sink: &mut S) {
        let mut buf_a = [GridVert::default(); MAX_COLS + 1];
        let mut buf_b = [GridVert::default(); MAX_COLS + 1];
        // The two rows swap roles each row rather than being copied.
        let (mut far, mut near) = (&mut buf_a, &mut buf_b);
        for (i, v) in far.iter_mut().enumerate().take(self.ncols + 1) {
            *v = self.vert(0, i);
        }
        for j in 0..self.rows() {
            for (i, v) in near.iter_mut().enumerate().take(self.ncols + 1) {
                *v = self.vert(j + 1, i);
            }
            sink.row(self.rows[j].z, self.rows[j + 1].z);
            for i in 0..self.ncols {
                self.cell([&far[i], &far[i + 1], &near[i], &near[i + 1]], sink);
            }
            core::mem::swap(&mut far, &mut near);
        }
    }

    /// One cell, given in GPU quad order.
    #[inline(always)]
    fn cell<S: GridSink>(&self, q: [&GridVert; 4], sink: &mut S) {
        let page = |v: &GridVert| (v.u >> 16, v.v >> 16);
        let p = page(q[0]);
        let inside =
            |v: &GridVert| v.u >= 0 && v.v >= 0 && v.u <= self.world_q8.0 && v.v <= self.world_q8.1;
        if q.iter().all(|v| page(v) == p && inside(v)) {
            sink.quad(p, q);
            return;
        }
        self.cut_cell(q, sink);
    }

    /// The slow path: cuts a cell that crosses a page or world edge into
    /// pieces inside one page each, and drops the parts outside the world.
    #[cold]
    #[inline(never)]
    fn cut_cell<S: GridSink>(&self, q: [&GridVert; 4], sink: &mut S) {
        let ring = [*q[0], *q[1], *q[3], *q[2]];
        self.cut_axis(&ring, false, sink);
    }

    /// Splits `poly` along every page line (and the world's edges) on one
    /// axis, then hands each strip to the other axis, or to the sink.
    ///
    /// The two working buffers swap roles after each cut rather than being
    /// copied: on this CPU each copy is a `memcpy` call.
    fn cut_axis<S: GridSink>(&self, poly: &[GridVert], along_v: bool, sink: &mut S) {
        let key = |v: &GridVert| if along_v { v.v } else { v.u };
        let limit = if along_v {
            self.world_q8.1
        } else {
            self.world_q8.0
        };
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for v in poly {
            lo = lo.min(key(v));
            hi = hi.max(key(v));
        }
        if hi <= 0 || lo >= limit {
            return;
        }
        // Most strips cross no line on the second axis: no buffers needed.
        if lo >= 0 && hi <= limit && ((lo >> 16) + 1) * PAGE_Q8 >= hi {
            self.finish(poly, along_v, sink);
            return;
        }
        let mut buf_a = [GridVert::default(); MAX_PIECE];
        let mut buf_b = [GridVert::default(); MAX_PIECE];
        let mut below = [GridVert::default(); MAX_PIECE];
        let (mut rest, mut spare) = (&mut buf_a, &mut buf_b);
        // The part not yet cut: `poly` itself until the first cut.
        let mut rest_n = 0usize;
        let mut first = true;
        // Cut lines in increasing order: the world's near edge (everything
        // below it is dropped), then every page line inside the span. Past
        // the world's far edge everything is dropped too.
        let mut at = if lo < 0 {
            0
        } else {
            ((lo >> 16) + 1) * PAGE_Q8
        };
        loop {
            let end = at >= limit;
            let at_line = at.min(limit);
            if at_line >= hi {
                break;
            }
            let src: &[GridVert] = if first { poly } else { &rest[..rest_n] };
            let (nb, na) = self.split(src, along_v, at_line, &mut below, spare);
            first = false;
            if at_line > 0 && nb >= 3 {
                self.finish(&below[..nb], along_v, sink);
            }
            if end || na < 3 {
                return;
            }
            core::mem::swap(&mut rest, &mut spare);
            rest_n = na;
            at = if at_line == 0 {
                PAGE_Q8
            } else {
                at_line + PAGE_Q8
            };
        }
        let src: &[GridVert] = if first { poly } else { &rest[..rest_n] };
        if src.len() >= 3 {
            self.finish(src, along_v, sink);
        }
    }

    /// A strip cut on one axis: cut it on the other, or emit it.
    fn finish<S: GridSink>(&self, poly: &[GridVert], along_v: bool, sink: &mut S) {
        if !along_v {
            self.cut_axis(poly, true, sink);
            return;
        }
        // Every vertex is inside the page or on its edge, so the box centre
        // names it.
        let (mut u0, mut u1, mut v0, mut v1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for v in poly {
            u0 = u0.min(v.u);
            u1 = u1.max(v.u);
            v0 = v0.min(v.v);
            v1 = v1.max(v.v);
        }
        let page = (((u0 + u1) >> 1) >> 16, ((v0 + v1) >> 1) >> 16);
        let mut refs: [&GridVert; MAX_PIECE] = [&poly[0]; MAX_PIECE];
        for (r, v) in refs.iter_mut().zip(poly.iter()) {
            *r = v;
        }
        if poly.len() == 4 {
            sink.quad(page, [refs[0], refs[1], refs[3], refs[2]]);
        } else {
            sink.fan(page, &refs[..poly.len()]);
        }
    }

    /// Splits convex `poly` at `key == at` into the part below and the part at
    /// or above. Returns their vertex counts.
    fn split(
        &self,
        poly: &[GridVert],
        along_v: bool,
        at: i32,
        below: &mut [GridVert; MAX_PIECE],
        above: &mut [GridVert; MAX_PIECE],
    ) -> (usize, usize) {
        let key = |v: &GridVert| if along_v { v.v } else { v.u };
        let (mut nb, mut na) = (0usize, 0usize);
        let n = poly.len();
        for i in 0..n {
            let a = &poly[i];
            let b = &poly[(i + 1) % n];
            let (ka, kb) = (key(a), key(b));
            if ka < at {
                if nb < MAX_PIECE {
                    below[nb] = *a;
                    nb += 1;
                }
            } else if na < MAX_PIECE {
                above[na] = *a;
                na += 1;
            }
            if (ka < at) != (kb < at) {
                let c = self.cut(a, b, along_v, at);
                if nb < MAX_PIECE {
                    below[nb] = c;
                    nb += 1;
                }
                if na < MAX_PIECE {
                    above[na] = c;
                    na += 1;
                }
            }
        }
        (nb, na)
    }

    /// The point on edge `a`-`b` where `key == at`.
    ///
    /// The endpoints are put in a canonical order first, so the two cells
    /// sharing the edge compute the same point and no pinhole opens between
    /// them. 32-bit throughout: the R3000 has no 64-bit divide, and the
    /// library one cost hundreds of cycles a call. An edge spans at most a
    /// few hundred texels, so the Q12 fraction and its products fit.
    fn cut(&self, a: &GridVert, b: &GridVert, along_v: bool, at: i32) -> GridVert {
        let (a, b) = if (a.z, a.r, a.u, a.v) <= (b.z, b.r, b.u, b.v) {
            (a, b)
        } else {
            (b, a)
        };
        let key = |v: &GridVert| if along_v { v.v } else { v.u };
        let den = key(b) - key(a);
        let t = if den == 0 {
            0
        } else {
            (((at - key(a)) << 12) / den).clamp(0, 4096)
        };
        let lerp = |x: i32, y: i32| x + (((y - x) * t) >> 12);
        let z = lerp(a.z, b.z).max(1);
        let r = lerp(a.r, b.r);
        let (u, v) = if along_v {
            (lerp(a.u, b.u), at)
        } else {
            (at, lerp(a.v, b.v))
        };
        // Depth is always positive here.
        let div_round = |n: i32, d: i32| {
            if n >= 0 {
                (n + d / 2) / d
            } else {
                -((-n + d / 2) / d)
            }
        };
        // On a row (one depth) the screen is linear in the ground, so the cut
        // stays on the row exactly. A column is a vertical screen line, so a
        // cut on one keeps its x exactly; anything else is projected,
        // rounded to the nearest pixel.
        let (sx, sy) = if a.z == b.z {
            (lerp(a.sx as i32, b.sx as i32), a.sy as i32)
        } else {
            let sy = self.horizon + div_round(self.k, z);
            let sx = if a.sx == b.sx {
                a.sx as i32
            } else {
                self.center_x + div_round(r * self.focal, z)
            };
            (sx, sy)
        };
        GridVert {
            sx: sx.clamp(-1000, 1000) as i16,
            sy: sy.clamp(-1000, 1000) as i16,
            u,
            v,
            z,
            r,
        }
    }
}
