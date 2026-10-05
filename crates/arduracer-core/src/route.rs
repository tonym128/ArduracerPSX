//! Track centrelines and distance-along-track.
//!
//! Every circuit's racing line is defined by its ordered gate positions: the
//! cooker sorts the legacy gate tiles into driving order with a two-opt tour, and
//! the AI drives gate centre to gate centre. There is no separate authored
//! centreline, so this module *derives* one by fitting a closed centripetal
//! Catmull-Rom spline through those ordered gate centres.
//!
//! That choice matters for two reasons. It needs no level-data change -- the
//! ordered gates are already in [`TrackDef::route_node`] -- so it works for all 24
//! circuits uniformly, including the four Super Stages. And because the spline
//! passes through the same points the AI steers between, the validation line and
//! the racing line agree by construction rather than by coincidence.
//!
//! [`Route`] is the unit of progress: [`Route::nearest`] converts a world
//! position into a distance along the centreline, which is what makes
//! centreline-relative checkpoints and direction-aware lap validation possible
//! (they previously used axis-aligned tile boxes, so a lap could be scored by
//! driving backwards through every gate).

use crate::math::{Fixed, Vec2, FP_ONE, FP_SHIFT};

/// Upper bound on generated centreline samples.
///
/// 17 gates (16 checkpoints plus the start/finish) at 8 samples per span is 136,
/// so 192 leaves headroom without needing a heap allocation on hardware.
pub const MAX_ROUTE_SAMPLES: usize = 192;

/// Default spline resolution: samples generated per span between gates.
pub const DEFAULT_SAMPLES_PER_SPAN: usize = 8;

/// A closed centreline with cumulative arc lengths, in world units.
///
/// `PartialEq`/`Eq` so a timer carrying a route stays comparable; it is plain
/// data with no float members.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Route {
    points: [Vec2; MAX_ROUTE_SAMPLES],
    /// Arc length at each sample. `arc[0]` is always 0.
    arc: [u16; MAX_ROUTE_SAMPLES],
    count: usize,
    total: u16,
}

impl Default for Route {
    fn default() -> Self {
        Route::empty()
    }
}

impl Route {
    const fn empty() -> Self {
        Route {
            points: [Vec2::ZERO; MAX_ROUTE_SAMPLES],
            arc: [0; MAX_ROUTE_SAMPLES],
            count: 0,
            total: 0,
        }
    }

    /// Fits a closed centripetal Catmull-Rom spline through `nodes` and
    /// precomputes arc lengths.
    ///
    /// Fewer than three nodes cannot describe a loop, so the result is empty and
    /// [`Route::is_empty`] reports it. Arc lengths saturate rather than wrap; a
    /// circuit longer than 65535 world units is not representable and would need
    /// a wider type, which no shipped circuit approaches.
    pub fn from_nodes(nodes: &[Vec2], samples_per_span: usize) -> Self {
        let mut route = Route::empty();
        let n = nodes.len();
        if n < 3 || samples_per_span == 0 {
            return route;
        }

        let per_span = samples_per_span.clamp(1, 16);
        // Number of spans equals number of nodes (the loop closes), and each span
        // contributes `per_span` samples.
        let want = n * per_span;
        let count = want.min(MAX_ROUTE_SAMPLES);

        for i in 0..count {
            // Which span this sample belongs to, and how far through it.
            let span = i / per_span;
            let t = Fixed::from_int((i % per_span) as i32) / Fixed::from_int(per_span as i32);
            let p0 = nodes[(span + n - 1) % n];
            let p1 = nodes[span % n];
            let p2 = nodes[(span + 1) % n];
            let p3 = nodes[(span + 2) % n];
            route.points[i] = catmull_rom(p0, p1, p2, p3, t);
        }
        route.count = count;

        // Cumulative arc length around the loop, in *world units*.
        //
        // Deliberately not Q20.12: `arc` and `total` are `u16`, so raw units
        // would saturate at 65535 -- i.e. any circuit longer than 16 world units
        // would report a full lap as 65535 and every gate would collapse onto one
        // arc position. `distance_raw` already returns whole world units.
        let mut acc: u32 = 0;
        route.arc[0] = 0;
        for i in 1..count {
            let d = distance_raw(route.points[i], route.points[i - 1]).max(0) as u32;
            acc = acc.saturating_add(d);
            route.arc[i] = acc.min(u16::MAX as u32) as u16;
        }
        // Closing segment back to the first sample.
        let d = distance_raw(route.points[0], route.points[count - 1]).max(0) as u32;
        route.total = acc.saturating_add(d).min(u16::MAX as u32) as u16;
        route
    }

    /// Builds the route from a track's ordered gate centres.
    ///
    /// Samples are clamped into the circuit bounds afterwards. Uniform Catmull-Rom
    /// overshoots at hairpins, and several legacy circuits have gates one tile
    /// from the edge, so without this the centreline can leave the track -- which
    /// would put lap validation outside the world. Clamping here rather than in
    /// the interpolator keeps the curve's shape intact everywhere else.
    pub fn from_track(track: &crate::track::TrackDef) -> Self {
        // Preferred: the control points the cooker rasterised the road along. Same
        // curve, same parameterisation, so the centreline *is* the road rather than
        // an approximation of it.
        if !track.route.is_empty() {
            return Route::from_nodes(track.route, DEFAULT_SAMPLES_PER_SPAN);
        }
        // Fallback for hand-written fixtures with no authored centreline: fit one
        // through the gate centres. An approximation, and measurably not the road --
        // which is exactly why the cooker emits the real thing.
        let mut nodes: [Vec2; 17] = [Vec2::ZERO; 17];
        let n = track.route_len().min(17);
        for (i, slot) in nodes.iter_mut().enumerate().take(n) {
            *slot = crate::track::TrackDef::gate_centre(&track.route_node(i));
        }
        Route::from_nodes(&nodes[..n], DEFAULT_SAMPLES_PER_SPAN)
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Number of centreline samples.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Iterate the centreline samples in order.
    pub fn points_iter(&self) -> impl Iterator<Item = Vec2> + '_ {
        self.points[..self.count].iter().copied()
    }

    /// Centreline sample `i`.
    pub fn point(&self, i: usize) -> Vec2 {
        self.points[i.min(self.count.saturating_sub(1))]
    }

    /// Distance along the centreline of sample `i`, in world units.
    pub fn arc_at(&self, i: usize) -> u16 {
        self.arc[i.min(self.count.saturating_sub(1))]
    }

    /// Total length of the closed loop, in world units.
    pub fn total_len(&self) -> u16 {
        self.total
    }

    /// Nearest point on the centreline to `pos`.
    ///
    /// `hint` is the index of the previous call's result and makes this an O(1)
    /// local search, which is what makes it affordable every frame on hardware.
    /// Pass [`Route::len`] to force a full scan (used on the first call and after
    /// a respawn, where the car can be anywhere).
    ///
    /// Returns `(arc_position, distance_to_centreline, sample_index)`.
    pub fn nearest(&self, pos: Vec2, hint: usize) -> (u16, i32, usize) {
        if self.count == 0 {
            return (0, 0, 0);
        }
        if self.count == 1 {
            let d = distance_raw(pos, self.points[0]);
            return (0, d, 0);
        }

        let best = if hint < self.count {
            let local = self.scan_window(pos, hint, 4);
            // A respawn or a big cut can leave the hint far from the car. If the
            // local search reports a distance larger than a third of the lap, the
            // hint is stale, so fall back to a full scan.
            if local.1 < (self.total as i32) / 3 {
                Some(local)
            } else {
                None
            }
        } else {
            None
        };
        let (arc, dist, idx) = best.unwrap_or_else(|| self.scan_all(pos));
        (arc, dist, idx)
    }

    /// Smallest and largest sample index in a window around `hint`, wrapping.
    fn scan_window(&self, pos: Vec2, hint: usize, radius: usize) -> (u16, i32, usize) {
        let n = self.count;
        let mut best = (0u16, i32::MAX, hint);
        for step in 0..=radius * 2 {
            let idx = (hint + n - (radius % n) + step) % n;
            let (arc, d, _) = self.project(pos, idx);
            if d < best.1 {
                best = (arc, d, idx);
            }
            if step == 0 {
                continue;
            }
            // Mirror the pass over the backwards neighbours.
            let back = (hint + n - (step % n)) % n;
            let (arc, d, _) = self.project(pos, back);
            if d < best.1 {
                best = (arc, d, back);
            }
        }
        best
    }

    fn scan_all(&self, pos: Vec2) -> (u16, i32, usize) {
        let mut best = (0u16, i32::MAX, 0usize);
        for i in 0..self.count {
            let (arc, d, _) = self.project(pos, i);
            if d < best.1 {
                best = (arc, d, i);
            }
        }
        best
    }

    /// Projects `pos` onto the segment starting at sample `i`, returning the
    /// interpolated arc position, perpendicular distance and clamped sample index.
    fn project(&self, pos: Vec2, i: usize) -> (u16, i32, usize) {
        let n = self.count;
        let a = self.points[i];
        let b = self.points[(i + 1) % n];
        // i64: raw differences are up to ~8 million, and squaring them overflows
        // i32 on any segment longer than a couple of tiles.
        let (dx, dy) = (
            b.x.raw() as i64 - a.x.raw() as i64,
            b.y.raw() as i64 - a.y.raw() as i64,
        );
        let len_sq = dx * dx + dy * dy;
        if len_sq == 0 {
            return (self.arc[i], distance_raw(pos, a), i);
        }
        let (px, py) = (
            pos.x.raw() as i64 - a.x.raw() as i64,
            pos.y.raw() as i64 - a.y.raw() as i64,
        );
        // Projection parameter, clamped to the segment.
        let dot = px * dx + py * dy;
        let t = (dot / len_sq).clamp(0, 1) as i32;
        let cx = (a.x.raw() as i64 + (dx >> FP_SHIFT) * t as i64) as i32;
        let cy = (a.y.raw() as i64 + (dy >> FP_SHIFT) * t as i64) as i32;
        let closest = Vec2 {
            x: Fixed::from_raw(cx),
            y: Fixed::from_raw(cy),
        };
        let seg_len = (isqrt(len_sq as u64) >> FP_SHIFT) as i32;
        let arc = (self.arc[i] as i32 + seg_len * t).clamp(0, self.total as i32) as u16;
        (arc, distance_raw(pos, closest), i)
    }

    /// True when progress moved from `prev` to `cur` across `gate_arc` in the
    /// given direction.
    ///
    /// This is the whole of centreline-relative lap validation: the previous
    /// tile-box test could not tell a forward crossing from a reverse one, which
    /// is why driving backwards through every gate used to score a lap.
    pub fn crossed(&self, prev: u16, cur: u16, gate_arc: u16, forward: bool) -> bool {
        let total = self.total as i32;
        if total == 0 {
            return false;
        }
        let (p, c, g) = (prev as i32, cur as i32, gate_arc as i32);

        // Reject implausible progress: a cut across the infield, a respawn or a
        // teleport. Measured as distance *travelled around the loop*, not the
        // linear gap, because a legitimate lap-boundary wrap has a huge linear
        // gap and a tiny travelled distance.
        let travelled = if forward {
            if c >= p {
                c - p
            } else {
                total - p + c
            }
        } else if c <= p {
            p - c
        } else {
            p + total - c
        };
        if travelled > total / 4 {
            return false;
        }

        if forward {
            // Wrapping forward: p <= g < c, or the step crossed the lap boundary.
            (p <= g && g < c) || (c < p && (g >= p || g < c))
        } else {
            (c <= g && g < p) || (p < c && (g <= p || g > c))
        }
    }
}

/// Euclidean distance in world units, as a whole number.
///
/// Integer only: the hardware has no FPU, so a float square root would be
/// emulated in software on every call, and this runs per frame.
fn distance_raw(a: Vec2, b: Vec2) -> i32 {
    let dx = (a.x.raw() as i64 - b.x.raw() as i64).abs();
    let dy = (a.y.raw() as i64 - b.y.raw() as i64).abs();
    (isqrt((dx * dx + dy * dy) as u64) as i32) >> FP_SHIFT
}

/// Integer square root by bit peeling: exact, branch-predictable, and no float.
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut rem = n;
    let mut root = 0u64;
    let mut bit = 1u64 << 62;
    while bit > rem {
        bit >>= 2;
    }
    while bit != 0 {
        let trial = root + bit;
        if rem >= trial {
            rem -= trial;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// Uniform Catmull-Rom interpolation between `p1` and `p2`.
///
/// Uniform parameterisation: it provably passes through its control points, which
/// matters because the control points *are* the gate centres, so the centreline
/// must pass through every gate. Centripetal parameterisation avoids overshoot on
/// tight turns but does *not* interpolate its control points, which is the wrong
/// trade here.
///
/// The known weakness is overshoot at hairpins. That is handled by clamping the
/// finished route into the circuit bounds (see `Route::from_track`) rather than
/// by clamping per-sample to the control hull: a hull clamp also catches
/// *interior* samples that merely pass near a hull extreme, and flattens whole
/// spans onto a single point.
///
/// `i64` intermediates: the cubic terms overflow `i32` in Q20.12.
fn catmull_rom(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: Fixed) -> Vec2 {
    let t = t.raw() as i64;
    let t2 = (t * t) >> FP_SHIFT;
    let t3 = (t2 * t) >> FP_SHIFT;

    fn axis(a: Fixed, b: Fixed, c: Fixed, d: Fixed, t: i64, t2: i64, t3: i64) -> i64 {
        let one = FP_ONE as i64;
        let (a, b, c, d) = (
            a.raw() as i64,
            b.raw() as i64,
            c.raw() as i64,
            d.raw() as i64,
        );
        // `a..d` are Q20.12 and `t`, `t2`, `t3` are Q12 fractions, so *every*
        // product needs exactly one renormalising divide by `one`. Dividing the
        // squared and cubed terms by `one^2` and `one^3` instead shrinks the
        // curvature by 4096x and 16.7M x, which flattens the spline into a
        // straight line between control points.
        let sum = (2 * b)
            + (-a + c) * t / one
            + (2 * a - 5 * b + 4 * c - d) * t2 / one
            + (-a + 3 * b - 3 * c + d) * t3 / one;
        sum >> 1
    }

    Vec2 {
        x: Fixed::from_raw(
            axis(p0.x, p1.x, p2.x, p3.x, t, t2, t3).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        ),
        y: Fixed::from_raw(
            axis(p0.y, p1.y, p2.y, p3.y, t, t2, t3).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        ),
    }
}
