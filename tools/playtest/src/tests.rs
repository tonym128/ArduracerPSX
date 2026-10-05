//! Unit tests for the invariants the playability harness *asserts* but cannot
//! observe from a lap-time summary.
//!
//! `main.rs` is the oracle; this module is the microscope. Everything here runs
//! on the host through `cargo test`, so it is a second, finer CI gate that does
//! not need 24 simulated circuits to produce a verdict.
//!
//! The three areas covered are the ones where the harness used to be blind:
//!
//! * the par-time drift gate itself (so a future refactor cannot quietly turn
//!   it back into a "needs recal" note),
//! * `AiRacer`'s dynamic obstacle avoidance, which had **zero** coverage -- the
//!   race loop in `game/src/main.rs` is its only real caller, every harness call
//!   site passed `&[]`, and it is the most bug-prone part of the AI,
//! * `ChampionshipSession`'s terminal stage and its full leaderboard ordering,
//!   neither of which any suite reached, plus interior-wall collision, for which
//!   no shipped circuit has a `Barrier` tile yet.

use super::{best_tuning_for, drive_track, par_drift, LAPS_PER_TRACK, MIN_TUNING_GAIN_PCT};
use arduracer_core::*;

// ---------------------------------------------------------------------------
// Synthetic tracks
// ---------------------------------------------------------------------------

/// Side of the synthetic test grids, in tiles. Large enough that the reference
/// AI cannot reach the end of its route within the tick budget used here, so
/// these tests observe the steering controller and not a U-turn at a gate.
const GRID: usize = 32;

const fn grid_with_wall_at(wx: usize, wy: usize) -> [TrackTile; GRID * GRID] {
    let mut tiles = [TrackTile::Tarmac; GRID * GRID];
    tiles[wy * GRID + wx] = TrackTile::Barrier;
    tiles
}

/// An all-tarmac grid with no walls. Used wherever a test needs a track that
/// cannot interfere with the thing under test.
static FLAT_TILES: [TrackTile; GRID * GRID] = [TrackTile::Tarmac; GRID * GRID];

/// The same grid with a single solid tile at grid (4, 3). The tile spans world
/// x in [256, 320) and y in [192, 256), so a car placed inside it can be pushed
/// at every one of the tile's faces.
static WALL_TILES: [TrackTile; GRID * GRID] = grid_with_wall_at(4, 3);

/// Where the straight test circuit starts, in grid coordinates. Its only
/// checkpoint is due north, so the desired heading is exactly 0 BAM.
const STRAIGHT_START: (u8, u8) = (16, 28);
/// The straight test circuit's only checkpoint, in grid coordinates.
const STRAIGHT_AHEAD: (u8, u8) = (16, 2);

fn gate(x: u8, y: u8) -> CheckpointGate {
    CheckpointGate {
        x,
        y,
        width: 1,
        height: 1,
    }
}

/// Builds a synthetic circuit. `checkpoints[0]` sits due north of the start, so
/// a car parked on the start tile and pointing north has a desired heading of
/// exactly 0 BAM. That matters: it makes the AI's route-following steering term
/// identically zero, so any steering the test observes comes from obstacle
/// avoidance and nothing else.
fn synthetic_track(tiles: &'static [TrackTile]) -> TrackDef {
    let mut checkpoints = [CheckpointGate::default(); MAX_TRACK_CHECKPOINTS];
    checkpoints[0] = gate(STRAIGHT_AHEAD.0, STRAIGHT_AHEAD.1);
    TrackDef {
        name: "synthetic",
        width: GRID as u8,
        height: GRID as u8,
        start_pos: TrackDef::gate_centre(&gate(STRAIGHT_START.0, STRAIGHT_START.1)),
        start_heading: 0,
        start_gate: gate(STRAIGHT_START.0, STRAIGHT_START.1),
        par_times: ParTimes {
            bronze_ticks: 900,
            silver_ticks: 800,
            gold_ticks: 700,
            dev_platinum_ticks: 600,
        },
        checkpoint_count: 1,
        checkpoints,
        tiles,
        // No authored centreline: exercises the gate-derived fallback.
        route: &[],
        half_width: 0,
    }
}

/// The synthetic north-facing straight used by the AI avoidance tests.
fn straight() -> TrackDef {
    synthetic_track(&FLAT_TILES)
}

/// A rival parked at world offset `(dx, dy)` from `at`.
fn rival_at(at: Vec2, dx: i32, dy: i32) -> Vec2 {
    Vec2::new(
        Fixed::from_int(at.x.to_int() + dx),
        Fixed::from_int(at.y.to_int() + dy),
    )
}

// ---------------------------------------------------------------------------
// Par-time drift gate
// ---------------------------------------------------------------------------

/// `par_drift` must be silent when the generated table still matches and must
/// name every field that moved. A gate that only checks `dev` (as the previous
/// `mismatched` boolean did) would miss a stale Gold/Silver/Bronze, which is
/// what actually decides the medal a player earns.
#[test]
fn par_drift_names_exactly_the_fields_that_moved() {
    let track = &ALL_TRACKS[0];
    let p = track.par_times;

    // Fed its own stored values, the gate must be silent.
    assert!(
        par_drift(
            track,
            p.dev_platinum_ticks,
            p.gold_ticks,
            p.silver_ticks,
            p.bronze_ticks
        )
        .is_none(),
        "an in-sync par table must not be reported as drift"
    );

    // One tick off Gold only: that field, and nothing else.
    let line = par_drift(
        track,
        p.dev_platinum_ticks,
        p.gold_ticks + 1,
        p.silver_ticks,
        p.bronze_ticks,
    )
    .expect("a one-tick Gold move is still drift");
    assert!(line.contains("gold"), "{line}");
    assert!(!line.contains("dev"), "{line}");
    assert!(!line.contains("silver"), "{line}");
    assert!(!line.contains("bronze"), "{line}");

    // A tracked circuit must never be un-calibratable: shifting Gold moves the
    // derived Silver/Bronze with it, and all four must be reported together.
    let all = par_drift(
        track,
        p.dev_platinum_ticks + 2,
        p.gold_ticks + 1,
        p.silver_ticks + 3,
        p.bronze_ticks + 4,
    )
    .expect("every field moved");
    for field in ["dev ", "gold ", "silver ", "bronze "] {
        assert!(all.contains(field), "missing {field} in {all}");
    }
    assert!(all.contains(&p.gold_ticks.to_string()), "{all}");
    assert!(all.contains(&(p.gold_ticks + 1).to_string()), "{all}");
}

/// A track whose par table was never generated stores `u32::MAX`. Reporting
/// that as "stale" would demand a calibration that cannot fix anything, so an
/// unset field is skipped rather than flagged.
#[test]
fn par_drift_ignores_uncalibrated_fields() {
    let mut track = straight();
    track.par_times = ParTimes {
        bronze_ticks: u32::MAX,
        silver_ticks: u32::MAX,
        gold_ticks: u32::MAX,
        dev_platinum_ticks: u32::MAX,
    };
    assert!(par_drift(&track, 700, 700, 800, 900).is_none());
}

// ---------------------------------------------------------------------------
// Tuning sweep
// ---------------------------------------------------------------------------

/// The sweep's whole justification is that the Garage changes pace. Every call
/// site of `best_tuning_for` used to take the winner on trust, so a vehicle
/// model that dropped the tuning sliders -- or six presets that all tied --
/// would have produced a perfectly green run.
#[test]
fn the_best_swept_tuning_beats_the_default_tune_by_the_required_margin() {
    for (idx, track) in ALL_TRACKS.iter().enumerate() {
        let default_run = drive_track(track, CarTuning::default());
        assert_eq!(
            default_run.laps_completed, LAPS_PER_TRACK,
            "{}: default tune did not complete the reference laps",
            track.name
        );
        let (_, best) = best_tuning_for(track, default_run.best_ticks);
        assert!(
            best as u64 * 100 <= default_run.best_ticks as u64 * (100 - MIN_TUNING_GAIN_PCT) as u64,
            "{}: best swept tuning {} ticks is not >= {}% faster than the default tune {} ticks",
            track.name,
            best,
            MIN_TUNING_GAIN_PCT,
            default_run.best_ticks
        );
        assert!(
            best != default_run.best_ticks,
            "{} (track {}): tuning made no difference at all",
            track.name,
            idx + 1
        );
    }
}

// ---------------------------------------------------------------------------
// AI obstacle avoidance
// ---------------------------------------------------------------------------

/// Mirrors `ai.rs`. Avoidance no longer nudges the heading by a fixed BAM count
/// per rival -- it shifts the aim point sideways by a fixed world offset in the
/// car's perpendicular frame, and the proportional steering gain then turns the
/// resulting heading error into a command. Quantities that still exist:
const AVOIDANCE_TILES: i32 = 1;
/// Perpendicular aim-point shift applied per rival in the near field.
const AVOIDANCE_OFFSET_UNITS: i32 = TILE_SIZE;
/// Distance beyond which a rival is ignored entirely.
const AVOIDANCE_RANGE_UNITS: i32 = AVOIDANCE_TILES * TILE_SIZE * 2;
/// The near-field test is a box half this wide in each axis.
const AVOID_HALF: i32 = AVOIDANCE_TILES * TILE_SIZE;

fn ai_on_the_straight() -> AiRacer {
    AiRacer::new(
        Vec2::new(Fixed::from_int(1056), Fixed::from_int(1824)),
        0,
        AI_PROFILES[0],
    )
}

/// A rival standing next to the AI pushes the steering command *away* from it,
/// by a known and constant amount. This is the assertion that was missing
/// everywhere: no suite had ever put a car in the rivals array at all.
#[test]
fn a_rival_beside_the_path_steers_the_ai_away_from_it() {
    let track = straight();
    let mut ai = ai_on_the_straight();
    let here = ai.state.position;

    // Sanity: pointing exactly at the only route node, there is no route
    // steering term to hide an avoidance response behind.
    let clear = ai.compute_input(&track, &[]).steer;
    assert_eq!(
        clear,
        Fixed::ZERO,
        "the synthetic straight must not contribute steering of its own"
    );

    // Heading 0 puts +X on the AI's right. A rival to the right must steer it
    // left (negative), a rival to the left must steer it right (positive).
    // The magnitude is deliberately not pinned: it is the product of a gain and
    // an aim-point offset, both of which are free to be retuned.
    for (dx, label) in [(8, "right"), (-8, "left")] {
        let rivals = [rival_at(here, dx, -4)];
        let steer = ai.compute_input(&track, &rivals).steer;
        assert!(
            steer.raw() * dx.signum() < 0,
            "a rival on the {label} produced steer {} raw, which does not steer away from it",
            steer.raw()
        );
    }

    // And the response is a real response, not a rounding-scale nudge.
    let rivals = [rival_at(here, 8, -4)];
    assert!(
        ai.compute_input(&track, &rivals).steer.abs().raw() >= FP_ONE / 8,
        "avoidance must produce a meaningful steering command, got {}",
        ai.compute_input(&track, &rivals).steer.raw()
    );
}

/// The dodge is per-rival and additive, and it saturates. Two rivals on the same
/// side must ask for a bigger correction than one, and a dense pack must be
/// clamped rather than winding the steering into a spin.
#[test]
fn avoidance_accumulates_across_a_pack_and_saturates() {
    let track = straight();
    let mut ai = ai_on_the_straight();
    let here = ai.state.position;

    let one = ai
        .compute_input(&track, &[rival_at(here, 8, -4)])
        .steer
        .raw();
    let two = ai
        .compute_input(&track, &[rival_at(here, 8, -4), rival_at(here, 8, 12)])
        .steer
        .raw();
    let three = ai
        .compute_input(
            &track,
            &[
                rival_at(here, 8, -12),
                rival_at(here, 8, 0),
                rival_at(here, 8, 12),
            ],
        )
        .steer
        .raw();

    assert!(one < 0, "one rival on the right must steer left, got {one}");
    // Each rival shifts the aim point by one tile perpendicular to travel, and
    // the total is clamped at two tiles -- so the second rival still adds
    // correction and the third saturates rather than winding further.
    assert!(
        two < one,
        "a second rival on the same side must add correction: {one} -> {two}"
    );
    assert!(
        two <= three,
        "a third rival must not reduce the correction: {two} -> {three}"
    );
    assert!(
        three.abs() <= FP_ONE && two.abs() <= FP_ONE,
        "a pack must not steer past full lock, got {three}"
    );
    assert_eq!(
        AVOIDANCE_OFFSET_UNITS, TILE_SIZE,
        "a single rival dodges by exactly one tile"
    );
    assert!(
        three > -FP_ONE / 2,
        "the clamp must leave usable steering authority, got {three}"
    );

    // The mirror-image pack steers the other way, so the response is a
    // difference and not a constant offset.
    let left = ai
        .compute_input(&track, &[rival_at(here, -8, 0)])
        .steer
        .raw();
    assert!(left > 0, "a rival on the left must steer right, got {left}");
}

/// The near-field box is the only thing that triggers avoidance. A rival beyond
/// the detection range, or inside the range but outside the box, must leave the
/// steering command untouched -- otherwise every rival on the circuit perturbs
/// the AI regardless of where it is. The box is sized in tiles, not in raw
/// units: the old 14x20 unit box was smaller than a single 64-unit tile, so
/// rivals drove through each other.
#[test]
fn avoidance_ignores_rivals_outside_the_near_field_box() {
    let track = straight();
    let mut ai = ai_on_the_straight();
    let here = ai.state.position;
    let clear = ai.compute_input(&track, &[]).steer;

    // Every case is inside the detection range (so the range check cannot be
    // what rejects it) but outside the box on at least one axis, except the
    // first which is past the range entirely.
    let ignored: [(&str, i32, i32); 4] = [
        ("beyond range", AVOIDANCE_RANGE_UNITS + 1, 0),
        ("outside the box laterally", 0, AVOID_HALF),
        ("outside the box longitudinally", AVOID_HALF, 0),
        ("diagonally clear", AVOID_HALF, AVOID_HALF),
    ];
    for (label, dx, dy) in ignored {
        let rivals = [rival_at(here, dx, dy)];
        assert_eq!(
            ai.compute_input(&track, &rivals).steer,
            clear,
            "a rival {} (+{}x, +{}y) must not steer the AI",
            label,
            dx,
            dy
        );
    }

    // ...and just inside the boundary it does steer, so the cases above are not
    // passing because the whole mechanism is dead.
    let near = [rival_at(here, AVOID_HALF - 1, AVOID_HALF - 1)];
    assert_ne!(
        ai.compute_input(&track, &near).steer,
        clear,
        "a rival just inside the box must steer the AI"
    );
}

/// A rival occupying the exact same position is skipped by the near-field box
/// (`dist_sq == 0`), so the grid spawn does not make every rival swerve on lap
/// one. Locked down here because it is a deliberate `continue`, not an accident.
#[test]
fn avoidance_ignores_a_rival_at_the_exact_same_position() {
    let track = straight();
    let mut ai = ai_on_the_straight();
    let here = ai.state.position;
    assert_eq!(
        ai.compute_input(&track, &[here]).steer,
        ai.compute_input(&track, &[]).steer,
        "a zero-distance rival must not steer the AI"
    );
}

/// End-to-end: a rival parked in the lane makes the AI leave the lane rather
/// than hold its line through the obstacle. The rivals array is what the race
/// loop passes, so this is the shape the game actually exercises.
#[test]
fn a_rival_avoids_a_car_parked_in_its_path() {
    let track = straight();
    let blocker = rival_at(
        Vec2::new(Fixed::from_int(1056), Fixed::from_int(1824)),
        0,
        -18,
    );

    let mut blocked = ai_on_the_straight();
    let mut unblocked = ai_on_the_straight();
    for _ in 0..90 {
        blocked.tick(&track, &[blocker]);
        unblocked.tick(&track, &[]);
    }

    // The parked car sits at ox = 0, which the box resolves as "+X side", so the
    // AI is pushed toward -X. Without the parked car it drives straight.
    assert!(
        blocked.state.position.x < unblocked.state.position.x - Fixed::from_int(8),
        "AI at {:?} did not pull clear of a parked car at {:?} (baseline {:?})",
        blocked.state.position,
        blocker,
        unblocked.state.position
    );
    assert!(
        blocked.state.position.y < blocker.y,
        "AI at {:?} overran the parked car at {:?} instead of steering around it",
        blocked.state.position,
        blocker
    );
    // It must still be racing: avoidance is a nudge, not a stop.
    assert!(
        blocked.state.speed > Fixed::ZERO,
        "avoiding a parked car must not bring the rival to a standstill"
    );
}

// ---------------------------------------------------------------------------
// Championship progression
// ---------------------------------------------------------------------------

/// The suites only ever awarded a single stage of six and asserted `!finished`,
/// so the terminal case -- the branch that actually ends a cup -- was never
/// reached. Walk the whole Bronze Cup and check every stage boundary.
#[test]
fn a_championship_reaches_its_terminal_stage() {
    let mut session = ChampionshipSession::new(0);
    assert_eq!(session.current_stage, 0);
    assert_eq!(session.current_track_idx(), 0);
    assert_eq!(session.competitors.len(), 6);

    for stage in 0..6u8 {
        assert_eq!(session.current_stage, stage, "stage did not advance");
        assert_eq!(session.current_track_idx(), stage as usize);
        // Player wins every stage; the order is irrelevant to progression.
        session.award_stage_points([0, 1, 2, 3, 4, 5]);
        let finished = session.advance_stage();
        assert_eq!(
            finished,
            stage == 5,
            "stage {} reported finished={finished}",
            stage
        );
    }

    // Terminal state: `advance_stage` clamps `current_stage` at the last stage
    // rather than letting it run past the end of the cup. Unclamped it reached
    // 6, 7, 8... and `current_track_idx()` -- `(cup_index * 6) + current_stage`
    // -- indexed `ALL_TRACKS` out of bounds for the final cup.
    assert_eq!(
        session.current_stage, 5,
        "the stage counter must clamp at the last stage"
    );
    assert!(
        session.advance_stage(),
        "advancing past the cup must stay terminal"
    );
    assert_eq!(session.current_stage, 5, "still terminal, still clamped");
    assert!(
        session.current_track_idx() < TOTAL_TRACKS,
        "a terminal session must still name a real track, got {}",
        session.current_track_idx()
    );

    // Six wins = six firsts at 10 points.
    assert_eq!(session.competitors[0].total_points, 60);
    assert_eq!(session.competitors[1].total_points, 36);
}

/// Every (cup, stage) pair must map onto a real circuit. `current_track_idx` is
/// plain arithmetic with no bounds check, so a fourth cup or a seventh stage
/// would index `ALL_TRACKS` out of range at runtime.
#[test]
fn every_championship_stage_maps_onto_a_shipped_circuit() {
    assert_eq!(ALL_TRACKS.len(), 24);
    for cup in 0..4u8 {
        for stage in 0..6u8 {
            let session = ChampionshipSession::new(cup);
            assert_eq!(session.cup_index, cup);
            let idx = session.current_track_idx() + stage as usize;
            assert!(
                idx < ALL_TRACKS.len(),
                "cup {cup} stage {stage} maps to circuit {} of {}",
                idx,
                ALL_TRACKS.len()
            );
        }
    }
    // Out-of-range cups are clamped, not multiplied into a bad index.
    assert_eq!(ChampionshipSession::new(9).cup_index, 3);
}

/// Only index 0 of the sorted leaderboard was ever asserted. The whole ordering
/// -- and its stability under ties, which decides tie-break presentation -- is
/// checked here.
#[test]
fn the_leaderboard_orders_every_competitor() {
    let mut session = ChampionshipSession::new(0);
    // A stage won by the player, then one won by rival 4: totals 16 / 6 / 4 /
    // 3 / 12 / 1, i.e. PLAYER, RIVAL4, RIVAL0, RIVAL1, RIVAL2, RIVAL3.
    session.award_stage_points([0, 1, 2, 3, 4, 5]);
    session.award_stage_points([4, 0, 1, 2, 3, 5]);

    let board = session.sorted_leaderboard();
    let expected_points = [16u16, 12, 10, 7, 5, 2];
    let expected_names = [
        "PLAYER",
        AI_PROFILES[3].name,
        AI_PROFILES[0].name,
        AI_PROFILES[1].name,
        AI_PROFILES[2].name,
        AI_PROFILES[4].name,
    ];
    for (place, ((row, points), name)) in board
        .iter()
        .zip(expected_points)
        .zip(expected_names)
        .enumerate()
    {
        assert_eq!(row.total_points, points, "place {} points", place + 1);
        assert_eq!(row.name, name, "place {}", place + 1);
    }

    // Every competitor appears exactly once.
    let mut sorted_names: Vec<&str> = board.iter().map(|c| c.name).collect();
    sorted_names.sort_unstable();
    sorted_names.dedup();
    assert_eq!(
        sorted_names.len(),
        6,
        "a competitor is missing or duplicated"
    );

    // Totals are monotonically non-increasing down the board.
    for place in 1..board.len() {
        assert!(
            board[place - 1].total_points >= board[place].total_points,
            "place {} has fewer points than place {}",
            place,
            place + 1
        );
    }

    // A fresh session is all ties; the sort is stable, so registration order
    // (player first) must survive rather than shuffling the grid.
    let fresh = ChampionshipSession::new(1).sorted_leaderboard();
    assert_eq!(fresh[0].name, "PLAYER");
    assert!(fresh[0].is_player);
    assert_eq!(
        fresh.iter().map(|c| c.name).collect::<Vec<_>>(),
        vec![
            "PLAYER",
            AI_PROFILES[0].name,
            AI_PROFILES[1].name,
            AI_PROFILES[2].name,
            AI_PROFILES[3].name,
            AI_PROFILES[4].name,
        ]
    );
}

// ---------------------------------------------------------------------------
// Interior-wall collision
// ---------------------------------------------------------------------------

/// `VehicleState::collide_with_track` must get a car out of a solid tile no
/// matter where in the tile it ends up. No shipped circuit has a `Barrier` tile
/// yet (the wall geometry is still being authored), so this builds one; without
/// it the interior-solid branch has no coverage at all.
///
/// The exit face is chosen by minimum translation -- the cheapest of the four
/// faces that actually lands the car on a drivable tile -- so the test asserts
/// the *invariant* rather than one particular face. A car driving into the wall
/// at any angle must (a) finish the tick on a non-solid tile, and (b) never be
/// left with velocity pointing back into solid geometry. Pinning one face here
/// would just re-assert whichever face the MTV heuristic happens to prefer.
#[test]
fn a_car_inside_an_interior_wall_is_always_ejected_onto_drivable_tiles() {
    let track = synthetic_track(&WALL_TILES);
    let (wx, wy) = (4u8, 3u8);

    let offsets: [(i32, i32); 5] = [(1, 1), (10, 5), (32, 32), (60, 60), (62, 3)];
    let headings: [(i32, i32); 8] = [
        (2, 0),
        (-2, 0),
        (0, 2),
        (0, -2),
        (1, 1),
        (-1, 1),
        (1, -1),
        (-1, -1),
    ];

    for (local_x, local_y) in offsets {
        for (vx, vy) in headings {
            let pos = Vec2::new(
                Fixed::from_int(wx as i32 * TILE_SIZE + local_x),
                Fixed::from_int(wy as i32 * TILE_SIZE + local_y),
            );
            let mut car = VehicleState::new(pos, 0, CarTuning::default());
            car.velocity = Vec2::new(Fixed::from_int(vx), Fixed::from_int(vy));
            car.speed = car.velocity.length();

            assert!(
                track.tile_at(wx, wy).is_solid(),
                "the wall tile must be solid"
            );
            assert!(
                car.collide_with_track(&track),
                "({}, {}) entering ({vx},{vy}) did not report a hit",
                local_x,
                local_y
            );

            let out_x = TrackDef::tile_x_of(car.position.x);
            let out_y = TrackDef::tile_y_of(car.position.y);
            assert!(
                !track.tile_at(out_x, out_y).is_solid(),
                "({local_x},{local_y}) entering ({vx},{vy}) was left inside a wall at {:?}",
                car.position
            );
            assert_eq!(
                car.speed,
                car.velocity.length(),
                "({local_x},{local_y}) entering ({vx},{vy}): speed not synced"
            );

            // Whatever face it left by, de-penetration must converge: keep
            // resolving collisions and the car has to end up outside the wall
            // and stay there. A tangential exit keeps its momentum and slides
            // clear, which is why "velocity points away from the wall" is not
            // the property worth pinning.
            let mut escaped_at = None;
            for tick in 0..16u32 {
                if !track
                    .tile_at(
                        TrackDef::tile_x_of(car.position.x),
                        TrackDef::tile_y_of(car.position.y),
                    )
                    .is_solid()
                {
                    escaped_at = Some(tick);
                    break;
                }
                car.collide_with_track(&track);
            }
            assert!(
                escaped_at.is_some(),
                "({local_x},{local_y}) entering ({vx},{vy}) never escaped the wall, ended at {:?}",
                car.position
            );
        }
    }
}

/// The tile the car was embedded in reads as the barrier, so the vehicle model
/// gives it no traction at all. Checked against the tile that was entered --
/// checking the car's own tile would race the ejection.
#[test]
fn a_solid_interior_tile_has_no_traction() {
    let track = synthetic_track(&WALL_TILES);
    assert_eq!(track.surface_at(4, 3), SurfaceType::Barrier);
    assert_eq!(track.surface_at(4, 3).traction(), Fixed::ZERO);

    // And a car that is not moving into the wall is left alone -- sliding along
    // a barrier must not be turned into a bounce.
    let pos = Vec2::new(
        Fixed::from_int(4 * TILE_SIZE + 10),
        Fixed::from_int(3 * TILE_SIZE + 5),
    );
    let mut car = VehicleState::new(pos, 0, CarTuning::default());
    car.velocity = Vec2::new(Fixed::from_int(-2), Fixed::ZERO);
    car.speed = car.velocity.length();
    car.collide_with_track(&track);
    assert_eq!(car.velocity, Vec2::new(Fixed::from_int(-2), Fixed::ZERO));
}

/// The synthetic wall grid must actually be different from the flat one, or
/// every interior-wall assertion above would be testing nothing.
#[test]
fn the_synthetic_grids_differ_only_by_the_wall() {
    assert_eq!(FLAT_TILES.len(), GRID * GRID);
    assert_eq!(WALL_TILES.len(), GRID * GRID);
    let solid = WALL_TILES.iter().filter(|t| t.is_solid()).count();
    assert_eq!(solid, 1, "the wall grid must have exactly one solid tile");
    assert_eq!(
        FLAT_TILES.iter().filter(|t| t.is_solid()).count(),
        0,
        "the flat grid must have no solid tiles"
    );
}
