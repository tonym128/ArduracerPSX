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

use crate::gpu::camera::{Camera, M7_Z_NEAR};

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
/// Each band boundary is twice the last, because halving a cell quarters its
/// affine error. The values were chosen by sweeping the host warp test
/// (`m7_mesh_tests`): 136/272/544 holds the same worst case (~5.4 px, at the
/// bottom corners) and the same share of pixels over 3 px (~0.4%) as the
/// stricter 193/386/771 the error model first suggested, with a quarter fewer
/// polygons; 115/230/460 starts to lose quality. Polygons are the expensive
/// part on this hardware.
pub const LEVEL_DEPTH_Q4: [i32; 3] = [136 << 4, 272 << 4, 544 << 4];

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
///
/// Depth is linear across the ground (`x * sin h - y * cos h`), so the nearest
/// corner is known from the heading's signs: one evaluation, not four. This
/// runs five times per tile, for the tile and its four neighbours.
pub fn tile_zmin_q4(camera: &Camera, r: &TileRect) -> i32 {
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;
    let x = if camera.hsin() > 0 {
        r.wx0
    } else {
        r.wx0 + r.cw
    };
    let y = if camera.hcos() > 0 {
        r.wy0 + r.ch
    } else {
        r.wy0
    };
    camera
        .view_from_offset_q4(x * 16 - cam_qx, y * 16 - cam_qy)
        .1
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
/// final texture coordinate.
///
/// Kept to 20 bytes, and carrying the texture coordinate already mapped into
/// the texture page: the quad path reads four of these per cell and writes
/// them to the GPU nearly as they are, and on the R3000 every byte is an
/// uncached RAM access.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct MeshVert {
    /// Ground offset from the camera, Q4 world units.
    pub dx: i32,
    pub dy: i32,
    /// Camera depth, Q4.
    pub z: i32,
    pub sx: i16,
    pub sy: i16,
    /// Texture coordinate in texture-page units; see [`TexMap`].
    pub u: u8,
    pub v: u8,
}

/// How a tile's 64x64 texels land in its texture page.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TexMap {
    /// The tile's top-left texel in the page.
    pub u0: u16,
    pub v0: u16,
    /// Right shift from texels to U units (the page's colour depth).
    pub u_shift: u8,
}

impl TexMap {
    /// Tile-local texels, 0..=64, for callers with no texture page.
    pub const LOCAL: TexMap = TexMap {
        u0: 0,
        v0: 0,
        u_shift: 0,
    };
}

/// Projects mesh vertices: reads each `dx`/`dy` ground offset and writes
/// `sx`/`sy` and the depth `z`.
///
/// Depth is the projector's job because the GTE produces it for free alongside
/// the screen position (`SZ`), which saves two multiplies per point. It must
/// equal [`Camera::view_from_offset_q4`]'s depth exactly, including for points
/// behind the camera -- clipping and culling both read it, and a point two tiles
/// share must classify the same way in both.
///
/// It also outcodes each point with `oc` as it goes, while the projected values
/// are still in registers; a second pass over the points re-read them all from
/// uncached RAM.
pub trait Projector {
    fn project(&mut self, pts: &mut [MeshVert], code: &mut [u8], oc: &Outcoder);
}

/// The software projector, [`Camera::project_cam`] over each vertex.
///
/// The host tests use this; the console uses the GTE, which computes the same
/// thing to within its division's rounding.
pub struct SoftwareProjector<'a>(pub &'a Camera);

impl Projector for SoftwareProjector<'_> {
    fn project(&mut self, pts: &mut [MeshVert], code: &mut [u8], oc: &Outcoder) {
        for (p, c) in pts.iter_mut().zip(code.iter_mut()) {
            let (r, z) = self.0.view_from_offset_q4(p.dx, p.dy);
            (p.sx, p.sy) = self.0.project_cam(r, z);
            p.z = z;
            *c = oc.code(p);
        }
    }
}

/// Receives the mesh's polygons.
pub trait Sink {
    /// One plain grid cell, corners in the GPU's quad order: top-left,
    /// top-right, bottom-left, bottom-right. By reference, because this is the
    /// path nearly every cell takes and copying its corners was measurable.
    fn quad(&mut self, q: [&MeshVert; 4]);
    /// A stitched or clipped cell: a convex polygon in winding order, to be
    /// drawn as a fan from vertex 0.
    fn fan(&mut self, verts: &[MeshVert]);
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

/// The parts of a tile's point set the cell pass needs to find its way round:
/// where each stitched side's points start and how many there are per cell.
pub struct Layout {
    n: i32,
    /// Cells built: `[i0, i1) x [j0, j1)`; see [`visible_cells`].
    i0: i32,
    i1: i32,
    j0: i32,
    j1: i32,
    /// Points per row of the built sub-grid.
    stride: usize,
    side_base: [usize; 4],
    side_step: [usize; 4],
    /// The outcoder the points were classified with, for clipped cut points.
    oc: Outcoder,
}

impl Layout {
    /// Point index of grid corner `(i, j)` within the built sub-grid.
    #[inline(always)]
    fn at(&self, i: i32, j: i32) -> usize {
        (j - self.j0) as usize * self.stride + (i - self.i0) as usize
    }
}

/// Builds every visible polygon of one tile and hands each to `sink`. Returns
/// how many points it projected.
///
/// `n` is the tile's own level from [`rect_level`], `sides` the points per
/// side from [`side_levels`]. Both must be powers of two in `1..=MAX_LEVEL`
/// with each side at least `n`. `tex` places the tile's texels in the texture
/// page. `oc` carries the far plane and the view; polygons wholly outside the
/// view are dropped. It depends only on the camera, so build it once a frame.
///
/// The stages are out of line and kept small on purpose. This runs once per
/// visible tile, and everything it executes has to share the R3000's 4 KB
/// instruction cache: inlined together the first version was 17 KB, and even
/// split up, a 7 KB per-tile path refilled the cache from RAM on every tile --
/// about 7,000 cycles a tile before any real work.
#[allow(clippy::too_many_arguments)]
pub fn build_tile<P: Projector, S: Sink>(
    camera: &Camera,
    r: &TileRect,
    n: i32,
    sides: [i32; 4],
    tex: TexMap,
    oc: &Outcoder,
    scratch: &mut MeshScratch,
    proj: &mut P,
    sink: &mut S,
) -> usize {
    let n = n.clamp(1, MAX_LEVEL);
    let cells = visible_cells(camera, r, n, oc);
    let (count, layout) = make_points(camera, r, n, sides, tex, cells, *oc, scratch);
    proj.project(&mut scratch.pts[..count], &mut scratch.code[..count], oc);
    emit_cells(&layout, scratch, proj, sink);
    count
}

/// Camera-relative offset (Q4) and page coordinate of grid line `k` of `2^lg`
/// along one tile axis.
///
/// The one formula every point goes through, grid and stitched side alike, so
/// a point two tiles share -- or a tile's grid and its own side -- is built from
/// identical integers. `origin_q4` is the axis origin's offset from the camera,
/// `extent` the trimmed and `full` the untrimmed length, `t0` and `shift` the
/// tile's place in its texture page. An untrimmed tile's lines fall on whole
/// texels, `64k / 2^lg`; only a tile cut by the circuit edge divides.
///
/// Out of line on purpose: inlined at each call site it made `make_points`
/// 5 KB under LTO, more than the whole instruction cache.
#[inline(never)]
fn line(origin_q4: i32, extent: i32, full: i32, k: i32, lg: u32, t0: u16, shift: u8) -> (i32, u8) {
    let o = (extent * k) >> lg;
    let texel = if extent == full {
        (64 * k) >> lg
    } else {
        (o * 64 + full / 2) / full
    };
    (
        origin_q4 + o * 16,
        ((t0 as i32 + texel) >> shift).min(255) as u8,
    )
}

/// Generates the tile's grid and stitched side points, unprojected.
///
/// Deliberately small: it runs once per tile, and the whole per-tile path has
/// to share the R3000's 4 KB instruction cache. The first version was 1.7 KB
/// of closures and a four-armed side `match`, and with the rest of the path it
/// refilled the cache from RAM on every tile.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn make_points(
    camera: &Camera,
    r: &TileRect,
    n: i32,
    sides: [i32; 4],
    tex: TexMap,
    (i0, i1, j0, j1): (i32, i32, i32, i32),
    oc: Outcoder,
    scratch: &mut MeshScratch,
) -> (usize, Layout) {
    let lg = n.trailing_zeros();
    let ox0 = r.wx0 * 16 - (camera.pos.x.raw() >> 8);
    let oy0 = r.wy0 * 16 - (camera.pos.y.raw() >> 8);
    let xl = |k: i32, lg: u32| line(ox0, r.cw, r.tw, k, lg, tex.u0, tex.u_shift);
    let yl = |k: i32, lg: u32| line(oy0, r.ch, r.th, k, lg, tex.v0, 0);
    let pts = &mut scratch.pts;
    let mut layout = Layout {
        n,
        i0,
        i1,
        j0,
        j1,
        stride: (i1 - i0 + 1).max(0) as usize,
        side_base: [usize::MAX; 4],
        side_step: [1; 4],
        oc,
    };
    if i0 >= i1 || j0 >= j1 {
        layout.i1 = i0;
        return (0, layout);
    }

    // The visible sub-grid, row-major; columns are tabulated once.
    let mut col = [(0i32, 0u8); GRID];
    for (slot, k) in col.iter_mut().zip(i0..=i1) {
        *slot = xl(k, lg);
    }
    let mut count = 0usize;
    for k in j0..=j1 {
        let (dy, v) = yl(k, lg);
        for &(dx, u) in &col[..layout.stride] {
            pts[count] = MeshVert {
                dx,
                dy,
                z: 0,
                sx: 0,
                sy: 0,
                u,
                v,
            };
            count += 1;
        }
    }

    // Stitched sides: all `m + 1` points of each side finer than the tile,
    // if the built sub-grid reaches that side. Top and bottom (0, 2) run along
    // x at y line 0 or 1 of 1; right and left (1, 3) along y at x line 1 or 0.
    let reached = [j0 == 0, i1 == n, j1 == n, i0 == 0];
    for s in 0..4 {
        let m = sides[s].clamp(n, MAX_LEVEL);
        if m == n || !reached[s] {
            continue;
        }
        let lm = m.trailing_zeros();
        let at_end = (s == 1 || s == 2) as i32;
        layout.side_base[s] = count;
        layout.side_step[s] = (m / n) as usize;
        for k in 0..=m {
            let (dx, u, dy, v) = if s & 1 == 0 {
                let (a, b) = (xl(k, lm), yl(at_end, 0));
                (a.0, a.1, b.0, b.1)
            } else {
                let (a, b) = (xl(at_end, 0), yl(k, lm));
                (a.0, a.1, b.0, b.1)
            };
            pts[count] = MeshVert {
                dx,
                dy,
                z: 0,
                sx: 0,
                sy: 0,
                u,
                v,
            };
            count += 1;
        }
    }
    (count, layout)
}

/// The cells of a tile that can reach the view, as `(i0, i1, j0, j1)`: cells
/// `[i0, i1) x [j0, j1)`. Conservative -- a whole row or column is kept if any
/// of it might show.
///
/// A tile near the camera is cut 8x8 but usually straddles it, or a screen
/// edge, and generating and projecting the points of cells that cannot show
/// was most of the point cost. Every visibility test here is a linear function
/// of ground position -- depth, and lateral offset against the screen edges --
/// so its value anywhere in the tile follows from its value at three corners by
/// interpolation. Each row and column is then tested with shifts and adds,
/// without projecting anything. Points that are built are computed exactly as
/// before, so neighbours still agree on every shared point.
fn visible_cells(camera: &Camera, r: &TileRect, n: i32, oc: &Outcoder) -> (i32, i32, i32, i32) {
    // Coarse tiles have too few points for the test to pay for itself.
    if n < 4 {
        return (0, n, 0, n);
    }
    let lg = n.trailing_zeros();
    let cam_qx = camera.pos.x.raw() >> 8;
    let cam_qy = camera.pos.y.raw() >> 8;
    let c = |x: i32, y: i32| camera.view_from_offset_q4(x * 16 - cam_qx, y * 16 - cam_qy);
    let c00 = c(r.wx0, r.wy0);
    let c10 = c(r.wx0 + r.cw, r.wy0);
    let c01 = c(r.wx0, r.wy0 + r.ch);

    let Frustum {
        f,
        z_lo,
        z_hi,
        x_lo,
        x_hi,
    } = oc.frustum;
    // Each plane as (right, depth) -> value, non-negative on the visible side.
    let planes = |(rq, z): (i32, i32)| -> [i32; 4] {
        [z - z_lo, z_hi - z, rq * f - x_lo * z, x_hi * z - rq * f]
    };
    let (p00, p10, p01) = (planes(c00), planes(c10), planes(c01));

    // Bit k of `rows` / `cols` survives while row / column k may be visible.
    let (mut rows, mut cols) = ((1u32 << n) - 1, (1u32 << n) - 1);
    for q in 0..4 {
        let (v, dx, dy) = (p00[q], p10[q] - p00[q], p01[q] - p00[q]);
        rows &= lines_on_plane(v + dx.max(0), dy, n, lg);
        cols &= lines_on_plane(v + dy.max(0), dx, n, lg);
    }
    if rows == 0 || cols == 0 {
        return (0, 0, 0, 0);
    }
    let span = |m: u32| (m.trailing_zeros() as i32, 32 - m.leading_zeros() as i32);
    let ((i0, i1), (j0, j1)) = (span(cols), span(rows));
    (i0, i1, j0, j1)
}

/// Bit k set for each of `n` strips whose largest plane value is non-negative.
///
/// The largest value over strip k is `base + (step * k + max(step, 0)) >> lg`:
/// linear in k, so it is stepped by addition rather than multiplied out. Out of
/// line so the four planes share one copy rather than unrolling into four.
#[inline(never)]
fn lines_on_plane(base: i32, step: i32, n: i32, lg: u32) -> u32 {
    let mut mask = 0u32;
    let mut acc = step.max(0);
    for k in 0..n {
        if base + (acc >> lg) >= 0 {
            mask |= 1 << k;
        }
        acc += step;
    }
    mask
}

/// Outcodes points, so a cell is culled with one AND.
///
/// Points behind the near plane get `OUT_NEAR` -- and, when the near plane
/// meets the ground below the view, `OUT_BOTTOM` too. Clipping would replace
/// such a point with a cut point on that row, below the screen, so a cell whose
/// in-front corners are all below the view is culled by the same AND as any
/// other instead of being clipped only to be thrown away. Near the camera that
/// is a whole row of every close tile.
#[derive(Copy, Clone)]
pub struct Outcoder {
    near_q4: i32,
    z_far_q4: i32,
    view: (i16, i16, i16, i16),
    behind: u8,
    frustum: Frustum,
}

/// The view's planes on the ground, for [`visible_cells`]: depth from the
/// screen's bottom row (or the near plane, if further) to the far plane, and
/// the screen edges as offsets from the projection centre -- each padded for
/// rounding. Per frame, since every term depends only on the camera.
#[derive(Copy, Clone)]
struct Frustum {
    /// Focal length, pixels.
    f: i32,
    z_lo: i32,
    z_hi: i32,
    x_lo: i32,
    x_hi: i32,
}

impl Outcoder {
    pub fn new(camera: &Camera, z_far_q4: i32, view: (i16, i16, i16, i16)) -> Self {
        let near_q4 = M7_Z_NEAR << 4;
        let near_row = camera.project_cam(0, near_q4).1;
        let f = (camera.m7_focal() * camera.m7_view_scale_q12()) / 4096;
        let below = view.3 as i32 - camera.m7_horizon_row();
        let z_bottom = if below > 0 { (2080 * f) / below } else { 0 };
        let cx = camera.center_x as i32;
        Self {
            near_q4,
            z_far_q4,
            view,
            frustum: Frustum {
                f,
                z_lo: near_q4.max(z_bottom) - 32,
                z_hi: z_far_q4 + 32,
                x_lo: view.0 as i32 - cx - 4,
                x_hi: view.2 as i32 - cx + 4,
            },
            behind: if near_row > view.3 {
                OUT_NEAR | OUT_BOTTOM
            } else {
                OUT_NEAR
            },
        }
    }

    /// The outcode of a projected point.
    #[inline(always)]
    pub fn code(&self, p: &MeshVert) -> u8 {
        if p.z < self.near_q4 {
            return self.behind;
        }
        let mut c = 0;
        if p.z > self.z_far_q4 {
            c |= OUT_FAR;
        }
        if p.sx < self.view.0 {
            c |= OUT_LEFT;
        } else if p.sx > self.view.2 {
            c |= OUT_RIGHT;
        }
        if p.sy < self.view.1 {
            c |= OUT_TOP;
        } else if p.sy > self.view.3 {
            c |= OUT_BOTTOM;
        }
        c
    }
}

/// The per-cell pass: culls, sends plain cells straight to the sink, and
/// hands the rest to [`emit_patched`]. Kept small; it runs per primitive.
#[inline(never)]
fn emit_cells<P: Projector, S: Sink>(
    l: &Layout,
    scratch: &mut MeshScratch,
    proj: &mut P,
    sink: &mut S,
) {
    let n = l.n;
    let side = l.side_base.map(|b| b != usize::MAX);
    for j in l.j0..l.j1 {
        for i in l.i0..l.i1 {
            let (a, c) = (l.at(i, j), l.at(i, j + 1));
            let (b, d) = (a + 1, c + 1);
            let code = &scratch.code;
            let (ca, cb, cc, cd) = (code[a], code[b], code[c], code[d]);
            // Wholly behind, beyond, or off one side of the screen. Stitched
            // points lie on the cell's edges, so they cannot rescue it.
            if ca & cb & cc & cd != 0 {
                continue;
            }
            let near_cut = (ca | cb | cc | cd) & OUT_NEAR != 0;
            let edge = (j == 0 && side[0])
                || (i == n - 1 && side[1])
                || (j == n - 1 && side[2])
                || (i == 0 && side[3]);
            if !near_cut && !edge {
                // The common case: one quad, straight from the grid.
                let pts = &scratch.pts;
                sink.quad([&pts[a], &pts[b], &pts[c], &pts[d]]);
                continue;
            }
            emit_patched(l, (i, j), [a, b, d, c], near_cut, scratch, proj, sink);
        }
    }
}

/// A cell on a stitched side or across the near plane: gathers its ring,
/// clips it if needed, and sends it as a fan.
#[inline(never)]
#[cold]
fn emit_patched<P: Projector, S: Sink>(
    l: &Layout,
    (i, j): (i32, i32),
    corners: [usize; 4],
    near_cut: bool,
    scratch: &mut MeshScratch,
    proj: &mut P,
    sink: &mut S,
) {
    let MeshScratch {
        pts,
        ring,
        poly,
        clipped,
        ..
    } = scratch;
    let last = l.n - 1;
    let view = l.oc.view;
    // Which of this cell's sides carry stitched points.
    let st = [
        j == 0 && l.side_base[0] != usize::MAX,
        i == last && l.side_base[1] != usize::MAX,
        j == last && l.side_base[2] != usize::MAX,
        i == 0 && l.side_base[3] != usize::MAX,
    ];

    // Ring: top-left, along the top, top-right, down the right, bottom-right,
    // back along the bottom, bottom-left, up the left. `corner[k]` records
    // where corner k (TL, TR, BR, BL) landed.
    let mut len = 0usize;
    let mut corner = [0usize; 4];
    for (k, &cnr) in corners.iter().enumerate() {
        corner[k] = len;
        ring[len] = cnr as u8;
        len += 1;
        if !st[k] {
            continue;
        }
        let rr = l.side_step[k];
        let base = l.side_base[k];
        // Top and bottom run along x, right and left along y. Top and right
        // are walked forward, bottom and left backward, to keep the winding.
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

    // Start at a corner touching no stitched side, so the fan from vertex 0
    // spans the cell rather than running slivers along a row of nearly
    // collinear points. Corner k touches sides k - 1 and k (TL: left and top).
    let mut start = 0usize;
    for k in 0..4 {
        if !st[k] && !st[(k + 3) & 3] {
            start = corner[k];
            break;
        }
    }

    let mut k = start;
    for slot in poly.iter_mut().take(len) {
        *slot = pts[ring[k] as usize];
        k += 1;
        if k == len {
            k = 0;
        }
    }

    let verts: &[MeshVert] = if near_cut {
        let cl = clip_near(&poly[..len], M7_Z_NEAR << 4, clipped, proj, &l.oc);
        if cl < 3 {
            return;
        }
        &clipped[..cl]
    } else {
        &poly[..len]
    };
    if verts.iter().all(|p| p.sx < view.0)
        || verts.iter().all(|p| p.sx > view.2)
        || verts.iter().all(|p| p.sy < view.1)
        || verts.iter().all(|p| p.sy > view.3)
    {
        return;
    }
    // A plain four-sided cell that only came here for the near plane, but was
    // not actually cut, is still a quad.
    if verts.len() == 4 && !st.iter().any(|&x| x) {
        sink.quad([&verts[0], &verts[1], &verts[3], &verts[2]]);
    } else {
        sink.fan(verts);
    }
}

/// Clips the polygon `src` to `z >= near`, writing the result to `out` and
/// projecting the new vertices. Returns the vertex count.
#[inline(never)]
#[cold]
fn clip_near<P: Projector>(
    src: &[MeshVert],
    near: i32,
    out: &mut [MeshVert; MAX_POLY],
    proj: &mut P,
    oc: &Outcoder,
) -> usize {
    let mut len = 0usize;
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
            let mut fresh = [cut(a, b, near)];
            proj.project(&mut fresh, &mut [0], oc);
            // The cut lies on the plane by construction; keep that exact
            // rather than whatever the rounded offsets re-project to.
            fresh[0].z = near;
            out[len] = fresh[0];
            len += 1;
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
        u: lerp(a.u as i32, b.u as i32) as u8,
        v: lerp(a.v as i32, b.v as i32) as u8,
    }
}
