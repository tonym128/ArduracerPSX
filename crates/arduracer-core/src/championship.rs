//! Championship Cups, Stages, and Points Leaderboard.
//!
//! Manages 4 Grand Prix Cups across all 24 tracks with arcade points scoring.

use crate::ai::AiRacer;
use crate::ai_profiles::AI_PROFILES;
use crate::math::Vec2;
use crate::track::TrackDef;

pub const POINTS_TABLE: [u8; 6] = [10, 6, 4, 3, 2, 1];

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
    pub competitors: [Competitor; 6],
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
            cup_index: cup_index.min(3),
            current_stage: 0,
            competitors,
        }
    }

    /// Returns the global track index (0..23) for the current stage.
    pub fn current_track_idx(&self) -> usize {
        (self.cup_index as usize * 6) + (self.current_stage as usize)
    }

    /// Awards points according to finish positions (finish_indices maps place 0..5 to competitor 0..5).
    pub fn award_stage_points(&mut self, finish_indices: [usize; 6]) {
        for (place, &comp_idx) in finish_indices.iter().enumerate() {
            if comp_idx < self.competitors.len() {
                let pts = POINTS_TABLE[place.min(POINTS_TABLE.len() - 1)];
                self.competitors[comp_idx].total_points += pts as u16;
            }
        }
    }

    /// Advances to the next stage in the cup. Returns true if championship finished.
    pub fn advance_stage(&mut self) -> bool {
        self.current_stage += 1;
        self.current_stage >= 6
    }

    /// Returns the competitors sorted by total_points descending.
    pub fn sorted_leaderboard(&self) -> [Competitor; 6] {
        let mut board = self.competitors;
        for i in 1..6 {
            let mut j = i;
            while j > 0 && board[j].total_points > board[j - 1].total_points {
                board.swap(j, j - 1);
                j -= 1;
            }
        }
        board
    }
}

/// Computes the race leaderboard / standings (indices 0..=5 representing player and 5 rivals).
/// Returns an array of competitor indices where result[0] is 1st place, result[1] is 2nd place, etc.
/// Index 0 is the human player, indices 1..5 are rivals 0..4.
pub fn compute_standings(
    player_pos: Vec2,
    player_lap: u8,
    player_gate: u8,
    player_finished: bool,
    rivals: &[AiRacer],
    track: &TrackDef,
) -> [usize; 6] {
    let gate_count = track.checkpoint_count.max(1) as usize;
    let mut scores = [(0i64, 0usize); 6];

    for i in 0..6 {
        let (pos, lap, gate_idx, finished) = if i == 0 {
            (player_pos, player_lap, player_gate, player_finished)
        } else {
            let r = &rivals[i - 1];
            (
                r.state.position,
                r.current_lap,
                r.target_gate_idx,
                r.is_finished,
            )
        };

        let gate = &track.checkpoints[(gate_idx as usize) % gate_count];
        let target_x = (gate.x as i32 * 64) + (gate.width as i32 * 32);
        let target_y = (gate.y as i32 * 64) + (gate.height as i32 * 32);
        let dx = target_x - pos.x.to_int();
        let dy = target_y - pos.y.to_int();
        let dist_sq = (dx as i64 * dx as i64) + (dy as i64 * dy as i64);

        let finish_bonus: i64 = if finished { 1_000_000_000_000 } else { 0 };
        let lap_score = (lap as i64) * 10_000_000;
        let gate_score = (gate_idx as i64) * 100_000;
        let score = finish_bonus + lap_score + gate_score - dist_sq.min(99_999);

        scores[i] = (score, i);
    }

    // Sort descending by score (insertion sort)
    for i in 1..6 {
        let mut j = i;
        while j > 0 && scores[j].0 > scores[j - 1].0 {
            scores.swap(j, j - 1);
            j -= 1;
        }
    }

    let mut standings = [0usize; 6];
    for i in 0..6 {
        standings[i] = scores[i].1;
    }
    standings
}
