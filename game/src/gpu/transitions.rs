//! Arcade Screen Transitions (`NO1-036`).
//!
//! Provides 8-frame screen transitions between menus, circuits, and results:
//! - **Venetian Blinds**: 16 horizontal slats expanding/contracting across the 240-line screen.
//! - **Shutter Dissolve**: Alternating vertical and horizontal shutter blades wiping into darkness.
//! - **Fade Dissolve**: Solid black screen wipe with smooth alpha-stepped coverage.

use psx_gpu as gpu;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    VenetianBlinds,
    ShutterDissolve,
    FadeDissolve,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScreenTransition {
    pub active: bool,
    pub kind: TransitionKind,
    /// `true` = fading/wiping OUT to black; `false` = fading/wiping IN from black to screen.
    pub is_out: bool,
    pub frame: u8,
    pub max_frames: u8,
}

impl Default for ScreenTransition {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenTransition {
    pub const fn new() -> Self {
        Self {
            active: false,
            kind: TransitionKind::VenetianBlinds,
            is_out: true,
            frame: 0,
            max_frames: 8,
        }
    }

    /// Begins a wipe-out transition to black (8 frames).
    pub fn start_wipe_out(&mut self, kind: TransitionKind) {
        self.active = true;
        self.kind = kind;
        self.is_out = true;
        self.frame = 0;
        self.max_frames = 8;
    }

    /// Begins a wipe-in transition from black into gameplay/menu (8 frames).
    pub fn start_wipe_in(&mut self, kind: TransitionKind) {
        self.active = true;
        self.kind = kind;
        self.is_out = false;
        self.frame = 0;
        self.max_frames = 8;
    }

    /// Advances the transition by 1 frame. Returns `true` while the transition is active.
    pub fn tick(&mut self) -> bool {
        if !self.active {
            return false;
        }
        self.frame += 1;
        if self.frame >= self.max_frames {
            self.active = false;
            return false;
        }
        true
    }

    /// Renders the transition overlay using GPU flat rectangles (`NO1-036`).
    pub fn render(&self) {
        if !self.active && self.frame == 0 {
            return;
        }

        let progress = if self.is_out {
            self.frame
        } else {
            self.max_frames.saturating_sub(self.frame)
        };

        if progress == 0 {
            return;
        }

        match self.kind {
            TransitionKind::VenetianBlinds => {
                // 16 slats of 15 scanlines each (16 * 15 = 240)
                let slat_h = 15u16;
                let cover_h = ((slat_h as u32 * progress as u32) / self.max_frames as u32) as u16;
                for slat in 0..16 {
                    let y = slat * slat_h;
                    gpu::draw_rect_flat(0, y as i16, 320, cover_h, 8, 10, 16);
                }
            }
            TransitionKind::ShutterDissolve => {
                // 10 vertical columns of 32 pixels each (10 * 32 = 320)
                let col_w = 32u16;
                let cover_w = ((col_w as u32 * progress as u32) / self.max_frames as u32) as u16;
                for col in 0..10 {
                    let x = col * col_w;
                    gpu::draw_rect_flat(x as i16, 0, cover_w, 240, 8, 10, 16);
                }
            }
            TransitionKind::FadeDissolve => {
                // Curtain closing from top and bottom
                let h = ((120u32 * progress as u32) / self.max_frames as u32) as u16;
                gpu::draw_rect_flat(0, 0, 320, h, 8, 10, 16);
                gpu::draw_rect_flat(0, (240 - h) as i16, 320, h, 8, 10, 16);
            }
        }
    }
}
