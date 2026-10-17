//! Where a car should aim on a circuit, shared by the AI and the playtest oracle.
//!
//! Both need the same answer -- "given where I am and which way I am facing,
//! what point do I steer at?" -- and both were answering it wrong in the same
//! way. Aiming at the next checkpoint gate is the obvious answer and it does not
//! work on a closed circuit: the gates sit 800-1300 world units apart on these
//! circuits, so the straight line to one runs across the infield and the car
//! drives into the barrier on the far side. Measured on Longbow before this
//! existed: the AI completed 0 of 5 laps in 60,000 ticks, and `make playtest`'s
//! reference driver completed 0 of 5 laps on 6 of the 24 circuits.
//!
//! So the line is followed from the circuit's own *centreline*
//! ([`TrackDef::route`]), which is the polyline the road was stamped along. Two
//! stages, because neither alone is sufficient:
//!
//! 1. The centreline says which *way*. Walking it puts the aim point on the road
//!    by construction, and always ahead in the driving direction.
//! 2. A road-preferring tile walk decides *where on the road*. The centreline
//!    is resampled from a walk by equal arc length, and the straight chords
//!    between its control points cut the corners the road goes round: measured
//!    across the 24 circuits, 2-14 of the 768 resampled samples per circuit land
//!    on runoff rather than tarmac. Following it exactly would aim into the
//!    runoff, so the walk only ever returns a tile the car can be driven on, and
//!    strongly prefers racing surface.
//!
//! The search for "where am I on the centreline" is windowed around the previous
//! answer, so it is O(1) per tick rather than O(samples); a respawn or a long
//! slide falls out of the window and triggers one full scan.

use crate::math::{Fixed, Vec2};
use crate::track::{TrackDef, TILE_SIZE};

/// How far either side of its last centreline match a car re-searches before
/// falling back to a full scan.
///
/// A car covers a couple of world units per tick and consecutive centreline
/// control points are 60-200 units apart, so the true match never moves more than
/// one sample per tick. This window absorbs a respawn and a shove without ever
/// scanning the whole loop.
const SEARCH_WINDOW: usize = 6;

/// How far off the centreline the nearest-sample match may be and still be
/// trusted, in world units.
///
/// A car-and-a-half wide, in world units: generous enough for a car running wide
/// on exit or dodging traffic, tight enough that a genuine centreline sample is
/// always nearer than any other. Beyond it the search has lost the plot and a
/// full scan is cheaper than continuing to guess.
///
/// Deliberately *not* `2 * TILE_SIZE`. This is a driving distance, not a grid
/// resolution: tied to `TILE_SIZE` it silently halved when the surface grid went
/// from 64-unit to 32-unit cells, and a 64-unit match radius is inside one road
/// width -- measured on The Esses, where the rival lost the line at the hairpin
/// exit and sat wedged against the infield for 700+ ticks.
const MATCH_UNITS: i32 = 128;

/// Squared world distance, in whole units. Squared because callers only compare.
pub fn distance_squared(a: Vec2, b: Vec2) -> i32 {
    let (dx, dy) = ((a.x - b.x).to_int() as i64, (a.y - b.y).to_int() as i64);
    (dx * dx + dy * dy).min(i32::MAX as i64) as i32
}

/// Closest point to `p` on the segment `a`-`b`.
///
/// Uses the segment length rather than its square so a long segment does not
/// lose precision, and degenerates to `a` for a zero-length segment -- which a
/// duplicated control point produces.
fn project_onto_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let (dx, dy) = ((b.x - a.x).to_int() as i64, (b.y - a.y).to_int() as i64);
    let len_sq = dx * dx + dy * dy;
    if len_sq == 0 {
        return a;
    }
    let (px, py) = ((p.x - a.x).to_int() as i64, (p.y - a.y).to_int() as i64);
    let t = (((px * dx + py * dy) * 4096) / len_sq).clamp(0, 4096) as i32;
    a + (b - a).scale(Fixed::from_raw(t))
}

/// Integer square root, for segment lengths. `Fixed::length` would do, but this
/// runs once per neighbour per candidate tile.
pub fn segment_units(dx: i32, dy: i32) -> i32 {
    let (x, y) = (dx.unsigned_abs() as u64, dy.unsigned_abs() as u64);
    let square = x * x + y * y;
    if square < 2 {
        return square as i32;
    }
    let mut x0 = square;
    let mut x1 = x0.div_ceil(2);
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + square / x0) / 2;
    }
    x0.min(i32::MAX as u64) as i32
}

/// World-space centre of a tile.
fn tile_centre(tx: u8, ty: u8) -> Vec2 {
    Vec2::new(
        Fixed::from_int(tx as i32 * TILE_SIZE + TILE_SIZE / 2),
        Fixed::from_int(ty as i32 * TILE_SIZE + TILE_SIZE / 2),
    )
}

/// Whether a tile coordinate is somewhere a car can be driven.
///
/// [`TrackDef::tile_at`] answers `Barrier` outside the grid, so this doubles as
/// an in-bounds test.
fn is_drivable(track: &TrackDef, tx: u8, ty: u8) -> bool {
    !track.tile_at(tx, ty).is_solid()
}

/// Neighbour offsets, axis-aligned first so a straight tile beats a diagonal on
/// equal merit.
const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
];

/// Score penalty for a drivable tile that is not racing surface.
///
/// Large enough to lose to any positional term, so a racing-surface tile always
/// wins while one is available.
const OFF_LINE_PENALTY: i64 = 1 << 50;

/// How far ahead along the racing line a car aims, in world units.
///
/// Four tiles. Far enough to be a real pure-pursuit target rather than a jitter
/// around the nose, short enough that the road's own width dominates the choice.
pub const DEFAULT_LOOKAHEAD_UNITS: i32 = 128;

/// Maximum tiles the road-following walk will cross.
///
/// Bounds the walk so it cannot wander: a car four tiles ahead is well inside
/// this, so the bound only bites when the road genuinely ends.
const MAX_TILE_WALK: usize = 14;

/// Follows a circuit's centreline, remembering where on it the car was.
///
/// Total: every index is wrapped or bounds-checked, `lookahead` is clamped
/// before use as a distance, and a track with no centreline falls back to
/// steering at `fallback` (a track with no centreline has no racing line to
/// follow, so the caller decides what to aim at).
#[derive(Clone, Debug, Default)]
pub struct RacingLine {
    /// Index into [`TrackDef::route`] of the centreline sample nearest the car.
    hint: usize,
    /// Whether `hint` means anything yet.
    hint_valid: bool,
}

impl RacingLine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the cached centreline match.
    ///
    /// For after the car has been teleported: the hint still gets re-validated,
    /// but dropping it makes the next query a full scan rather than a confident
    /// wrong answer.
    pub fn reset(&mut self) {
        self.hint_valid = false;
    }

    /// Index of the centreline sample last matched, and whether it means
    /// anything yet.
    ///
    /// A search hint only, and re-derived and re-validated on every query, so a
    /// stale value costs a wider search rather than correctness. Exposed because
    /// lap progress is measured as forward movement along this line.
    pub fn hint(&self) -> usize {
        self.hint
    }

    /// Whether a centreline match has been established.
    pub fn has_match(&self) -> bool {
        self.hint_valid
    }

    /// Centreline sample index nearest `pos`, searched around the cached hint.
    fn nearest_index(&mut self, track: &TrackDef, pos: Vec2) -> Option<usize> {
        let route = track.route;
        let n = route.len();
        if n < 2 {
            return None;
        }
        let hint = if self.hint_valid { self.hint % n } else { n };

        let mut best = (0usize, i32::MAX);
        for offset in 0..=SEARCH_WINDOW {
            for side in [0usize, 1] {
                let idx = if side == 0 {
                    hint.wrapping_add(offset) % n
                } else {
                    (hint + n - offset % n) % n
                };
                let d = distance_squared(route[idx], pos);
                if d < best.1 {
                    best = (idx, d);
                }
            }
            // A genuine match is always the closest thing in its neighbourhood,
            // so two either side is enough to be sure.
            if best.1 <= MATCH_UNITS * MATCH_UNITS && offset >= 2 {
                break;
            }
        }
        // Too far from anything the window found: the hint is stale (a respawn,
        // or a slide across the circuit). One full scan settles it.
        if best.1 > MATCH_UNITS * MATCH_UNITS {
            best = (0, i32::MAX);
            for (idx, p) in route.iter().enumerate() {
                let d = distance_squared(*p, pos);
                if d < best.1 {
                    best = (idx, d);
                }
            }
        }
        self.hint = best.0;
        self.hint_valid = true;
        Some(best.0)
    }

    /// A point `lookahead` world units along the centreline from `pos`.
    ///
    /// This is the *direction* stage: on the road by construction, and always
    /// ahead. It can still be off the surface where the centreline cuts a corner,
    /// which is what [`RacingLine::aim`] then corrects for.
    pub fn centreline_point(
        &mut self,
        track: &TrackDef,
        pos: Vec2,
        lookahead: i32,
    ) -> Option<Vec2> {
        let route = track.route;
        let n = route.len();
        let start = self.nearest_index(track, pos)?;
        let lookahead = lookahead.max(0);

        // Start from where the car actually is *on the line*, not from the
        // matched sample: control points are 60-200 units apart, so beginning at
        // the sample would put the point up to a whole step further away than
        // asked for. The nearest *sample* is not necessarily the nearest
        // *segment* -- the car may be past the midpoint -- so both segments
        // meeting at it are projected and the closer one wins.
        let prev = (start + n - 1) % n;
        let on_prev = project_onto_segment(pos, route[prev], route[start]);
        let on_next = project_onto_segment(pos, route[start], route[(start + 1) % n]);
        let (mut idx, mut from) =
            if distance_squared(on_prev, pos) <= distance_squared(on_next, pos) {
                (prev, on_prev)
            } else {
                (start, on_next)
            };

        // Walk forward, accumulating real segment lengths, so `lookahead` is
        // world units rather than samples.
        let mut travelled = 0i32;
        for _ in 0..=n {
            let b = route[(idx + 1) % n];
            let step = segment_units((b.x - from.x).to_int(), (b.y - from.y).to_int()).max(1);
            if travelled + step >= lookahead {
                let fraction = ((lookahead - travelled).max(0) as i64 * 4096) / step as i64;
                return Some(
                    from + (b - from).scale(Fixed::from_raw(fraction.clamp(0, 4096) as i32)),
                );
            }
            travelled += step;
            idx = (idx + 1) % n;
            from = b;
        }
        Some(route[start])
    }

    /// The point to steer at from `pos`, facing `heading`.
    ///
    /// Walks the centreline [`RacingLine::centreline_point`] for the direction,
    /// then homes in on that with a road-preferring tile walk, so the answer is a
    /// tile the car can actually be driven onto however far the centreline strays.
    ///
    /// `fallback` is the aim point for a circuit with no authored centreline.
    /// The tile walk cannot get lost: it is bounded by [`MAX_TILE_WALK`] steps, a
    /// `visited` list stops it doubling back, and every step moves it closer to
    /// the target, so it always makes progress towards it.
    pub fn aim(
        &mut self,
        track: &TrackDef,
        pos: Vec2,
        heading: u16,
        lookahead: i32,
        fallback: Vec2,
    ) -> Vec2 {
        let lookahead = lookahead.max(0);
        let target = self
            .centreline_point(track, pos, lookahead)
            .unwrap_or(fallback);
        self.tile_walk(track, pos, heading, lookahead, target)
    }

    /// Greedy tile-by-tile walk from `pos` towards `target`.
    fn tile_walk(
        &self,
        track: &TrackDef,
        pos: Vec2,
        heading: u16,
        lookahead: i32,
        target: Vec2,
    ) -> Vec2 {
        let mut visited = [(0u8, 0u8); MAX_TILE_WALK + 1];
        let mut visited_len = 1usize;
        let mut tx = crate::track::TrackDef::tile_x_of(pos.x).min(track.width.saturating_sub(1));
        let mut ty = crate::track::TrackDef::tile_y_of(pos.y).min(track.height.saturating_sub(1));
        visited[0] = (tx, ty);

        let mut from = pos;
        let mut travelled = 0i32;
        // Direction the line is currently going, seeded from the car's heading so
        // the walk starts by following the car and only bends when the road does.
        let mut leg = heading;
        for _ in 0..MAX_TILE_WALK {
            let mut best_score = i64::MAX;
            let mut best: Option<(u8, u8)> = None;
            for (dx, dy) in NEIGHBOURS {
                let nx = tx as i32 + dx;
                let ny = ty as i32 + dy;
                if nx < 0 || ny < 0 || nx >= track.width as i32 || ny >= track.height as i32 {
                    continue;
                }
                let (nx, ny) = (nx as u8, ny as u8);
                if !is_drivable(track, nx, ny) || visited[..visited_len].contains(&(nx, ny)) {
                    continue;
                }
                let centre = tile_centre(nx, ny);
                let to_x = (target.x - centre.x).to_int() as i64;
                let to_y = (target.y - centre.y).to_int() as i64;
                let distance = to_x * to_x + to_y * to_y;
                // Shaping: among near-equal candidates keep going the way the
                // line already goes, so it hugs the middle of the road instead of
                // sawing across it.
                let align = crate::ai::angle_error(crate::ai::heading_towards(from, centre), leg)
                    .abs() as i64;
                // Racing surface first: off-road is drivable, but a line that
                // uses it is a line that cuts.
                let surface = if track.is_road_at(nx, ny) {
                    0
                } else {
                    OFF_LINE_PENALTY
                };
                let score = distance * 8 + align * 64 + surface;
                if score < best_score {
                    best_score = score;
                    best = Some((nx, ny));
                }
            }
            let (nx, ny) = match best {
                Some(next) => next,
                None => break,
            };
            let centre = tile_centre(nx, ny);
            let step =
                segment_units((centre.x - from.x).to_int(), (centre.y - from.y).to_int()).max(1);
            if travelled + step >= lookahead {
                let fraction = ((lookahead - travelled).max(0) as i64 * 4096) / step as i64;
                let aim =
                    from + (centre - from).scale(Fixed::from_raw(fraction.clamp(0, 4096) as i32));
                // The aim point must be somewhere a car can actually be.
                //
                // The walk only ever *steps* through road tiles, but two things
                // can still put the aim point somewhere a car cannot: the segment
                // between two road tiles can clip a corner and cross runoff, and
                // the road itself can have a hole in it (a chicane's own infield,
                // or two straights a cell apart). Either way the aim point is a
                // world position the steering error is measured against, so an
                // aim point inside scenery makes the car steer into it. Measured
                // on Chicane Park: the aim sat at tile (24,8), a hole between two
                // parts of the circuit, and held the car against the far side of
                // it for 60,000 ticks without completing a lap.
                //
                // A no-op whenever the aim point was already fine.
                let aim_tile = (
                    TrackDef::tile_x_of(aim.x).min(track.width.saturating_sub(1)),
                    TrackDef::tile_y_of(aim.y).min(track.height.saturating_sub(1)),
                );
                return if track.is_road_at(aim_tile.0, aim_tile.1) {
                    aim
                } else {
                    from
                };
            }
            travelled += step;
            leg = crate::ai::heading_towards(from, centre);
            from = centre;
            tx = nx;
            ty = ny;
            visited[visited_len] = (nx, ny);
            visited_len += 1;
        }
        from
    }
}
