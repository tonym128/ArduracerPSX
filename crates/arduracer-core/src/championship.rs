//! Championship Cups, Stages, and Points Leaderboard.
//!
//! Manages 4 Grand Prix Cups across all 24 tracks with arcade points scoring.

use crate::ai::AiRacer;
use crate::ai_profiles::AI_PROFILES;
use crate::math::Vec2;
use crate::save::TOTAL_TRACKS;
use crate::timing::TOTAL_LAPS;
use crate::track::TrackDef;

pub const POINTS_TABLE: [u8; 6] = [10, 6, 4, 3, 2, 1];

/// Competitors in a championship: the player plus five rivals.
pub const COMPETITORS_PER_SESSION: usize = 6;
/// Races in each cup.
pub const STAGES_PER_CUP: u8 = 6;
/// Cups in the championship.
pub const CUP_COUNT: usize = 4;
/// Weight of one completed lap in the standings score, in Q20.12 raw units.
/// Larger than [`STANDINGS_DISTANCE_CAP`] so a car a whole lap *behind* can never
/// out-score one that is nearer to its next gate.
const STANDINGS_LAP_WEIGHT: i64 = 1 << 30;
/// Weight of having taken the flag at all.
const STANDINGS_FINISH_WEIGHT: i64 = 1 << 40;
/// Largest distance to the next gate the positional term can represent, in Q20.12
/// raw units (2^24 raw == 4096 world units, twice the width of the biggest
/// circuit).
///
/// The score compares `lap * STANDINGS_LAP_WEIGHT - distance`, so as long as the
/// clamped distance is well below the lap weight, "further round the circuit"
/// always loses to "another lap further on" -- which is what a race position
/// means. Total: with 6 competitors the worst score is under 2^44.
const STANDINGS_DISTANCE_CAP: i64 = 1 << 24;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Competitor {
    pub name: &'static str,
    pub color: (u8, u8, u8),
    pub total_points: u16,
    pub is_player: bool,
}

pub struct ChampionshipSession {
    pub cup_index: u8,     // 0..3 (Bronze, Silver, Gold, Platinum)
    pub current_stage: u8, // 0..5 (6 stages per cup)
    pub competitors: [Competitor; COMPETITORS_PER_SESSION],
}

impl ChampionshipSession {
    pub fn new(cup_index: u8) -> Self {
        let competitors = [
            Competitor {
                name: "PLAYER",
                color: (220, 25, 45), // Crimson Red
                total_points: 0,
                is_player: true,
            },
            Competitor {
                name: AI_PROFILES[0].name,
                color: AI_PROFILES[0].color,
                total_points: 0,
                is_player: false,
            },
            Competitor {
                name: AI_PROFILES[1].name,
                color: AI_PROFILES[1].color,
                total_points: 0,
                is_player: false,
            },
            Competitor {
                name: AI_PROFILES[2].name,
                color: AI_PROFILES[2].color,
                total_points: 0,
                is_player: false,
            },
            Competitor {
                name: AI_PROFILES[3].name,
                color: AI_PROFILES[3].color,
                total_points: 0,
                is_player: false,
            },
            Competitor {
                name: AI_PROFILES[4].name,
                color: AI_PROFILES[4].color,
                total_points: 0,
                is_player: false,
            },
        ];

        ChampionshipSession {
            cup_index: cup_index.min(CUP_COUNT as u8 - 1),
            current_stage: 0,
            competitors,
        }
    }

    /// Returns the global track index (0..TOTAL_TRACKS-1) for the current stage.
    ///
    /// `cup_index` is clamped by `new`, and `current_stage` is kept inside
    /// `0..STAGES_PER_CUP` by [`ChampionshipSession::advance_stage`], so this
    /// can never index past `ALL_TRACKS`. The `min` is belt-and-braces for a
    /// hand-built session: `TOTAL_TRACKS` is a hard data limit, and a race
    /// screen that indexes `ALL_TRACKS[current_track_idx()]` would otherwise be
    /// an out-of-bounds read.
    pub fn current_track_idx(&self) -> usize {
        let raw =
            (self.cup_index as usize * STAGES_PER_CUP as usize) + (self.current_stage as usize);
        raw.min(TOTAL_TRACKS - 1)
    }

    /// Awards points according to finish positions.
    ///
    /// `finish_indices[place]` is the competitor index finishing in that place.
    /// Contract:
    ///
    /// * An index `>= COMPETITORS_PER_SESSION` is ignored (no such competitor).
    /// * A **repeated** index is awarded only for its first place. The old code
    ///   range-checked the index but never checked uniqueness, so
    ///   `[0, 0, 1, 2, 3, 4]` paid competitor 0 both 1st and 2nd -- 16 points in
    ///   a single race.
    pub fn award_stage_points(&mut self, finish_indices: [usize; COMPETITORS_PER_SESSION]) {
        let mut scored: u8 = 0;
        for (place, &comp_idx) in finish_indices.iter().enumerate() {
            if comp_idx >= COMPETITORS_PER_SESSION {
                continue;
            }
            let bit = 1u8 << comp_idx;
            if scored & bit != 0 {
                continue;
            }
            scored |= bit;
            let pts = POINTS_TABLE[place.min(POINTS_TABLE.len() - 1)];
            self.competitors[comp_idx].total_points += pts as u16;
        }
    }

    /// Advances to the next stage in the cup. Returns true if championship finished.
    ///
    /// `current_stage` is clamped to `STAGES_PER_CUP - 1` so
    /// [`ChampionshipSession::current_track_idx`] can never run past the end of a
    /// cup: the old `current_stage += 1` left it at 6 (and 7, 8, ...) on
    /// repeated calls, which resolved to track 24+ for the Platinum cup against a
    /// 24-track `ALL_TRACKS`.
    pub fn advance_stage(&mut self) -> bool {
        let next = self.current_stage.saturating_add(1);
        if next >= STAGES_PER_CUP {
            self.current_stage = STAGES_PER_CUP - 1;
            true
        } else {
            self.current_stage = next;
            false
        }
    }

    /// Returns the competitors sorted by total_points descending.
    ///
    /// Stable: equal points keep grid order, so the leaderboard never flickers
    /// between two rivals on the same score.
    pub fn sorted_leaderboard(&self) -> [Competitor; COMPETITORS_PER_SESSION] {
        let mut board = self.competitors;
        for i in 1..COMPETITORS_PER_SESSION {
            let mut j = i;
            while j > 0 && board[j].total_points > board[j - 1].total_points {
                board.swap(j, j - 1);
                j -= 1;
            }
        }
        board
    }
}

/// Where a competitor is trying to get to next, and how far away it is.
///
/// The `route_node_index` used for `player_gate` and [`AiRacer::target_gate_idx`]
/// is a **next-gate index**: the index of the route node the car has not reached
/// yet, in `0 .. track.route_len()`. In particular `track.route_node(n)` -- where
/// `n == track.checkpoint_count()` -- is the start/finish line itself, not
/// `checkpoints[0]`. The old code reduced it with `% gate_count` and so measured a
/// car sitting on the line against the gate at the far side of the circuit (640
/// units away on TRACK_01).
fn gate_progress_score(track: &TrackDef, pos: Vec2, next_gate: usize) -> i64 {
    let gate = track.route_node(next_gate);
    let target = TrackDef::gate_centre(&gate);
    // `length()` is a Q20.12 sqrt of a 64-bit accumulator, so the largest
    // separation any circuit can produce (1920 units) is exact and saturates
    // rather than wrapping.
    let distance = (target - pos).length().raw() as i64;
    distance.clamp(0, STANDINGS_DISTANCE_CAP)
}

/// Computes the race leaderboard / standings.
///
/// Returns an array of competitor indices where `result[0]` is 1st place,
/// `result[1]` is 2nd, and so on. Index `0` is the human player, indices
/// `1..=5` are `rivals[0..5]`.
///
/// # Contract
///
/// * `player_lap` / [`AiRacer::current_lap`] are *completed + 1* laps: the
///   player is on lap 1 while running the opening lap. Anything derived from
///   them (the lap timer, the rival counter) must agree or the two fields are
///   not comparable.
/// * `player_gate` / [`AiRacer::target_gate_idx`] are **next**-gate indices in
///   `0 ..= track.route_len() - 1`, wrapping at the start/finish. See
///   [`gate_progress_score`].
/// * `rivals` may be **any length**, including empty and including more than
///   [`COMPETITORS_PER_SESSION`] - 1. The result is always a permutation of
///   `0..COMPETITORS_PER_SESSION`: competitors that were not supplied rank last,
///   in ascending index order, so the caller can index the array
///   unconditionally. The old code indexed `rivals[i - 1]` for `i` in `1..6` and
///   panicked with "index out of bounds: the len is 1 but the index is 1" for a
///   one-rival race.
/// * Deterministic and stable: the sort breaks ties on competitor index, so the
///   same inputs always produce the same order.
///
/// # Ranking
///
/// Taken flag, then laps completed, then distance to the next gate, then index.
/// The old score was `finish + lap * 10_000_000 + gate_idx * 100_000 -
/// min(dist_sq, 99_999)`, which ranked on the raw gate *number* rather than on
/// where the car actually is: a larger index always won, and because the
/// positional term was clamped to 99_999 it could never outweigh a single gate
/// step. With all six cars on the start tile it reported `standings =
/// [1, 2, 3, 4, 5, 0]`, i.e. the player in 6th while leading.
pub fn compute_standings(
    player_pos: Vec2,
    player_lap: u8,
    player_gate: u8,
    player_finished: bool,
    rivals: &[AiRacer],
    track: &TrackDef,
) -> [usize; COMPETITORS_PER_SESSION] {
    let mut scores = [(0i64, 0usize); COMPETITORS_PER_SESSION];

    let consider = |pos: Vec2, lap: u8, gate: usize, finished: bool| -> i64 {
        let progress = gate_progress_score(track, pos, gate);
        (if finished { STANDINGS_FINISH_WEIGHT } else { 0 })
            + (finishing_lap(lap, finished) as i64) * STANDINGS_LAP_WEIGHT
            - progress
    };

    scores[0] = (
        consider(
            player_pos,
            player_lap,
            player_gate as usize,
            player_finished,
        ),
        0,
    );
    let mut entrants = 1usize;

    for (i, rival) in rivals.iter().take(COMPETITORS_PER_SESSION - 1).enumerate() {
        let idx = i + 1;
        scores[idx] = (
            consider(
                rival.state.position,
                rival.current_lap,
                rival.target_gate_idx as usize,
                rival.is_finished,
            ),
            idx,
        );
        entrants += 1;
    }

    // Descending by score; ties broken by ascending competitor index, so the
    // order is total and reproducible. The comparator is written out rather than
    // comparing the `(score, index)` tuples directly: tuple ordering puts the
    // *larger* index first on a tie, which reversed the whole field.
    let ranks_before = |a: (i64, usize), b: (i64, usize)| b.0 > a.0 || (b.0 == a.0 && b.1 < a.1);
    for i in 1..entrants {
        let mut j = i;
        while j > 0 && ranks_before(scores[j - 1], scores[j]) {
            scores.swap(j, j - 1);
            j -= 1;
        }
    }

    let mut standings = [0usize; COMPETITORS_PER_SESSION];
    for (place, slot) in standings.iter_mut().enumerate() {
        *slot = if place < entrants {
            scores[place].1
        } else {
            // Not racing: park them at the back, in index order.
            place
        };
    }
    standings
}

/// Lap count a finished race settles on, shared by the player and the rivals.
///
/// A rival is flagged finished once `current_lap` passes
/// [`crate::timing::TOTAL_LAPS`], while the lap timer holds at
/// `TOTAL_LAPS` and only sets `is_finished`. Normalising both here keeps a
/// car that took the flag ahead of one still on the final lap instead of
/// handing the finish to whoever happened to count one higher.
pub fn finishing_lap(lap: u8, finished: bool) -> u8 {
    if finished && lap > TOTAL_LAPS {
        TOTAL_LAPS
    } else {
        lap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_profiles::AI_PROFILES;
    use crate::levels::{ALL_TRACKS, ALL_TRACK_VISUALS, AUTHORED_TRACKS};
    use crate::math::Fixed;

    fn rivals_on(
        track: &TrackDef,
        count: usize,
        mutate: impl Fn(usize, &mut AiRacer),
    ) -> Vec<AiRacer> {
        (0..count)
            .map(|i| {
                let profile = AI_PROFILES[i % AI_PROFILES.len()];
                let _ = i;
                let mut racer = AiRacer::new(track.start_pos, track.start_heading, profile);
                racer.offset_from_pole(track.start_pos, track.start_heading, 0, 0);
                mutate(i, &mut racer);
                racer
            })
            .collect()
    }

    // ========================================================================
    // bug 3: fewer than five rivals
    // ========================================================================

    #[test]
    fn compute_standings_survives_any_rival_count() {
        // Reproduced: one rival panicked with "index out of bounds: the len is 1
        // but the index is 1" because `rivals[i - 1]` ran for `i` in `1..6`.
        let track = ALL_TRACKS[0];
        for count in 0..12usize {
            let rivals = rivals_on(track, count, |_, _| {});
            let standings = compute_standings(track.start_pos, 1, 0, false, &rivals, track);
            let mut seen = [false; COMPETITORS_PER_SESSION];
            for &competitor in standings.iter() {
                assert!(
                    competitor < COMPETITORS_PER_SESSION,
                    "{count} rivals produced out-of-range index {competitor}"
                );
                assert!(!seen[competitor], "{count} rivals repeated an index");
                seen[competitor] = true;
            }
        }
    }

    #[test]
    fn absent_rivals_are_ranked_last_in_index_order() {
        let track = ALL_TRACKS[0];
        let rivals = rivals_on(track, 2, |_, _| {});
        let standings = compute_standings(track.start_pos, 1, 0, false, &rivals, track);
        // Six cars on the grid, all level: index order decides, player first.
        assert_eq!(standings, [0, 1, 2, 3, 4, 5]);
    }

    // ========================================================================
    // bug 2: rank on real race progress
    // ========================================================================

    #[test]
    fn six_cars_on_the_grid_do_not_put_the_player_in_sixth() {
        // Reproduced: with all six cars on the start tile the old code returned
        // `standings = [1, 2, 3, 4, 5, 0]`, showing the player 6th while leading.
        let track = ALL_TRACKS[0];
        let rivals = rivals_on(track, 5, |_, _| {});
        let standings = compute_standings(track.start_pos, 1, 0, false, &rivals, track);
        assert_eq!(
            standings[0], 0,
            "the player on the grid must be listed first"
        );
        assert_eq!(standings, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_car_with_a_larger_gate_index_is_not_automatically_ahead() {
        // The old score awarded `gate_idx * 100_000` for the raw number while the
        // positional term was clamped to 99_999, so one more gate always beat any
        // amount of being closer to yours.
        let track = ALL_TRACKS[0];
        let n = track.route_len();
        // Player is right on gate 1; rival has run on to gate 2 but is 600 units
        // back down the road from it.
        let gate1 = TrackDef::gate_centre(&track.route_node(1));
        let gate2 = TrackDef::gate_centre(&track.route_node(2 % n));
        let player_pos = gate1;
        let rivals = rivals_on(track, 5, |i, racer| {
            if i == 0 {
                racer.state.position = gate2;
                racer.target_gate_idx = 2;
                racer.current_lap = 1;
            } else {
                racer.state.position = gate1;
                racer.target_gate_idx = 1;
                racer.current_lap = 1;
            }
        });
        let standings = compute_standings(player_pos, 1, 1, false, &rivals, track);
        let rival0_place = standings.iter().position(|&c| c == 1).unwrap_or(99);
        let player_place = standings.iter().position(|&c| c == 0).unwrap_or(99);
        assert!(
            player_place < rival0_place,
            "player ranked {player_place}, rival one gate further on ranked {rival0_place}"
        );
    }

    #[test]
    fn being_physically_closer_to_your_next_gate_wins() {
        let track = ALL_TRACKS[0];
        let gate = track.route_node(2);
        let centre = TrackDef::gate_centre(&gate);
        let rivals = rivals_on(track, 5, |i, racer| {
            racer.current_lap = 2;
            racer.target_gate_idx = 2;
            racer.state.position = if i == 0 {
                centre
            } else {
                centre + Vec2::new(Fixed::from_int(120), Fixed::from_int(120))
            };
        });
        // Player is lapped but right on the gate; every rival is on the same lap.
        let standings = compute_standings(centre, 2, 2, false, &rivals, track);
        assert_eq!(standings[0], 0, "the player on the gate must lead the lap");
    }

    #[test]
    fn a_lap_always_beats_any_distance_within_a_lap() {
        // The lap weight has to dominate the clamped positional term, or a car a
        // whole lap behind could out-score one on the road.
        let track = ALL_TRACKS[0];
        let n = track.route_len();
        let far = TrackDef::gate_centre(&track.route_node(n - 1));
        let rivals = rivals_on(track, 5, |i, racer| {
            racer.current_lap = if i == 0 { 1 } else { 2 };
            racer.target_gate_idx = if i == 0 { n as u8 } else { 0 };
            racer.state.position = far;
        });
        // Player on lap 2 but sitting on the last gate; rival still on lap 1.
        let standings = compute_standings(far, 2, (n - 1) as u8, false, &rivals, track);
        let player_place = standings.iter().position(|&c| c == 0).unwrap_or(99);
        let rival_place = standings.iter().position(|&c| c == 1).unwrap_or(99);
        assert!(
            player_place < rival_place,
            "player on lap 2 ranked {player_place}, rival on lap 1 ranked {rival_place}"
        );
    }

    #[test]
    fn the_start_finish_line_is_measured_against_the_line() {
        // `target_gate_idx == checkpoint_count` is the start/finish route node.
        // The old `% gate_count` mapped it onto `checkpoints[0]`, 640 units away on
        // TRACK_01, so a rival standing on the line ranked behind one 640 units
        // back at gate 0.
        let track = ALL_TRACKS[0];
        assert_eq!(track.checkpoint_count as usize + 1, track.route_len());
        let on_the_line = Vec2::new(
            Fixed::from_int(
                track.start_gate.x as i32 * crate::track::TILE_SIZE
                    + track.start_gate.width as i32 * crate::track::TILE_SIZE / 2,
            ),
            Fixed::from_int(
                track.start_gate.y as i32 * crate::track::TILE_SIZE
                    + track.start_gate.height as i32 * crate::track::TILE_SIZE / 2,
            ),
        );
        let rivals = rivals_on(track, 5, |i, racer| {
            racer.current_lap = 2;
            racer.target_gate_idx = track.checkpoint_count;
            racer.state.position = if i == 0 {
                on_the_line
            } else {
                on_the_line + Vec2::new(Fixed::from_int(400), Fixed::from_int(0))
            };
        });
        let standings = compute_standings(
            on_the_line + Vec2::new(Fixed::from_int(400), Fixed::from_int(0)),
            2,
            track.checkpoint_count,
            false,
            &rivals,
            track,
        );
        let rival_place = standings.iter().position(|&c| c == 1).unwrap_or(99);
        let player_place = standings.iter().position(|&c| c == 0).unwrap_or(99);
        assert!(
            rival_place < player_place,
            "the car on the line (place {rival_place}) must lead the one 400 units back (place {player_place})"
        );
    }

    #[test]
    fn taking_the_flag_beats_everything_else() {
        let track = ALL_TRACKS[0];
        // The player has taken the flag; every rival is still running.
        let rivals = rivals_on(track, 5, |i, racer| {
            racer.current_lap = 4;
            racer.target_gate_idx = (i + 1) as u8;
            racer.is_finished = false;
            racer.state.position = track.start_pos;
        });
        let standings = compute_standings(track.start_pos, 4, 9, true, &rivals, track);
        assert_eq!(standings[0], 0, "the finisher must be classified first");
    }

    #[test]
    fn a_finisher_ahead_of_a_merely_running_car_wins() {
        let track = ALL_TRACKS[0];
        let rivals = rivals_on(track, 5, |i, racer| {
            racer.current_lap = if i == 0 { 6 } else { 5 };
            racer.target_gate_idx = 0;
            racer.is_finished = i == 0;
            racer.state.position = track.start_pos;
        });
        let standings = compute_standings(
            track.start_pos,
            crate::timing::TOTAL_LAPS,
            0,
            false,
            &rivals,
            track,
        );
        // Player is still running the last lap (lap 5, not finished), the rival
        // has taken the flag (lap 6 -> normalised to 5, finished).
        assert_eq!(standings[0], 1, "the rival that finished must lead");
        assert_eq!(finishing_lap(6, true), crate::timing::TOTAL_LAPS);
        assert_eq!(finishing_lap(5, true), 5);
        assert_eq!(finishing_lap(6, false), 6);
    }

    #[test]
    fn standings_are_stable_and_deterministic() {
        let track = ALL_TRACKS[7];
        for run in 0..4 {
            let rivals = rivals_on(track, 5, |i, racer| {
                racer.current_lap = 1 + (i as u8 % 3);
                racer.target_gate_idx = i as u8;
            });
            let first = compute_standings(track.start_pos, 2, 3, false, &rivals, track);
            for _ in 0..3 {
                let again = compute_standings(track.start_pos, 2, 3, false, &rivals, track);
                assert_eq!(first, again, "run {run} was not reproducible");
            }
        }
    }

    #[test]
    fn standings_are_total_for_every_public_gate_index() {
        let track = ALL_TRACKS[0];
        let rivals = rivals_on(track, 5, |_, racer| {
            racer.current_lap = 1;
            racer.target_gate_idx = u8::MAX;
            racer.state.position = Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MIN));
        });
        for gate in [0u8, 1, 4, 5, 16, 200, 255] {
            for pos in [
                Vec2::ZERO,
                Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MAX)),
                Vec2::new(Fixed::from_raw(i32::MIN), Fixed::from_raw(i32::MIN)),
            ] {
                let standings = compute_standings(pos, 3, gate, false, &rivals, track);
                for &competitor in standings.iter() {
                    assert!(competitor < COMPETITORS_PER_SESSION);
                }
            }
        }
    }

    // ========================================================================
    // bug 4: advance_stage leaving current_stage out of range
    // ========================================================================

    #[test]
    fn advance_stage_never_runs_past_the_end_of_a_cup() {
        // Reproduced: for cup_index 3, stage 6 -> track 24, 7 -> 25, 8 -> 26
        // against a 24-track ALL_TRACKS.
        for cup in 0..CUP_COUNT as u8 {
            let mut session = ChampionshipSession::new(cup);
            let mut finishes = 0;
            for _ in 0..40 {
                if session.advance_stage() {
                    finishes += 1;
                }
                assert!(
                    session.current_stage < STAGES_PER_CUP,
                    "cup {cup}: stage ran to {}",
                    session.current_stage
                );
                let idx = session.current_track_idx();
                assert!(
                    idx < TOTAL_TRACKS && idx < ALL_TRACKS.len(),
                    "cup {cup}: track index {idx} is out of range"
                );
            }
            // Five advances walk the stages, then the cup reports itself done
            // for good.
            assert_eq!(
                finishes,
                40 - (STAGES_PER_CUP as usize - 1),
                "cup {cup} reported itself finished {finishes} times"
            );
        }
    }

    #[test]
    fn the_last_stage_of_the_last_cup_is_the_last_track() {
        let mut session = ChampionshipSession::new(3);
        while !session.advance_stage() {}
        assert_eq!(session.current_stage, STAGES_PER_CUP - 1);
        assert_eq!(session.current_track_idx(), TOTAL_TRACKS - 1);
        // Every stage of every cup walks a different track, and the last one is
        // the last track shipped.
        for cup in 0..CUP_COUNT as u8 {
            let mut session = ChampionshipSession::new(cup);
            for stage in 0..STAGES_PER_CUP {
                assert_eq!(
                    session.current_track_idx(),
                    cup as usize * STAGES_PER_CUP as usize + stage as usize
                );
                assert_eq!(session.current_stage, stage);
                session.advance_stage();
            }
            assert!(session.advance_stage(), "cup {cup} should report done");
        }
        // Every slot resolves to the authored circuit `ALL_TRACK_VISUALS` says it
        // does.
        //
        // Was `assert_eq!(ALL_TRACKS[23].name, AUTHORED_TRACKS[0].name)`, which was
        // true only while one geometry filled every slot, and before that a
        // hardcoded `"Ivory Straits"` naming a circuit that no longer exists. The
        // index assertion above already covers "the final stage lands on the final
        // slot", so what is worth checking here is the *table*: the slot-to-circuit
        // mapping the renderer uploads its world texture from, against the
        // slot-to-`TrackDef` mapping everything else drives.
        //
        // These are two generated arrays filled by two loops in `build_atlas.py`,
        // and the only thing that keeps them agreeing is that they are generated
        // from the same round-robin. Checking every slot rather than just the last
        // is what catches the mapping being wrong in the middle, where a player
        // would race one circuit while looking at another circuit's road -- with
        // no error anywhere, because both lookups are individually in range.
        for (slot, track) in ALL_TRACKS.iter().enumerate() {
            let authored = ALL_TRACK_VISUALS[slot];
            assert!(
                authored < AUTHORED_TRACKS.len(),
                "slot {slot} names authored circuit {authored}, but there are only {}",
                AUTHORED_TRACKS.len()
            );
            assert_eq!(
                track.name, AUTHORED_TRACKS[authored].name,
                "slot {slot} is {} but ALL_TRACK_VISUALS says it is circuit \
                 {authored} ({})",
                track.name, AUTHORED_TRACKS[authored].name,
            );
        }
    }

    #[test]
    fn a_hand_built_session_cannot_index_past_all_tracks() {
        let mut session = ChampionshipSession::new(0);
        session.cup_index = 200;
        session.current_stage = 200;
        assert!(
            session.current_track_idx() < ALL_TRACKS.len(),
            "current_track_idx ran past ALL_TRACKS"
        );
        assert!(ChampionshipSession::new(200).cup_index < CUP_COUNT as u8);
    }

    // ========================================================================
    // bug 5: award_stage_points uniqueness
    // ========================================================================

    #[test]
    fn a_repeated_competitor_index_cannot_be_scored_twice() {
        // Reproduced: `[0, 0, 1, 2, 3, 4]` paid competitor 0 sixteen points in a
        // single race (10 for P1 plus 6 for P2).
        let mut session = ChampionshipSession::new(0);
        session.award_stage_points([0, 0, 1, 2, 3, 4]);
        assert_eq!(
            session.competitors[0].total_points, 10,
            "the player was paid for a place they did not take"
        );
        // The duplicated 2nd place is skipped, so the rivals keep *their* places.
        assert_eq!(session.competitors[1].total_points, 4);
        assert_eq!(session.competitors[2].total_points, 3);
        assert_eq!(session.competitors[3].total_points, 2);
        assert_eq!(session.competitors[4].total_points, 1);
        assert_eq!(session.competitors[5].total_points, 0);
    }

    #[test]
    fn every_duplicate_pattern_is_bounded_by_one_score_per_competitor() {
        // Every entry the same competitor: one award, for 1st, nothing else.
        for repeated in 0..COMPETITORS_PER_SESSION {
            let mut session = ChampionshipSession::new(0);
            session.award_stage_points([repeated; COMPETITORS_PER_SESSION]);
            assert_eq!(
                session.competitors[repeated].total_points, POINTS_TABLE[0] as u16,
                "competitor {repeated} should only be paid once"
            );
            let total: u32 = session
                .competitors
                .iter()
                .map(|c| c.total_points as u32)
                .sum();
            assert_eq!(
                total, POINTS_TABLE[0] as u32,
                "duplicates inflated the points awarded"
            );
        }
        // And the pathological mix from the bug report.
        let mut session = ChampionshipSession::new(0);
        session.award_stage_points([0, 0, 1, 2, 3, 4]);
        assert_eq!(session.competitors[0].total_points, 10);
    }

    #[test]
    fn out_of_range_competitor_indices_are_ignored() {
        let mut session = ChampionshipSession::new(0);
        // Places 1, 2 and 4 name competitors that do not exist, so those places
        // are simply not awarded.
        session.award_stage_points([0, 6, usize::MAX, 3, 200, 4]);
        assert_eq!(session.competitors[0].total_points, 10);
        assert_eq!(session.competitors[1].total_points, 0);
        assert_eq!(session.competitors[2].total_points, 0);
        assert_eq!(session.competitors[3].total_points, 3);
        assert_eq!(session.competitors[4].total_points, 1);
        assert_eq!(session.competitors[5].total_points, 0);
    }

    #[test]
    fn a_permutation_awards_the_full_points_table() {
        let mut session = ChampionshipSession::new(0);
        let finish = [3usize, 0, 5, 1, 4, 2];
        session.award_stage_points(finish);
        let mut total = 0u32;
        for (place, &competitor) in finish.iter().enumerate() {
            assert_eq!(
                session.competitors[competitor].total_points,
                POINTS_TABLE[place] as u16,
                "competitor {competitor} finished P{}",
                place + 1
            );
            total += session.competitors[competitor].total_points as u32;
        }
        assert_eq!(total, 26, "the full points table must still add up");
    }

    #[test]
    fn the_leaderboard_is_a_stable_sort_by_points() {
        let mut session = ChampionshipSession::new(0);
        // Three rivals tied on points must keep grid order.
        session.competitors[1].total_points = 7;
        session.competitors[2].total_points = 7;
        session.competitors[3].total_points = 9;
        let board = session.sorted_leaderboard();
        assert_eq!(board[0].name, AI_PROFILES[2].name);
        assert_eq!(board[1].name, AI_PROFILES[0].name);
        assert_eq!(board[2].name, AI_PROFILES[1].name);
    }

    #[test]
    fn six_stage_championships_award_the_expected_totals() {
        let mut session = ChampionshipSession::new(0);
        for _ in 0..STAGES_PER_CUP {
            session.award_stage_points([0, 1, 2, 3, 4, 5]);
            session.advance_stage();
        }
        // 10 + 6 + 4 + 3 + 2 + 1 points, six times.
        assert_eq!(session.competitors[0].total_points, 60);
        assert_eq!(session.competitors[1].total_points, 36);
        assert_eq!(session.competitors[2].total_points, 24);
        assert_eq!(session.competitors[3].total_points, 18);
        assert_eq!(session.competitors[4].total_points, 12);
        assert_eq!(session.competitors[5].total_points, 6);
        assert_eq!(session.sorted_leaderboard()[0].name, "PLAYER");
    }
}
