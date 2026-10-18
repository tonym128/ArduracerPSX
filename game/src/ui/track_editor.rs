//! TrackMania In-Game Modular Track Builder Screen.
//!
//! Allows players to construct, rotate, place, test-drive, and validate custom
//! tracks on a 20x20 modular grid (80x80 physics tile world).
//!
//! Compact serialization directly writes up to 4 custom tracks into the PS1 Memory Card.

use crate::ui::font::draw_text;
use arduracer_core::track_builder::{
    CustomTrackDef, PlacedBlock, TrackBlockKind, TrackBuildError, CUSTOM_GRID_MACRO_DIM,
    TOTAL_CUSTOM_TRACK_SLOTS,
};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

/// Outcome action from updating the track editor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrackEditorAction {
    None,
    ExitToMenu,
    TestDriveValidated(CustomTrackDef),
    SaveToCard(usize, CustomTrackDef),
    LoadFromCard(usize),
}

pub struct TrackEditorScreen {
    pub slot: usize,
    pub track: CustomTrackDef,
    pub cursor_x: u8,
    pub cursor_y: u8,
    pub palette_kind: TrackBlockKind,
    pub palette_orientation: u8,
    pub anim_timer: u32,
    pub status_msg: Option<(&'static str, u16)>,

    // Edge-detection button states
    prev_up: bool,
    prev_down: bool,
    prev_left: bool,
    prev_right: bool,
    prev_cross: bool,
    prev_square: bool,
    prev_triangle: bool,
    prev_circle: bool,
    prev_l1: bool,
    prev_r1: bool,
    prev_l2: bool,
    prev_r2: bool,
    prev_start: bool,
    prev_select: bool,
}

impl Default for TrackEditorScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackEditorScreen {
    pub fn new() -> Self {
        let mut track = CustomTrackDef::new("CUSTOM ARENA 1");
        // Seed default starter layout
        let _ = track.place_block(8, 9, TrackBlockKind::StartGrid, 0);
        let _ = track.place_block(8, 8, TrackBlockKind::Straight, 0);
        let _ = track.place_block(8, 7, TrackBlockKind::Turn90, 0);

        Self {
            slot: 0,
            track,
            cursor_x: 8,
            cursor_y: 9,
            palette_kind: TrackBlockKind::Straight,
            palette_orientation: 0,
            anim_timer: 0,
            status_msg: None,
            prev_up: true,
            prev_down: true,
            prev_left: true,
            prev_right: true,
            prev_cross: true,
            prev_square: true,
            prev_triangle: true,
            prev_circle: true,
            prev_l1: true,
            prev_r1: true,
            prev_l2: true,
            prev_r2: true,
            prev_start: true,
            prev_select: true,
        }
    }

    pub fn arm_for_entry(&mut self) {
        self.prev_up = true;
        self.prev_down = true;
        self.prev_left = true;
        self.prev_right = true;
        self.prev_cross = true;
        self.prev_square = true;
        self.prev_triangle = true;
        self.prev_circle = true;
        self.prev_l1 = true;
        self.prev_r1 = true;
        self.prev_l2 = true;
        self.prev_r2 = true;
        self.prev_start = true;
        self.prev_select = true;
    }

    pub fn set_status(&mut self, msg: &'static str) {
        self.status_msg = Some((msg, 120)); // Show for 2 seconds (120 frames)
    }

    pub fn load_track(&mut self, track: CustomTrackDef) {
        self.track = track;
        self.set_status("TRACK LOADED FROM CARD!");
    }

    pub fn update(&mut self, pad: &PadState) -> TrackEditorAction {
        self.anim_timer = self.anim_timer.wrapping_add(1);

        if let Some((_, frames)) = &mut self.status_msg {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.status_msg = None;
            }
        }

        let b = pad.buttons;
        let up = b.is_held(button::UP);
        let down = b.is_held(button::DOWN);
        let left = b.is_held(button::LEFT);
        let right = b.is_held(button::RIGHT);

        // Move cursor with auto-repeat cadence
        if up && !self.prev_up {
            if self.cursor_y > 0 {
                self.cursor_y -= 1;
            } else {
                self.cursor_y = (CUSTOM_GRID_MACRO_DIM - 1) as u8;
            }
        }
        if down && !self.prev_down {
            if (self.cursor_y as usize) < CUSTOM_GRID_MACRO_DIM - 1 {
                self.cursor_y += 1;
            } else {
                self.cursor_y = 0;
            }
        }
        if left && !self.prev_left {
            if self.cursor_x > 0 {
                self.cursor_x -= 1;
            } else {
                self.cursor_x = (CUSTOM_GRID_MACRO_DIM - 1) as u8;
            }
        }
        if right && !self.prev_right {
            if (self.cursor_x as usize) < CUSTOM_GRID_MACRO_DIM - 1 {
                self.cursor_x += 1;
            } else {
                self.cursor_x = 0;
            }
        }

        self.prev_up = up;
        self.prev_down = down;
        self.prev_left = left;
        self.prev_right = right;

        // L1 / R1: Rotate orientation
        let l1 = b.is_held(button::L1);
        let r1 = b.is_held(button::R1);
        if l1 && !self.prev_l1 {
            self.palette_orientation = (self.palette_orientation + 3) & 0x03;
        }
        if r1 && !self.prev_r1 {
            self.palette_orientation = (self.palette_orientation + 1) & 0x03;
        }
        self.prev_l1 = l1;
        self.prev_r1 = r1;

        // L2 / R2: Cycle palette block kinds
        let l2 = b.is_held(button::L2);
        let r2 = b.is_held(button::R2);
        if l2 && !self.prev_l2 {
            let cur = self.palette_kind.as_u8();
            let prev = if cur > 0 {
                cur - 1
            } else {
                (TrackBlockKind::COUNT - 1) as u8
            };
            self.palette_kind = TrackBlockKind::from_u8(prev);
        }
        if r2 && !self.prev_r2 {
            let cur = self.palette_kind.as_u8();
            let next = (cur + 1) % (TrackBlockKind::COUNT as u8);
            self.palette_kind = TrackBlockKind::from_u8(next);
        }
        self.prev_l2 = l2;
        self.prev_r2 = r2;

        // CROSS: Place block at cursor
        let cross = b.is_held(button::CROSS);
        if cross && !self.prev_cross {
            match self.track.place_block(
                self.cursor_x,
                self.cursor_y,
                self.palette_kind,
                self.palette_orientation,
            ) {
                Ok(()) => {
                    self.set_status("BLOCK PLACED");
                }
                Err(TrackBuildError::DuplicatePosition) => {
                    self.set_status("MAX 64 BLOCKS REACHED!");
                }
                Err(_) => {
                    self.set_status("CANNOT PLACE HERE");
                }
            }
        }
        self.prev_cross = cross;

        // SQUARE: Delete block at cursor
        let square = b.is_held(button::SQUARE);
        if square && !self.prev_square && self.track.remove_block(self.cursor_x, self.cursor_y) {
            self.set_status("BLOCK DELETED");
        }
        self.prev_square = square;

        // TRIANGLE: Cycle Slot 1..4 and request load
        let triangle = b.is_held(button::TRIANGLE);
        if triangle && !self.prev_triangle {
            self.slot = (self.slot + 1) % TOTAL_CUSTOM_TRACK_SLOTS;
            return TrackEditorAction::LoadFromCard(self.slot);
        }
        self.prev_triangle = triangle;

        // CIRCLE: Save track to card slot
        let circle = b.is_held(button::CIRCLE);
        if circle && !self.prev_circle {
            self.set_status("SAVED TO MEMORY CARD!");
            return TrackEditorAction::SaveToCard(self.slot, self.track);
        }
        self.prev_circle = circle;

        // SELECT: Exit back to Main Menu
        let select = b.is_held(button::SELECT);
        if select && !self.prev_select {
            return TrackEditorAction::ExitToMenu;
        }
        self.prev_select = select;

        // START: Test Drive & Validate
        let start = b.is_held(button::START);
        if start && !self.prev_start {
            match self.track.validate() {
                Ok(()) => {
                    return TrackEditorAction::TestDriveValidated(self.track);
                }
                Err(TrackBuildError::NoStartGrid) => {
                    self.set_status("NEED 1 START/FINISH BLOCK!");
                }
                Err(TrackBuildError::MultipleStartGrids) => {
                    self.set_status("ONLY 1 START/FINISH ALLOWED!");
                }
                Err(TrackBuildError::EmptyTrack) => {
                    self.set_status("MINIMUM 3 BLOCKS REQUIRED!");
                }
                Err(_) => {
                    self.set_status("INVALID TRACK LAYOUT!");
                }
            }
        }
        self.prev_start = start;

        TrackEditorAction::None
    }

    /// Renders the TrackMania track builder screen.
    pub fn render(&self) {
        // Outer frame
        gpu::draw_rect_flat(6, 6, 308, 228, 12, 14, 20);
        gpu::draw_rect_flat(8, 8, 304, 224, 24, 28, 36);

        // Header Title
        draw_text(12, 12, "TRACK BUILDER", (255, 225, 40), 1);
        let slot_str = match self.slot {
            0 => "SLOT 1",
            1 => "SLOT 2",
            2 => "SLOT 3",
            _ => "SLOT 4",
        };
        draw_text(112, 12, slot_str, (100, 220, 255), 1);

        let count = self.track.block_count;
        let count_str = match count {
            0..=9 => "BLKS: 0",
            10..=19 => "BLKS: 1",
            20..=29 => "BLKS: 2",
            30..=39 => "BLKS: 3",
            40..=49 => "BLKS: 4",
            50..=59 => "BLKS: 5",
            _ => "BLKS: 6",
        };
        draw_text(170, 12, count_str, (180, 195, 215), 1);

        if self.track.is_validated() {
            draw_text(240, 12, "VALIDATED", (50, 240, 80), 1);
        } else {
            draw_text(240, 12, "DRAFT", (255, 140, 40), 1);
        }

        // Viewport dimensions: 8x8 blocks visible on screen
        const VIEW_SIZE: usize = 8;
        const CELL_PIXELS: i16 = 20;
        const CELL_W_U16: u16 = 20;
        const VIEW_W_U16: u16 = 160;

        let start_x = (self.cursor_x as usize)
            .saturating_sub(VIEW_SIZE / 2)
            .min(CUSTOM_GRID_MACRO_DIM - VIEW_SIZE);
        let start_y = (self.cursor_y as usize)
            .saturating_sub(VIEW_SIZE / 2)
            .min(CUSTOM_GRID_MACRO_DIM - VIEW_SIZE);

        let grid_screen_x: i16 = 14;
        let grid_screen_y: i16 = 26;

        // Draw grid background
        gpu::draw_rect_flat(
            grid_screen_x - 1,
            grid_screen_y - 1,
            VIEW_W_U16 + 2,
            VIEW_W_U16 + 2,
            16,
            20,
            28,
        );

        for vy in 0..VIEW_SIZE {
            for vx in 0..VIEW_SIZE {
                let gx = (start_x + vx) as u8;
                let gy = (start_y + vy) as u8;

                let sx = grid_screen_x + (vx as i16) * CELL_PIXELS;
                let sy = grid_screen_y + (vy as i16) * CELL_PIXELS;

                // Find block at (gx, gy)
                let mut found_block: Option<&PlacedBlock> = None;
                for i in 0..self.track.block_count as usize {
                    if self.track.blocks[i].x == gx && self.track.blocks[i].y == gy {
                        found_block = Some(&self.track.blocks[i]);
                        break;
                    }
                }

                if let Some(b) = found_block {
                    // Draw colored tile with orientation
                    let (cr, cg, cb) = match b.kind {
                        TrackBlockKind::StartGrid => (40, 200, 60),      // Green
                        TrackBlockKind::Straight => (120, 130, 145),     // Gray
                        TrackBlockKind::Turn90 => (50, 160, 230),        // Blue
                        TrackBlockKind::Hairpin => (240, 130, 30),       // Orange
                        TrackBlockKind::Chicane => (190, 60, 210),       // Purple
                        TrackBlockKind::DirtSection => (160, 105, 50),   // Brown
                        TrackBlockKind::IceBobsleigh => (120, 210, 255), // Cyan
                        TrackBlockKind::JumpRamp => (240, 200, 20),      // Yellow
                        TrackBlockKind::BridgeOverpass => (160, 170, 185), // Steel
                        TrackBlockKind::FreeWheel => (230, 40, 40),      // Red
                        TrackBlockKind::ReactorBoost => (210, 40, 160),  // Violet
                        TrackBlockKind::CheckpointGate => (245, 230, 40), // Gold
                    };

                    gpu::draw_rect_flat(sx + 1, sy + 1, CELL_W_U16 - 2, CELL_W_U16 - 2, cr, cg, cb);

                    // Direction arrow
                    let arrow = match b.orientation & 0x03 {
                        0 => "^",
                        1 => ">",
                        2 => "V",
                        _ => "<",
                    };
                    draw_text((sx + 7) as u16, (sy + 6) as u16, arrow, (255, 255, 255), 1);
                } else {
                    // Empty cell
                    gpu::draw_rect_flat(sx + 1, sy + 1, CELL_W_U16 - 2, CELL_W_U16 - 2, 28, 32, 44);
                }

                // Cursor highlight
                if gx == self.cursor_x && gy == self.cursor_y {
                    let pulse = (self.anim_timer / 10).is_multiple_of(2);
                    let (hr, hg, hb) = if pulse {
                        (255, 255, 60)
                    } else {
                        (255, 180, 20)
                    };
                    // Draw outer selection border
                    gpu::draw_rect_flat(sx, sy, CELL_W_U16, 2, hr, hg, hb);
                    gpu::draw_rect_flat(sx, sy + CELL_PIXELS - 2, CELL_W_U16, 2, hr, hg, hb);
                    gpu::draw_rect_flat(sx, sy, 2, CELL_W_U16, hr, hg, hb);
                    gpu::draw_rect_flat(sx + CELL_PIXELS - 2, sy, 2, CELL_W_U16, hr, hg, hb);
                }
            }
        }

        // Right side: Active tool palette & controls
        let panel_x: i16 = 184;
        let panel_y: i16 = 26;
        gpu::draw_rect_flat(panel_x, panel_y, 124, 162, 20, 24, 34);

        draw_text(
            panel_x as u16 + 6,
            panel_y as u16 + 6,
            "CURRENT TOOL:",
            (255, 235, 40),
            1,
        );
        draw_text(
            panel_x as u16 + 6,
            panel_y as u16 + 18,
            self.palette_kind.name(),
            (255, 255, 255),
            1,
        );

        let rot_label = match self.palette_orientation & 0x03 {
            0 => "DIR: NORTH (0)",
            1 => "DIR: EAST (90)",
            2 => "DIR: SOUTH (180)",
            _ => "DIR: WEST (270)",
        };
        draw_text(
            panel_x as u16 + 6,
            panel_y as u16 + 30,
            rot_label,
            (100, 220, 255),
            1,
        );

        // Tool preview box
        let pr_x = panel_x + 46;
        let pr_y = panel_y + 44;
        gpu::draw_rect_flat(pr_x - 1, pr_y - 1, 26, 26, 12, 14, 20);
        let (pcr, pcg, pcb) = match self.palette_kind {
            TrackBlockKind::StartGrid => (40, 200, 60),
            TrackBlockKind::Straight => (120, 130, 145),
            TrackBlockKind::Turn90 => (50, 160, 230),
            TrackBlockKind::Hairpin => (240, 130, 30),
            TrackBlockKind::Chicane => (190, 60, 210),
            TrackBlockKind::DirtSection => (160, 105, 50),
            TrackBlockKind::IceBobsleigh => (120, 210, 255),
            TrackBlockKind::JumpRamp => (240, 200, 20),
            TrackBlockKind::BridgeOverpass => (160, 170, 185),
            TrackBlockKind::FreeWheel => (230, 40, 40),
            TrackBlockKind::ReactorBoost => (210, 40, 160),
            TrackBlockKind::CheckpointGate => (245, 230, 40),
        };
        gpu::draw_rect_flat(pr_x, pr_y, 24, 24, pcr, pcg, pcb);
        let parrow = match self.palette_orientation & 0x03 {
            0 => "^",
            1 => ">",
            2 => "V",
            _ => "<",
        };
        draw_text(
            (pr_x + 9) as u16,
            (pr_y + 8) as u16,
            parrow,
            (255, 255, 255),
            1,
        );

        // Controls legend
        let cx = panel_x as u16 + 6;
        let cy = panel_y as u16 + 74;
        draw_text(cx, cy, "CROSS: PLACE", (180, 200, 220), 1);
        draw_text(cx, cy + 11, "SQUARE: DELETE", (180, 200, 220), 1);
        draw_text(cx, cy + 22, "L1/R1: ROTATE", (180, 200, 220), 1);
        draw_text(cx, cy + 33, "L2/R2: BLOCK", (180, 200, 220), 1);
        draw_text(cx, cy + 44, "TRI: SLOT 1-4", (180, 200, 220), 1);
        draw_text(cx, cy + 55, "CIRC: SAVE CARD", (180, 200, 220), 1);
        draw_text(cx, cy + 66, "START: TEST DRIVE", (255, 215, 30), 1);
        draw_text(cx, cy + 77, "SELECT: EXIT", (150, 160, 175), 1);

        // Bottom status banner / toast
        gpu::draw_rect_flat(14, 194, 294, 28, 18, 22, 32);
        if let Some((msg, _)) = self.status_msg {
            draw_text(20, 202, msg, (255, 240, 60), 1);
        } else {
            draw_text(
                20,
                202,
                "DRIVE 1 CLEAN LAP TO VALIDATE & SET AUTHOR TIME",
                (160, 180, 200),
                1,
            );
        }
    }
}
