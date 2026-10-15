//! Real-time Debug Metrics and Performance Overlays.
//!
//! Provides non-intrusive onscreen telemetry for PSX hardware and emulators:
//! - Mode 0 (Off): Hidden.
//! - Mode 1 (Simple FPS): Compact frame rate and frame timing badge.
//! - Mode 2 (Simple Graphing FPS): Frame rate and timing plus a 44-frame sparkline
//!   histogram with target frame budget indicator.
//! - Mode 3 (All Metrics): Full developer dashboard with subsystem CPU times
//!   (sim, track render, car render, misc), async tile streaming counters,
//!   DMA/FIFO upload telemetry, and live vehicle dynamics.

use crate::ui::font::{draw_char, draw_text};
use arduracer_core::surface::SurfaceType;
use arduracer_core::track::TrackDef;
use arduracer_core::vehicle::VehicleState;
use psx_gpu as gpu;

pub const DEBUG_MODE_OFF: u8 = 0;
pub const DEBUG_MODE_FPS: u8 = 1;
pub const DEBUG_MODE_GRAPH: u8 = 2;
pub const DEBUG_MODE_ALL: u8 = 3;

/// Rolling sparkline history length (matches 44-column display).
pub const HISTORY_CAPACITY: usize = 44;

/// Root counter (Timer 2) clock rate is 4.2336 MHz (33.8688 MHz sysclock / 8).
/// 4,233.6 ticks = 1 millisecond.
/// 423.36 ticks = 0.1 millisecond (1 tenth of a millisecond).
#[inline]
pub fn ticks_to_tenths(ticks: u16) -> u16 {
    (((ticks as u32) * 10) / 4234).min(u16::MAX as u32) as u16
}

/// State and measurement buffers for the onscreen debug overlay.
pub struct DebugOverlay {
    pub mode: u8,
    // Frame history in tenths of ms (e.g. 167 = 16.7 ms, 333 = 33.3 ms).
    history: [u16; HISTORY_CAPACITY],
    history_head: usize,

    // Sliding window FPS calculation (sampled over 30 frames).
    window_vblanks: u32,
    window_frames: u32,
    pub fps_tenths: u16,
    pub frame_ms_tenths: u16,

    // Subsystem CPU processing times in tenths of ms.
    pub sim_ms_tenths: u16,
    pub track_ms_tenths: u16,
    pub cars_ms_tenths: u16,
    pub misc_ms_tenths: u16,
    pub cpu_ms_tenths: u16,

    // Tile streaming telemetry.
    last_landed_sample: u32,
    pub fetches_per_sec: u16,
    fetch_timer_frames: u16,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugOverlay {
    pub const fn new() -> Self {
        Self {
            mode: DEBUG_MODE_OFF,
            history: [167; HISTORY_CAPACITY],
            history_head: 0,
            window_vblanks: 0,
            window_frames: 0,
            fps_tenths: 600,
            frame_ms_tenths: 167,
            sim_ms_tenths: 0,
            track_ms_tenths: 0,
            cars_ms_tenths: 0,
            misc_ms_tenths: 0,
            cpu_ms_tenths: 0,
            last_landed_sample: 0,
            fetches_per_sec: 0,
            fetch_timer_frames: 0,
        }
    }

    /// Records stage timing deltas (Timer 2 ticks) and updates pacing statistics.
    pub fn record_timings(
        &mut self,
        elapsed_vblanks: u32,
        sim_ticks: u16,
        track_ticks: u16,
        cars_ticks: u16,
        misc_ticks: u16,
    ) {
        let sim_t = ticks_to_tenths(sim_ticks);
        let track_t = ticks_to_tenths(track_ticks);
        let cars_t = ticks_to_tenths(cars_ticks);
        let misc_t = ticks_to_tenths(misc_ticks);

        self.sim_ms_tenths = sim_t;
        self.track_ms_tenths = track_t;
        self.cars_ms_tenths = cars_t;
        self.misc_ms_tenths = misc_t;
        self.cpu_ms_tenths = sim_t
            .saturating_add(track_t)
            .saturating_add(cars_t)
            .saturating_add(misc_t);

        // Frame duration based on elapsed VBlanks (each VBlank is 16.666 ms).
        let vblanks = elapsed_vblanks.clamp(1, 10);
        self.frame_ms_tenths = ((vblanks * 1000) / 60) as u16;

        // Windowed FPS calculation across the last 30 frames.
        self.window_vblanks += vblanks;
        self.window_frames += 1;
        if self.window_frames >= 30 {
            if let Some(fps) = (self.window_frames * 600).checked_div(self.window_vblanks) {
                self.fps_tenths = fps.min(999) as u16;
            }
            self.window_vblanks = 0;
            self.window_frames = 0;
        }

        // Store frame time sample in rolling sparkline.
        // If 1 VBlank, record total CPU time so budget headroom is visible.
        let sample = if vblanks > 1 {
            self.frame_ms_tenths
        } else {
            self.cpu_ms_tenths.max(self.frame_ms_tenths)
        };
        self.history[self.history_head] = sample;
        self.history_head = (self.history_head + 1) % HISTORY_CAPACITY;

        // Tile streaming rate per second.
        self.fetch_timer_frames += 1;
        if self.fetch_timer_frames >= 60 {
            let current_landed = crate::async_tile::landed();
            self.fetches_per_sec = current_landed
                .saturating_sub(self.last_landed_sample)
                .min(u16::MAX as u32) as u16;
            self.last_landed_sample = current_landed;
            self.fetch_timer_frames = 0;
        }
    }

    /// Renders the active debug metrics mode.
    pub fn render(&self, player: &VehicleState, track: &TrackDef, vblanks_per_frame: u32) {
        match self.mode {
            DEBUG_MODE_FPS => self.render_simple_fps(vblanks_per_frame),
            DEBUG_MODE_GRAPH => self.render_graphing_fps(vblanks_per_frame),
            DEBUG_MODE_ALL => self.render_all_metrics(player, track, vblanks_per_frame),
            _ => {}
        }
    }

    /// Mode 1: Compact FPS & frame timing pill beneath top-right HUD cluster.
    fn render_simple_fps(&self, vblanks_per_frame: u32) {
        let x: i16 = 216;
        let y: i16 = 52;
        let w: u16 = 96;
        let h: u16 = 16;

        gpu::draw_rect_flat(x, y, w, h, 14, 16, 24);
        gpu::draw_rect_flat(x + 1, y + 1, w - 2, h - 2, 22, 26, 36);

        let fps_col = fps_color(self.fps_tenths, vblanks_per_frame);
        let mut fps_buf = [b' '; 9];
        format_fps_str(&mut fps_buf, self.fps_tenths);
        blit_buf(x + 4, y + 4, &fps_buf, fps_col);

        let mut ms_buf = [b' '; 7];
        format_ms_str(&mut ms_buf, self.frame_ms_tenths);
        blit_buf(x + 52, y + 4, &ms_buf, (200, 215, 240));
    }

    /// Mode 2: FPS, frame timing, and rolling sparkline histogram.
    fn render_graphing_fps(&self, vblanks_per_frame: u32) {
        let x: i16 = 216;
        let y: i16 = 52;
        let w: u16 = 96;
        let h: u16 = 44;

        gpu::draw_rect_flat(x, y, w, h, 14, 16, 24);
        gpu::draw_rect_flat(x + 1, y + 1, w - 2, h - 2, 22, 26, 36);

        // Header: FPS and ms
        let fps_col = fps_color(self.fps_tenths, vblanks_per_frame);
        let mut fps_buf = [b' '; 9];
        format_fps_str(&mut fps_buf, self.fps_tenths);
        blit_buf(x + 4, y + 4, &fps_buf, fps_col);

        let mut ms_buf = [b' '; 7];
        format_ms_str(&mut ms_buf, self.frame_ms_tenths);
        blit_buf(x + 52, y + 4, &ms_buf, (200, 215, 240));

        // Graph area: 88 px wide (44 columns x 2 px), 24 px tall
        let gx: i16 = x + 4;
        let gy: i16 = y + 15;
        let gw: u16 = 88;
        let gh: u16 = 24;
        gpu::draw_rect_flat(gx, gy, gw, gh, 12, 14, 20);

        // Reference target budget line (16.7 ms for 60 FPS, 33.3 ms for 30 FPS)
        let target_tenths: u16 = if vblanks_per_frame <= 1 { 167 } else { 333 };
        let max_scale_tenths: u16 = if vblanks_per_frame <= 1 { 333 } else { 500 };
        let budget_y: i16 = gy + (gh as i16)
            - (((target_tenths as i32 * gh as i32) / max_scale_tenths as i32) as i16);
        gpu::draw_rect_flat(gx, budget_y, gw, 1, 60, 100, 140);

        // Draw rolling bars
        let mut head = self.history_head;
        for col in 0..HISTORY_CAPACITY {
            let sample_ms = self.history[head];
            head = (head + 1) % HISTORY_CAPACITY;

            let bar_h = ((sample_ms as i32 * (gh as i32 - 1)) / max_scale_tenths as i32)
                .clamp(1, gh as i32) as u16;
            let bar_y = gy + (gh as i16) - (bar_h as i16);
            let bar_x = gx + (col as i16) * 2;

            let (br, bg, bb) = if sample_ms <= target_tenths {
                (40, 220, 70) // Target met: green
            } else if sample_ms <= target_tenths + 30 {
                (255, 200, 30) // Minor frame pacing jitter: yellow
            } else {
                (245, 60, 60) // Dropped frame spike: red
            };

            gpu::draw_rect_flat(bar_x, bar_y, 1, bar_h, br, bg, bb);
        }
    }

    /// Mode 3: Full developer telemetry panel at bottom-right of screen.
    fn render_all_metrics(&self, player: &VehicleState, track: &TrackDef, vblanks_per_frame: u32) {
        let x: i16 = 152;
        let y: i16 = 118;
        let w: u16 = 162;
        let h: u16 = 118;

        gpu::draw_rect_flat(x, y, w, h, 14, 16, 24);
        gpu::draw_rect_flat(x + 1, y + 1, w - 2, h - 2, 22, 26, 36);
        gpu::draw_rect_flat(x + 1, y + 1, w - 2, 2, 220, 40, 60);

        // Line 1: FPS & Frame Time
        let fps_col = fps_color(self.fps_tenths, vblanks_per_frame);
        draw_text((x + 4) as u16, (y + 5) as u16, "FPS:", (150, 160, 185), 1);
        let mut fps_buf = [b' '; 5];
        format_tenths(&mut fps_buf, self.fps_tenths);
        blit_buf(x + 30, y + 5, &fps_buf, fps_col);

        let mut ms_buf = [b' '; 7];
        format_ms_str(&mut ms_buf, self.frame_ms_tenths);
        blit_buf(x + 76, y + 5, &ms_buf, (200, 215, 240));

        // Line 2: CPU Processing Time & Slack Budget
        let target_tenths: u16 = if vblanks_per_frame <= 1 { 167 } else { 333 };
        let slack_tenths = target_tenths.saturating_sub(self.cpu_ms_tenths);
        draw_text((x + 4) as u16, (y + 14) as u16, "CPU:", (150, 160, 185), 1);
        let mut cpu_buf = [b' '; 7];
        format_ms_str(&mut cpu_buf, self.cpu_ms_tenths);
        blit_buf(x + 30, y + 14, &cpu_buf, (255, 230, 100));

        draw_text((x + 80) as u16, (y + 14) as u16, "SLK:", (150, 160, 185), 1);
        let mut slk_buf = [b' '; 7];
        format_ms_str(&mut slk_buf, slack_tenths);
        blit_buf(x + 106, y + 14, &slk_buf, (100, 220, 150));

        // Line 3: Sim Time & Track Render Time
        draw_text((x + 4) as u16, (y + 23) as u16, "SIM:", (150, 160, 185), 1);
        let mut sim_buf = [b' '; 7];
        format_ms_str(&mut sim_buf, self.sim_ms_tenths);
        blit_buf(x + 30, y + 23, &sim_buf, (180, 200, 220));

        draw_text((x + 80) as u16, (y + 23) as u16, "TRK:", (150, 160, 185), 1);
        let mut trk_buf = [b' '; 7];
        format_ms_str(&mut trk_buf, self.track_ms_tenths);
        blit_buf(x + 106, y + 23, &trk_buf, (180, 200, 220));

        // Line 4: Car Render & Misc
        draw_text((x + 4) as u16, (y + 32) as u16, "CAR:", (150, 160, 185), 1);
        let mut car_buf = [b' '; 7];
        format_ms_str(&mut car_buf, self.cars_ms_tenths);
        blit_buf(x + 30, y + 32, &car_buf, (180, 200, 220));

        draw_text((x + 80) as u16, (y + 32) as u16, "MSC:", (150, 160, 185), 1);
        let mut msc_buf = [b' '; 7];
        format_ms_str(&mut msc_buf, self.misc_ms_tenths);
        blit_buf(x + 106, y + 32, &msc_buf, (180, 200, 220));

        // Line 5: Tile Streaming Queue & Landed
        let q_depth = crate::async_tile::outstanding();
        let landed = crate::async_tile::landed();
        draw_text(
            (x + 4) as u16,
            (y + 41) as u16,
            "TILE Q:",
            (150, 160, 185),
            1,
        );
        let mut num_buf = [b' '; 4];
        format_u16_dec(&mut num_buf, q_depth.min(9999) as u16);
        blit_buf(x + 46, y + 41, &num_buf, (255, 255, 255));

        draw_text((x + 80) as u16, (y + 41) as u16, "LND:", (150, 160, 185), 1);
        format_u16_dec(&mut num_buf, landed.min(9999) as u16);
        blit_buf(x + 106, y + 41, &num_buf, (255, 255, 255));

        // Line 6: Fetch Rate & Seeks
        let seeks = crate::async_tile::seeks();
        draw_text(
            (x + 4) as u16,
            (y + 50) as u16,
            "F/SEC:",
            (150, 160, 185),
            1,
        );
        format_u16_dec(&mut num_buf, self.fetches_per_sec);
        blit_buf(x + 44, y + 50, &num_buf, (100, 220, 255));

        draw_text(
            (x + 80) as u16,
            (y + 50) as u16,
            "SEEK:",
            (150, 160, 185),
            1,
        );
        format_u16_dec(&mut num_buf, seeks.min(9999) as u16);
        blit_buf(x + 112, y + 50, &num_buf, (255, 255, 255));

        // Line 7: Waited & Stalled
        let waited = crate::async_tile::waited();
        let stalled = crate::async_tile::stalled();
        draw_text((x + 4) as u16, (y + 59) as u16, "WAIT:", (150, 160, 185), 1);
        format_u16_dec(&mut num_buf, waited.min(9999) as u16);
        blit_buf(x + 36, y + 59, &num_buf, (255, 255, 255));

        draw_text(
            (x + 80) as u16,
            (y + 59) as u16,
            "STAL:",
            (150, 160, 185),
            1,
        );
        let stall_col = if stalled > 0 {
            (255, 80, 80)
        } else {
            (255, 255, 255)
        };
        format_u16_dec(&mut num_buf, stalled.min(9999) as u16);
        blit_buf(x + 112, y + 59, &num_buf, stall_col);

        // Line 8: Uploads DMA & FIFO
        let (dma_u, fifo_u) = crate::upload_path_counts();
        draw_text(
            (x + 4) as u16,
            (y + 68) as u16,
            "UP DMA:",
            (150, 160, 185),
            1,
        );
        format_u16_dec(&mut num_buf, dma_u.min(9999) as u16);
        blit_buf(x + 48, y + 68, &num_buf, (100, 220, 255));

        draw_text(
            (x + 80) as u16,
            (y + 68) as u16,
            "FIFO:",
            (150, 160, 185),
            1,
        );
        let fifo_col = if fifo_u > 0 {
            (255, 180, 50)
        } else {
            (255, 255, 255)
        };
        format_u16_dec(&mut num_buf, fifo_u.min(9999) as u16);
        blit_buf(x + 112, y + 68, &num_buf, fifo_col);

        // Line 9: Speed & RPM
        let speed_mph = ((player.speed.raw() * 145) / 14000).clamp(0, 199) as u16;
        draw_text((x + 4) as u16, (y + 77) as u16, "SPD:", (150, 160, 185), 1);
        let mut spd_buf = [b' '; 7];
        format_speed_short(&mut spd_buf, speed_mph);
        blit_buf(x + 30, y + 77, &spd_buf, (240, 240, 250));

        draw_text((x + 80) as u16, (y + 77) as u16, "RPM:", (150, 160, 185), 1);
        format_u16_dec(&mut num_buf, player.engine_rpm);
        blit_buf(x + 106, y + 77, &num_buf, (255, 220, 110));

        // Line 10: Gear & Surface
        draw_text((x + 4) as u16, (y + 86) as u16, "GEAR:", (150, 160, 185), 1);
        let gear_char = if player.is_reversing {
            b'R'
        } else {
            b'0' + player.gear.clamp(1, 5)
        };
        draw_char(
            (x + 36) as u16,
            (y + 86) as u16,
            gear_char,
            (255, 220, 110),
            1,
        );

        let tx = player.position.x.to_int().clamp(0, 255) as u8;
        let ty = player.position.y.to_int().clamp(0, 255) as u8;
        let surf = track.surface_at(tx, ty);
        let surf_label = match surf {
            SurfaceType::Tarmac => "TARMAC",
            SurfaceType::Curb => "CURB",
            SurfaceType::OffRoad => "OFFROAD",
            SurfaceType::OilSlick => "OIL",
            SurfaceType::BoostPad => "BOOST",
            SurfaceType::Barrier => "WALL",
        };
        draw_text((x + 60) as u16, (y + 86) as u16, "SRF:", (150, 160, 185), 1);
        draw_text(
            (x + 86) as u16,
            (y + 86) as u16,
            surf_label,
            (180, 220, 255),
            1,
        );

        // Mini sparkline graph at the bottom of the panel
        let gx: i16 = x + 4;
        let gy: i16 = y + 97;
        let gw: u16 = 152;
        let gh: u16 = 16;
        gpu::draw_rect_flat(gx, gy, gw, gh, 12, 14, 20);

        let max_scale_tenths: u16 = if vblanks_per_frame <= 1 { 333 } else { 500 };
        let budget_y: i16 = gy + (gh as i16)
            - (((target_tenths as i32 * gh as i32) / max_scale_tenths as i32) as i16);
        gpu::draw_rect_flat(gx, budget_y, gw, 1, 60, 100, 140);

        let mut head = self.history_head;
        for col in 0..HISTORY_CAPACITY {
            let sample_ms = self.history[head];
            head = (head + 1) % HISTORY_CAPACITY;

            let bar_h = ((sample_ms as i32 * (gh as i32 - 1)) / max_scale_tenths as i32)
                .clamp(1, gh as i32) as u16;
            let bar_y = gy + (gh as i16) - (bar_h as i16);
            let bar_x = gx + (col as i16) * 3;

            let (br, bg, bb) = if sample_ms <= target_tenths {
                (40, 220, 70)
            } else if sample_ms <= target_tenths + 30 {
                (255, 200, 30)
            } else {
                (245, 60, 60)
            };

            gpu::draw_rect_flat(bar_x, bar_y, 2, bar_h, br, bg, bb);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fps_color(fps_tenths: u16, vblanks_per_frame: u32) -> (u8, u8, u8) {
    if vblanks_per_frame <= 1 {
        if fps_tenths >= 550 {
            (60, 240, 90)
        } else if fps_tenths >= 300 {
            (255, 200, 30)
        } else {
            (245, 60, 60)
        }
    } else if fps_tenths >= 280 {
        (60, 240, 90)
    } else {
        (245, 60, 60)
    }
}

fn blit_buf(x: i16, y: i16, buf: &[u8], color: (u8, u8, u8)) {
    let mut cur_x = x;
    for &b in buf.iter() {
        if cur_x >= 0 && y >= 0 {
            draw_char(cur_x as u16, y as u16, b, color, 1);
        }
        cur_x += 6;
    }
}

/// Formats tenths of a value as "  0.0" .. "999.9" into a 5-byte buffer.
pub fn format_tenths(buf: &mut [u8; 5], val: u16) {
    let int_part = val / 10;
    let dec_part = (val % 10) as u8;
    let h = (int_part / 100) as u8;
    let t = ((int_part / 10) % 10) as u8;
    let o = (int_part % 10) as u8;
    buf[0] = if h > 0 { b'0' + h } else { b' ' };
    buf[1] = if h > 0 || t > 0 { b'0' + t } else { b' ' };
    buf[2] = b'0' + o;
    buf[3] = b'.';
    buf[4] = b'0' + dec_part;
}

/// Formats "60.0 FPS" into a 9-byte buffer.
pub fn format_fps_str(buf: &mut [u8; 9], tenths: u16) {
    let mut num = [b' '; 5];
    format_tenths(&mut num, tenths);
    buf[0..5].copy_from_slice(&num);
    buf[5] = b' ';
    buf[6] = b'F';
    buf[7] = b'P';
    buf[8] = b'S';
}

/// Formats " 16.7MS" into a 7-byte buffer.
pub fn format_ms_str(buf: &mut [u8; 7], tenths: u16) {
    let mut num = [b' '; 5];
    format_tenths(&mut num, tenths);
    buf[0..5].copy_from_slice(&num);
    buf[5] = b'M';
    buf[6] = b'S';
}

/// Formats integer value up to 9999 right-aligned into 4 bytes.
pub fn format_u16_dec(buf: &mut [u8; 4], val: u16) {
    let th = (val / 1000) as u8;
    let h = ((val / 100) % 10) as u8;
    let t = ((val / 10) % 10) as u8;
    let o = (val % 10) as u8;
    buf[0] = if th > 0 { b'0' + th } else { b' ' };
    buf[1] = if th > 0 || h > 0 { b'0' + h } else { b' ' };
    buf[2] = if th > 0 || h > 0 || t > 0 {
        b'0' + t
    } else {
        b' '
    };
    buf[3] = b'0' + o;
}

/// Formats speed into " 84MPH" 7-byte buffer.
pub fn format_speed_short(buf: &mut [u8; 7], mph: u16) {
    let h = (mph / 100) as u8;
    let t = ((mph / 10) % 10) as u8;
    let o = (mph % 10) as u8;
    buf[0] = if h > 0 { b'0' + h } else { b' ' };
    buf[1] = if h > 0 || t > 0 { b'0' + t } else { b' ' };
    buf[2] = b'0' + o;
    buf[3] = b' ';
    buf[4] = b'M';
    buf[5] = b'P';
    buf[6] = b'H';
}
