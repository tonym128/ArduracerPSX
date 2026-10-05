//! Car Tuning Garage UI Screen for PlayStation 1.
//!
//! Provides interactive 5-slider tuning allocating a 20-point performance budget
//! across Top Speed, Acceleration, Handling/Grip, Drift Stability, and Gearing,
//! complete with real-time stats and a rotating 3D turntable vehicle preview.

use crate::ui::font::{draw_char, draw_text};
use crate::ui::tuning_input::{Reject, TuningInput, TuningMenu};
use arduracer_core::math;
use arduracer_core::tuning::{CarTuning, MAX_SLIDER, MIN_SLIDER};
use psx_gpu as gpu;
use psx_pad::{button, PadState};

pub struct TuningScreen {
    /// Selection, budget and edge state. All of it is hardware-free and
    /// unit-tested in `tools/test_ui` via `tuning_input.rs` (TASK-1210).
    menu: TuningMenu,
    pub preview_angle: u16,
    /// Frames left showing the refusal banner, so a short press is legible.
    banner_frames: u8,
    /// Which refusal the banner is currently showing.
    last_reject: Reject,
}

/// How long a refusal banner stays up, in frames. Long enough to read at 60 Hz.
const BANNER_FRAMES: u8 = 90;

impl TuningScreen {
    pub fn new(initial: CarTuning) -> Self {
        TuningScreen {
            menu: TuningMenu::new(initial),
            preview_angle: 0,
            banner_frames: 0,
            last_reject: Reject::NoPointsLeft,
        }
    }

    /// The setup the player has built. Read by the exit path before persisting.
    pub fn tuning(&self) -> CarTuning {
        self.menu.tuning
    }

    /// Adopts a stored preset, discarding any in-progress edits and any armed
    /// reset prompt.
    pub fn load_tuning(&mut self, tuning: CarTuning) {
        self.menu = TuningMenu::new(tuning);
        self.banner_frames = 0;
    }

    /// Adopts the buttons held right now as the edge baseline.
    ///
    /// Called when the garage opens so a `Cross`/`Start` still held from the
    /// main menu does not immediately save-and-exit.
    pub fn sync_edges(&mut self, pad: &PadState) {
        self.menu.sync_edges(Self::input_from_pad(pad));
    }

    /// Bridges `psx_pad`'s bitmask to the hardware-free input struct.
    pub fn input_from_pad(pad: &PadState) -> TuningInput {
        let b = pad.buttons;
        TuningInput {
            up: b.is_held(button::UP),
            down: b.is_held(button::DOWN),
            left: b.is_held(button::LEFT),
            right: b.is_held(button::RIGHT),
            cross: b.is_held(button::CROSS),
            circle: b.is_held(button::CIRCLE),
            start: b.is_held(button::START),
            triangle: b.is_held(button::TRIANGLE),
        }
    }

    /// Human-readable reason a press was refused.
    fn banner_text(reject: Reject) -> &'static str {
        match reject {
            Reject::AtMax => "MAX",
            Reject::AtMin => "MIN",
            Reject::NoPointsLeft => "NO POINTS",
            Reject::Unbalanced => "SPEND ALL PTS",
        }
    }

    /// Updates tuning navigation and adjustments. Returns true when the player
    /// saves and leaves with CIRCLE/START.
    pub fn update(&mut self, pad: &PadState) -> bool {
        self.preview_angle = (self.preview_angle + 24) % 4096;

        let frame = self.menu.update(Self::input_from_pad(pad));

        if let Some(reject) = frame.rejected {
            self.last_reject = reject;
            self.banner_frames = BANNER_FRAMES;
        } else if self.banner_frames > 0 {
            self.banner_frames -= 1;
        }

        frame.exited
    }

    /// Renders the tuning garage interface.
    pub fn render(&self, _draw_y: i16) {
        // Background
        gpu::draw_rect_flat(10, 10, 300, 220, 16, 20, 28);
        gpu::draw_rect_flat(12, 12, 296, 216, 25, 30, 38);

        // Header
        draw_text(90, 18, "TUNING GARAGE", (255, 220, 0), 2);
        gpu::draw_rect_flat(30, 36, 260, 2, 220, 40, 60);

        // Points budget summary. `points_remaining` clamps at zero, and the
        // budget is now enforced on both edges, so the count is a real 0..=20
        // rather than wrapping at 9 as it did when the range was unbounded.
        let remaining = self.menu.points_remaining();

        draw_text(24, 46, "POINTS REMAINING: ", (0, 220, 255), 1);
        draw_char(132, 46, b'0' + (remaining % 10), (255, 255, 255), 1);
        if remaining >= 10 {
            draw_char(140, 46, b'0' + (remaining / 10), (255, 255, 255), 1);
        }

        // Sliders
        let sliders = [
            ("TOP SPEED", self.menu.tuning.top_speed),
            ("ACCELERATION", self.menu.tuning.acceleration),
            ("HANDLING", self.menu.tuning.handling),
            ("DRIFT GRIP", self.menu.tuning.drift_stability),
            ("GEARING RATIO", self.menu.tuning.gearing),
        ];

        for (idx, (name, val)) in sliders.iter().enumerate() {
            let y = 64 + (idx as i16) * 26;
            let is_sel = (idx as u8) == self.menu.selected.index();

            if is_sel {
                gpu::draw_rect_flat(18, y - 2, 180, 22, 220, 30, 50);
            }

            draw_text(
                22,
                (y + 2) as u16,
                name,
                if is_sel {
                    (255, 255, 255)
                } else {
                    (180, 190, 200)
                },
                1,
            );

            // Slider gauge, one notch per legal slider value. Scaled from
            // `MIN_SLIDER..=MAX_SLIDER` so the gauge and the gate
            // `CarTuning::is_valid` applies can never disagree.
            let gauge_x = 105i16;
            let gauge_y = y + 4;
            let gauge_w = 80u16;
            gpu::draw_rect_flat(gauge_x, gauge_y, gauge_w, 10, 20, 20, 30);
            let span = (MAX_SLIDER - MIN_SLIDER) as i32;
            let filled = (*val as i32) - MIN_SLIDER as i32;
            let fill_w = (gauge_w as i32 * filled) / span;
            if fill_w > 0 {
                gpu::draw_rect_flat(
                    gauge_x + 1,
                    gauge_y + 1,
                    (fill_w - 1) as u16,
                    8,
                    255,
                    200,
                    0,
                );
            }
            draw_char(
                (gauge_x as u16) + gauge_w + 4,
                (gauge_y - 4) as u16,
                b'0' + val,
                (255, 255, 255),
                1,
            );
        }

        // Vehicle Preview Turntable (Top Right panel)
        gpu::draw_rect_flat(210, 64, 86, 86, 14, 16, 22);
        gpu::draw_rect_flat(212, 66, 82, 82, 30, 36, 48);
        draw_text(224, 70, "CAR 01", (0, 220, 255), 1);

        // Rotating preview car on turntable
        let pv_cx = 253i16;
        let pv_cy = 115i16;
        let cos_a = math::cos(self.preview_angle).raw() as i32;
        let sin_a = math::sin(self.preview_angle).raw() as i32;

        let tf = |u: i32, v: i32| -> (i16, i16) {
            let x = pv_cx as i32 + ((u * cos_a + v * sin_a) >> 12);
            let y = pv_cy as i32 + ((u * sin_a - v * cos_a) >> 12);
            (x as i16, y as i16)
        };

        // Rotating turntable shadow
        let p_sh = [tf(-6, 12), tf(6, 12), tf(-7, -12), tf(7, -12)];
        let p_sh_offset = [
            (p_sh[0].0 + 2, p_sh[0].1 + 2),
            (p_sh[1].0 + 2, p_sh[1].1 + 2),
            (p_sh[2].0 + 2, p_sh[2].1 + 2),
            (p_sh[3].0 + 2, p_sh[3].1 + 2),
        ];
        gpu::draw_quad_flat(p_sh_offset, 16, 20, 28);

        // Rotating car body
        let p_body = [tf(-6, 12), tf(6, 12), tf(-7, -12), tf(7, -12)];
        gpu::draw_quad_flat(p_body, 220, 25, 45);

        // Rotating windshield glass
        let p_glass = [tf(-4, 4), tf(4, 4), tf(-5, -4), tf(5, -4)];
        gpu::draw_quad_flat(p_glass, 25, 40, 60);

        // Headlights
        let hl_l = tf(-4, 11);
        let hl_r = tf(4, 11);
        gpu::draw_line_mono(hl_l.0, hl_l.1, hl_l.0 + 1, hl_l.1, 255, 240, 150);
        gpu::draw_line_mono(hl_r.0, hl_r.1, hl_r.0 + 1, hl_r.1, 255, 240, 150);

        // Reset confirmation prompt. Triangle arms this instead of wiping the
        // setup outright, which is what a single mis-press used to do.
        if self.menu.reset_pending {
            gpu::draw_rect_flat(60, 96, 200, 44, 40, 20, 20);
            draw_text(76, 104, "RESET TO DEFAULT?", (255, 240, 120), 1);
            draw_text(76, 122, "X: YES   O: CANCEL", (220, 220, 230), 1);
        }

        // Refusal banner, so a press that changed nothing explains itself
        // instead of looking like dropped input.
        if self.banner_frames > 0 {
            draw_text(
                206,
                46,
                Self::banner_text(self.last_reject),
                (255, 120, 120),
                1,
            );
        }

        // Instructions Footer
        draw_text(
            24,
            200,
            "L/R: TUNE  TRI: RESET  CIR/STA: SAVE & EXIT",
            (130, 150, 170),
            1,
        );
    }
}
