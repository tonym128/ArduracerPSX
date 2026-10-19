//! Mode 7 ground mesh: depth-banded subdivision, T-junction stitching and
//! near-plane clipping.
//!
//! Hardware-free -- it takes the projector as a parameter -- so `tools/test_ui`
//! drives the real mesh through a software rasterizer on the host. The renderer
//! in `tracktex.rs` hands it the GTE.
//!
//! # Why subdivide by depth
//!
//! The GPU maps textures affinely, so a big perspective cell swims. The error
//! grows with the depth a cell spans and falls with its distance, so a fixed
//! grid either wastes cells far away or warps up close. A tile is cut into
//! `n x n` cells with `n` chosen from its nearest corner ([`level_for_depth`]).
//!
//! # Why not cut only the axis that runs into the screen
//!
//! It looks free: a tile seen square-on has no depth change across its width,
//! so an 8x1 strip should do the work of 8x8. Measured, it warps three times
//! worse. The GPU draws each quad as two triangles, and the diagonal between
//! them carries the cell's full depth change across the cell's full *screen
//! width*; the displacement scales with both. Error tracks cell area, so a long
//! thin cell buys nothing over a square one of the same area.
//!
//! # Why that does not reopen seams
//!
//! Two tiles at different counts share one edge, cut at different points. The
//! counts are powers of two, so the coarse tile's edge points are a subset of
//! the fine tile's -- and perspective maps a straight ground edge to a straight
//! screen edge, so the fine tile's extra points lie on the coarse edge. "On the
//! edge" after integer rounding is not good enough for a rasterizer, though: a
//! T-junction leaves pinholes. So the *coarse* tile takes the fine neighbour's
//! points into its own boundary cells (a convex polygon, drawn as a fan). Every
//! shared edge then has identical endpoints on both sides, computed from
//! identical inputs, and the mesh is watertight by construction rather than
//! by a guard band. (The October sawtooth came from mixing counts of 3 and 2,
//! which share no interior points at all.)
//!
//! # Why clip rather than clamp
//!
//! Clamping a vertex nearer than `M7_Z_NEAR` to that depth moves it off its
//! edge, which bends the cell and opens a gap against the neighbour that did not
//! clamp. Clipping keeps the cut point on the edge, and the cut is computed with
//! the edge's endpoints in a canonical order so both cells sharing it agree.

use crate::gpu::camera::{Camera, M7_Z0, M7_Z_NEAR};

/// Finest subdivision per tile axis.
pub const MAX_LEVEL: i32 = 8;
/// Grid points per axis at [`MAX_LEVEL`].
const GRID: usize = MAX_LEVEL as usize + 1;
/// Points one tile may project: its grid plus a stitched point on every
/// division of all four sides.
const MAX_POINTS: usize = GRID * GRID + 4 * GRID;
/// Polygon capacity: four corners, up to seven stitched points per side, and
/// the one extra vertex a convex polygon can gain from a single plane clip.
pub const MAX_POLY: usize = 4 + 4 * (MAX_LEVEL as usize - 1) + 1;

/// Nearest-corner depth (Q4) at which a tile drops from 8 to 4, 4 to 2 and
/// 2 to 1 cells per axis.
///
/// Derived from the affine error of the worst-case cell (a diagonal span of
/// `sqrt(2)` cell widths in depth, just in front of the camera): each band
/// starts where the coarser grid's worst error falls to ~2 px. Halving a cell
/// quarters its error, so each threshold is twice the last.
pub const LEVEL_DEPTH_Q4: [i32; 3] = [193 << 4, 386 << 4, 771 << 4];

/// Cells per axis for a tile whose nearest corner is at `zmin_q4`.
pub const fn level_for_depth(zmin_q4: i32) -> i32 {
    if zmin_q4 < LEVEL_DEPTH_Q4[0] {
        8
    } else if zmin_q4 < LEVEL_DEPTH_Q4[1] {
        4
    } else if zmin_q4 < LEVEL_DEPTH_Q4[2] {
        2
    } else {
        1
    }
}

/// A tile's world rectangle, trimmed to the circuit edge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TileRect {
    pub wx0: i32,
    pub wy0: i32,
    /// Trimmed extent.
    pub cw: i32,
    pub ch: i32,
    /// Untrimmed extent; texel scale is always 64 texels per untrimmed tile.
    pub tw: i32,
    pub th: i32,
}

impl TileRect {
    /// The rectangle of tile `(tx, ty)`, or `None` outside the world.
    pub fn new(tile: (i32, i32), tile_wu: (i32, i32), world: (i32, i32)) -> Option<Self> {
        let (tx, ty) = tile;
        let (tw, th) = tile_wu;
        if tx < 0 || ty < 0 || tw <= 0 || th <= 0 {
            return None;
        }
        let wx0 = tx * tw;
        let wy0 = ty * th;
        let cw = (wx0 + tw).min(world.0) - wx0;
        let ch = (wy0 + th).min(world.1) - wy0;
        if cw <= 0 || ch <= 0 {
            return None;
        }
        Some(Self {
            wx0,
            wy0,
            cw,
            ch,
            tw,
            th,
        })
    }
}

/// Depth (Q4) of the tile's nearest corner.
pub fn tile_zmin_q4(camera: &Camera, r: &TileRect) -> i32 {
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;
    let mut zmin = i32::MAX;
    for (x, y) in [
        (r.wx0, r.wy0),
        (r.wx0 + r.cw, r.wy0),
        (r.wx0, r.wy0 + r.ch),
        (r.wx0 + r.cw, r.wy0 + r.ch),
    ] {
        let (_, z) = camera.view_from_offset_q4(x * 16 - cam_qx, y * 16 - cam_qy);
        zmin = zmin.min(z);
    }
    zmin
}

/// Cells per axis for a tile rectangle.
pub fn rect_level(camera: &Camera, r: &TileRect) -> i32 {
    level_for_depth(tile_zmin_q4(camera, r))
}

/// Cells per axis for tile `(tx, ty)`, or 0 when there is no such tile.
pub fn tile_level(
    camera: &Camera,
    tile: (i32, i32),
    tile_wu: (i32, i32),
    world: (i32, i32),
) -> i32 {
    match TileRect::new(tile, tile_wu, world) {
        Some(r) => rect_level(camera, &r),
        None => 0,
    }
}

/// Points per side of tile `(tx, ty)`, top, right, bottom, left: the tile's
/// own level, or a finer neighbour's where one shares that side.
pub fn side_levels(
    camera: &Camera,
    tile: (i32, i32),
    tile_wu: (i32, i32),
    world: (i32, i32),
    own: i32,
) -> [i32; 4] {
    let (tx, ty) = tile;
    let nb = [(tx, ty - 1), (tx + 1, ty), (tx, ty + 1), (tx - 1, ty)];
    let mut out = [own; 4];
    for (s, t) in nb.iter().enumerate() {
        out[s] = own.max(tile_level(camera, *t, tile_wu, world));
    }
    out
}

/// One mesh vertex: camera-relative ground offset, depth, screen position and
/// tile-local texture coordinate.
///
/// Kept to 20 bytes: every cell copies its corners, and on the R3000 every
/// byte of that is an uncached RAM access.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshVert {
    /// Ground offset from the camera, Q4 world units.
    pub dx: i32,
    pub dy: i32,
    /// Camera depth, Q4.
    pub z: i32,
    pub sx: i16,
    pub sy: i16,
    /// Tile-local texture coordinate in 1/16 texel (0..=1024 across a tile).
    pub u: u16,
    pub v: u16,
}

/// Fills `out[k]` with the screen position of ground offset `offs[k]`.
pub trait Projector {
    fn project(&mut self, offs: &[(i32, i32)], out: &mut [(i16, i16)]);
}

/// The software projector, [`Camera::project_cam`] over each offset.
///
/// The host tests use this; the console uses the GTE, which computes the same
/// thing to within its division's rounding.
pub struct SoftwareProjector<'a>(pub &'a Camera);

impl Projector for SoftwareProjector<'_> {
    fn project(&mut self, offs: &[(i32, i32)], out: &mut [(i16, i16)]) {
        for (o, p) in offs.iter().zip(out.iter_mut()) {
            let (r, z) = self.0.view_from_offset_q4(o.0, o.1);
            *p = self.0.project_cam(r, z);
        }
    }
}

/// Per-frame working memory for [`build_tile`].
///
/// Several kilobytes, so it is built once per frame and reused for every tile
/// rather than zeroed on each call: on the R3000 that memset alone was ~6% of
/// the frame, because the stack lives in uncached main RAM.
pub struct MeshScratch {
    pts: [MeshVert; MAX_POINTS],
    /// Outcode of each point; see `OUT_*`.
    code: [u8; MAX_POINTS],
    offs: [(i32, i32); MAX_POINTS],
    scr: [(i16, i16); MAX_POINTS],
    ring: [u8; MAX_POLY],
    poly: [MeshVert; MAX_POLY],
    clipped: [MeshVert; MAX_POLY],
}

impl MeshScratch {
    pub const fn new() -> Self {
        const V: MeshVert = MeshVert {
            dx: 0,
            dy: 0,
            z: 0,
            sx: 0,
            sy: 0,
            u: 0,
            v: 0,
        };
        Self {
            pts: [V; MAX_POINTS],
            code: [0; MAX_POINTS],
            offs: [(0, 0); MAX_POINTS],
            scr: [(0, 0); MAX_POINTS],
            ring: [0; MAX_POLY],
            poly: [V; MAX_POLY],
            clipped: [V; MAX_POLY],
        }
    }
}

impl Default for MeshScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcode bits. A cell whose corners all share one bit is wholly outside that
/// plane and is dropped with one AND, before any per-vertex work. The screen
/// bits are only set for points in front of the near plane, where the
/// projection means something.
const OUT_NEAR: u8 = 1;
const OUT_FAR: u8 = 2;
const OUT_LEFT: u8 = 4;
const OUT_RIGHT: u8 = 8;
const OUT_TOP: u8 = 16;
const OUT_BOTTOM: u8 = 32;

/// Builds every visible polygon of one tile and hands each to `emit`, as its
/// vertices in winding order plus the depth range (Q4) it spans. Returns how
/// many points it projected.
///
/// `n` is the tile's own level from [`rect_level`], `sides` the points per
/// side from [`side_levels`]. Both must be powers of two in `1..=MAX_LEVEL`
/// with each side at least `n`. Polygons wholly outside `view`
/// (`x0, y0, x1, y1`) are dropped.
#[allow(clippy::too_many_arguments)]
pub fn build_tile<P: Projector, E: FnMut(&[MeshVert], i32, i32)>(
    camera: &Camera,
    r: &TileRect,
    n: i32,
    sides: [i32; 4],
    z_far_q4: i32,
    view: (i16, i16, i16, i16),
    scratch: &mut MeshScratch,
    proj: &mut P,
    mut emit: E,
) -> usize {
    let n = n.clamp(1, MAX_LEVEL);
    let lg = n.trailing_zeros();
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;
    let near_q4 = M7_Z_NEAR << 4;
    let (hsin, hcos) = (camera.hsin(), camera.hcos());
    let MeshScratch {
        pts,
        code,
        offs,
        scr,
        ring,
        poly,
        clipped,
    } = scratch;

    // World point -> mesh vertex. Every point goes through this, so a point
    // two tiles share is built from identical integers on both sides. Depth is
    // the same expression as `Camera::view_from_offset_q4`, without the unused
    // lateral term; it is computed directly rather than stepped across the
    // grid, because accumulated steps round differently from a neighbour's.
    let point = |wx: i32, wy: i32, u: u16, v: u16| -> MeshVert {
        let dx = wx * 16 - cam_qx;
        let dy = wy * 16 - cam_qy;
        MeshVert {
            dx,
            dy,
            z: ((dx * hsin - dy * hcos) >> 12) + (M7_Z0 << 4),
            sx: 0,
            sy: 0,
            u,
            v,
        }
    };
    // Texture coordinate of a local offset, in 1/16 texel.
    let tex_u = |ox: i32| ((ox * 1024) / r.tw) as u16;
    let tex_v = |oy: i32| ((oy * 1024) / r.th) as u16;

    let mut count = 0usize;

    // Grid, row-major, stride n + 1. Offsets and texture coordinates are per
    // column and per row, so the divides run once per line, not per point.
    let stride = (n + 1) as usize;
    let mut col = [(0i32, 0u16); GRID];
    let mut row = [(0i32, 0u16); GRID];
    for k in 0..=n {
        let ox = (r.cw * k) >> lg;
        let oy = (r.ch * k) >> lg;
        col[k as usize] = (r.wx0 + ox, tex_u(ox));
        row[k as usize] = (r.wy0 + oy, tex_v(oy));
    }
    for &(wy, v) in &row[..stride] {
        for &(wx, u) in &col[..stride] {
            pts[count] = point(wx, wy, u, v);
            count += 1;
        }
    }

    // Stitched sides: all `m + 1` points of each side finer than the tile.
    // `(L * k) >> lg(m)` equals the neighbour's own `(L * k') >> lg(n')` at
    // every shared point, because the counts are powers of two.
    let mut side_base = [usize::MAX; 4];
    let mut side_step = [1usize; 4];
    for (s, &m_req) in sides.iter().enumerate() {
        let m = m_req.clamp(n, MAX_LEVEL);
        if m == n {
            continue;
        }
        let lm = m.trailing_zeros();
        side_base[s] = count;
        side_step[s] = (m / n) as usize;
        for k in 0..=m {
            let ox = (r.cw * k) >> lm;
            let oy = (r.ch * k) >> lm;
            pts[count] = match s {
                0 => point(r.wx0 + ox, r.wy0, tex_u(ox), 0),
                1 => point(r.wx0 + r.cw, r.wy0 + oy, tex_u(r.cw), tex_v(oy)),
                2 => point(r.wx0 + ox, r.wy0 + r.ch, tex_u(ox), tex_v(r.ch)),
                _ => point(r.wx0, r.wy0 + oy, 0, tex_v(oy)),
            };
            count += 1;
        }
    }

    // Project every point in one batch, so the GTE path can run triples, then
    // classify each once.
    for k in 0..count {
        offs[k] = (pts[k].dx, pts[k].dy);
    }
    proj.project(&offs[..count], &mut scr[..count]);
    for k in 0..count {
        let p = &mut pts[k];
        p.sx = scr[k].0;
        p.sy = scr[k].1;
        code[k] = if p.z < near_q4 {
            OUT_NEAR
        } else {
            let mut c = 0;
            if p.z > z_far_q4 {
                c |= OUT_FAR;
            }
            if p.sx < view.0 {
                c |= OUT_LEFT;
            } else if p.sx > view.2 {
                c |= OUT_RIGHT;
            }
            if p.sy < view.1 {
                c |= OUT_TOP;
            } else if p.sy > view.3 {
                c |= OUT_BOTTOM;
            }
            c
        };
    }

    let any_stitch = side_base.iter().any(|&b| b != usize::MAX);
    let last = n - 1;
    for j in 0..n {
        let row_top = j as usize * stride;
        let row_bot = row_top + stride;
        for i in 0..n {
            let (a, b) = (row_top + i as usize, row_top + i as usize + 1);
            let (c, d) = (row_bot + i as usize, row_bot + i as usize + 1);
            let (ca, cb, cc, cd) = (code[a], code[b], code[c], code[d]);
            // Wholly behind, beyond, or off one side of the screen. Stitched
            // points lie on the cell's edges, so they cannot rescue it.
            if ca & cb & cc & cd != 0 {
                continue;
            }

            // Which of this cell's sides carry stitched points.
            let st = if any_stitch {
                [
                    j == 0 && side_base[0] != usize::MAX,
                    i == last && side_base[1] != usize::MAX,
                    j == last && side_base[2] != usize::MAX,
                    i == 0 && side_base[3] != usize::MAX,
                ]
            } else {
                [false; 4]
            };
            let near_cut = (ca | cb | cc | cd) & OUT_NEAR != 0;

            if !(st[0] || st[1] || st[2] || st[3] || near_cut) {
                // The common case: one quad, straight from the grid.
                let q = [pts[a], pts[b], pts[d], pts[c]];
                let zmin = q[0].z.min(q[1].z).min(q[2].z).min(q[3].z);
                let zmax = q[0].z.max(q[1].z).max(q[2].z).max(q[3].z);
                emit(&q, zmin, zmax);
                continue;
            }

            // Ring: top-left, along the top, top-right, down the right,
            // bottom-right, back along the bottom, bottom-left, up the left.
            // `corner[k]` records where corner k (TL, TR, BR, BL) landed.
            let mut len = 0usize;
            let mut corner = [0usize; 4];
            for (k, &cnr) in [a, b, d, c].iter().enumerate() {
                corner[k] = len;
                ring[len] = cnr as u8;
                len += 1;
                if !st[k] {
                    continue;
                }
                let rr = side_step[k];
                let base = side_base[k];
                // Top and bottom run along x, right and left along y. Top and
                // right are walked forward, bottom and left backward, to keep
                // the winding.
                let along = if k == 0 || k == 2 { i } else { j } as usize;
                let (lo, hi) = (along * rr + 1, (along + 1) * rr);
                if k < 2 {
                    for s in lo..hi {
                        ring[len] = (base + s) as u8;
                        len += 1;
                    }
                } else {
                    for s in (lo..hi).rev() {
                        ring[len] = (base + s) as u8;
                        len += 1;
                    }
                }
            }

            // Start at a corner touching no stitched side, so the renderer's
            // fan from vertex 0 spans the cell rather than running slivers
            // along a row of nearly collinear points. Corner k touches sides
            // k - 1 and k (TL: left and top).
            let mut start = 0usize;
            for k in 0..4 {
                if !st[k] && !st[(k + 3) & 3] {
                    start = corner[k];
                    break;
                }
            }

            let mut zmin = i32::MAX;
            let mut zmax = i32::MIN;
            let mut k = start;
            for slot in poly.iter_mut().take(len) {
                let pv = pts[ring[k] as usize];
                zmin = zmin.min(pv.z);
                zmax = zmax.max(pv.z);
                *slot = pv;
                k += 1;
                if k == len {
                    k = 0;
                }
            }

            let verts: &[MeshVert] = if near_cut {
                let cl = clip_near(&poly[..len], near_q4, clipped, proj);
                if cl < 3 {
                    continue;
                }
                zmin = near_q4;
                &clipped[..cl]
            } else {
                &poly[..len]
            };
            if verts.iter().all(|p| p.sx < view.0)
                || verts.iter().all(|p| p.sx > view.2)
                || verts.iter().all(|p| p.sy < view.1)
                || verts.iter().all(|p| p.sy > view.3)
            {
                continue;
            }
            emit(verts, zmin, zmax);
        }
    }
    count
}

/// Clips the polygon `src` to `z >= near`, writing the result to `out` and
/// projecting the new vertices. Returns the vertex count.
fn clip_near<P: Projector>(
    src: &[MeshVert],
    near: i32,
    out: &mut [MeshVert; MAX_POLY],
    proj: &mut P,
) -> usize {
    let mut len = 0usize;
    let mut fresh = [0usize; 4];
    let mut fresh_n = 0usize;
    let count = src.len();
    for e in 0..count {
        let a = src[e];
        let b = src[if e + 1 == count { 0 } else { e + 1 }];
        let a_in = a.z >= near;
        let b_in = b.z >= near;
        if a_in && len < MAX_POLY {
            out[len] = a;
            len += 1;
        }
        if a_in != b_in && len < MAX_POLY {
            out[len] = cut(a, b, near);
            if fresh_n < fresh.len() {
                fresh[fresh_n] = len;
                fresh_n += 1;
            }
            len += 1;
        }
    }
    if fresh_n > 0 {
        let mut offs = [(0i32, 0i32); 4];
        let mut scr = [(0i16, 0i16); 4];
        for f in 0..fresh_n {
            offs[f] = (out[fresh[f]].dx, out[fresh[f]].dy);
        }
        proj.project(&offs[..fresh_n], &mut scr[..fresh_n]);
        for f in 0..fresh_n {
            out[fresh[f]].sx = scr[f].0;
            out[fresh[f]].sy = scr[f].1;
        }
    }
    len
}

/// The point where edge `a`-`b` crosses depth `near`.
///
/// The endpoints are ordered canonically first: the two cells either side of
/// an edge walk it in opposite directions, and integer interpolation from the
/// other end can land a unit away, which is a crack.
fn cut(a: MeshVert, b: MeshVert, near: i32) -> MeshVert {
    let (a, b) = if (a.dx, a.dy) <= (b.dx, b.dy) {
        (a, b)
    } else {
        (b, a)
    };
    let num = near - a.z;
    let den = b.z - a.z;
    if den == 0 {
        return a;
    }
    // i32 is enough: a cell edge spans at most one tile (2,560 Q4) and the
    // depth term at most the scan radius, well inside 2^31 -- and it avoids
    // the R3000's software 64-bit divide.
    let lerp = |p: i32, q: i32| p + ((q - p) * num) / den;
    MeshVert {
        dx: lerp(a.dx, b.dx),
        dy: lerp(a.dy, b.dy),
        z: near,
        sx: 0,
        sy: 0,
        u: lerp(a.u as i32, b.u as i32) as u16,
        v: lerp(a.v as i32, b.v as i32) as u16,
    }
}
