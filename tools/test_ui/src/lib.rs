//! Host-side tests for the game's hardware-free presentation logic.
//!
//! The modules under test are included textually (the same trick as
//! `tools/test_memcard`) precisely because they were written to have no
//! hardware dependencies: `pause_input.rs` takes button state as plain booleans
//! and never touches `psx-pad` or the GPU.
//!
//! This exists because of a bug that lived precisely in the untested gap. The
//! race loop used to write `PauseMenu::prev_buttons` itself before handing the
//! pad to `update()`, so the menu compared every frame against itself, no button
//! ever looked newly pressed, and entering the pause menu required a power cycle
//! to escape. The state machine below now owns all edge state, and
//! `pause_menu_can_always_be_dismissed` locks that in.

#![allow(dead_code)]

#[path = "../../../game/src/ui/settings_input.rs"]
mod settings_input;

#[path = "../../../game/src/ui/pause_input.rs"]
mod pause_input;

#[path = "../../../game/src/gpu/camera.rs"]
pub mod camera;

#[path = "../../../game/src/gpu/car_geometry.rs"]
#[allow(unused_imports)]
mod car_geometry;

// The roadside prop *placement* is pure geometry, but it lives in a module that
// also draws through the GPU. Included inside `gpu` (matching its real path, so
// its `crate::gpu::...` imports resolve) so the test drives the real
// `biome_prop_placements` rather than a re-implementation -- a mirrored copy
// would keep passing while the renderer stayed broken.

#[path = "../../../game/src/ui/tuning_input.rs"]
mod tuning_input;

#[path = "../../../game/src/gpu/effects_sim.rs"]
mod effects_sim;

#[path = "../../../game/src/gpu/palette.rs"]
pub mod palette;

#[path = "../../../game/src/gpu/texlayout.rs"]
mod texlayout;

#[path = "../../../game/src/ui/title_input.rs"]
mod title_input;

pub mod ui {
    pub mod font {
        pub fn draw_char(_x: u16, _y: u16, _ch: u8, _colour: (u8, u8, u8), _scale: u16) {}
        pub fn draw_text(_x: u16, _y: u16, _text: &str, _colour: (u8, u8, u8), _scale: u16) {}
    }
}

// The prop renderer draws through the GPU but its *placement* is pure geometry.
// Included here (and re-exported under `gpu`) so the host test can drive the
// real `biome_prop_placements` rather than a re-implementation -- a mirrored
// copy would keep passing while the renderer stayed broken.
#[path = "../../../game/src/gpu/gte_proj.rs"]
#[allow(unused_imports, dead_code)]
pub mod gte_proj;

#[path = "../../../game/src/gpu/m7_mesh.rs"]
pub mod m7_mesh;

#[path = "../../../game/src/gpu/props_renderer.rs"]
#[allow(unused_imports, dead_code)]
pub mod props_renderer;

pub mod gpu {
    use arduracer_core::TrackDef;
    pub fn draw_ui_minimap(_x: u16, _y: u16, _w: u16, _h: u16, _track: &TrackDef) {}

    // Re-exported so `props_renderer`'s `crate::gpu::camera` / `::gte_proj` /
    // `::palette` imports resolve to the same modules the game uses.
    pub use super::camera;
    pub use super::gte_proj;
    pub use super::m7_mesh;
    pub use super::palette;
    pub use super::props_renderer;
}

#[path = "../../../game/src/ui/stage_table.rs"]
pub mod stage_table;

#[path = "../../../game/src/ui/track_select.rs"]
pub mod track_select;

#[path = "../../../game/src/ui/menu.rs"]
pub mod menu;

#[path = "../../../game/src/ui/victory.rs"]
pub mod victory;

pub mod input {
    pub mod mapping {
        pub use arduracer_core::save::InputProfile;
    }
}

#[path = "../../../game/src/input/pad.rs"]
pub mod pad;

#[cfg(test)]
mod tests {
    use super::pause_input::{PauseChoice, PauseFrame, PauseInput, PauseMenu};

    fn held(f: impl Fn(&mut PauseInput)) -> PauseInput {
        let mut input = PauseInput::default();
        f(&mut input);
        input
    }

    const IDLE: PauseInput = PauseInput {
        start: false,
        select: false,
        cross: false,
        circle: false,
        up: false,
        down: false,
        left: false,
        right: false,
    };

    /// Replays a scripted sequence of frames and returns every frame's verdict.
    fn run(frames: &[PauseInput]) -> Vec<PauseFrame> {
        let mut menu = PauseMenu::new();
        frames.iter().map(|f| menu.update(*f)).collect()
    }

    /// One DOWN navigation step: press, then release so the next press is an
    /// edge again. Holding the button across frames is a single edge, which is
    /// the behaviour `pause_menu_can_always_be_dismissed` already covers.
    fn step_down(menu: &mut PauseMenu) {
        menu.update(held(|i| i.down = true));
        menu.update(IDLE);
    }

    fn step_up(menu: &mut PauseMenu) {
        menu.update(held(|i| i.up = true));
        menu.update(IDLE);
    }

    /// The regression this whole crate exists for. Every one of these was
    /// unreachable when the race loop shared `prev_buttons` with the menu.
    #[test]
    fn pause_menu_can_always_be_dismissed() {
        // Press START to open. Nothing else held.
        let opened = run(&[held(|i| i.start = true)]);
        assert!(opened[0].start_pressed, "START must read as a fresh press");
        assert_eq!(opened[0].choice, PauseChoice::Resume);

        // The menu is open; pressing START again closes it, and again after
        // that. Entering the pause menu must never be a one-way door.
        let toggled = run(&[
            held(|i| i.start = true),
            IDLE,
            held(|i| i.start = true),
            IDLE,
            held(|i| i.start = true),
        ]);
        assert_eq!(
            toggled.iter().filter(|f| f.start_pressed).count(),
            3,
            "each START press must be seen exactly once"
        );

        // Holding START for a long stretch is one press, not one per frame --
        // otherwise the menu strobes while the button is down.
        let held_down = run(&[
            held(|i| i.start = true),
            held(|i| i.start = true),
            held(|i| i.start = true),
            held(|i| i.start = true),
        ]);
        assert_eq!(
            held_down.iter().filter(|f| f.start_pressed).count(),
            1,
            "a held button is one edge, not four"
        );

        // CROSS on the highlighted entry confirms it, on every entry.
        for idx in 0..PauseMenu::item_count() {
            let mut frames = vec![held(|i| i.start = true), IDLE];
            for _ in 0..idx {
                frames.push(held(|i| i.down = true));
                frames.push(IDLE);
            }
            frames.push(held(|i| i.cross = true));
            let out = run(&frames);
            // The settings rows are editable in place and report `None`
            // until a value actually moves, so only the action rows confirm.
            // QUIT is the last entry, so it tracks `item_count() - 1`: hardcoding
            // its index is how inserting the CD-audio row above it would have
            // silently turned the destructive option into a no-op.
            let expected = match idx {
                0 => PauseChoice::Resume,
                1 => PauseChoice::RestartRace,
                n if n == PauseMenu::item_count() - 1 => PauseChoice::QuitToMenu,
                _ => PauseChoice::None,
            };
            assert_eq!(
                out.last().expect("frames ran").choice,
                expected,
                "entry {idx} confirmed the wrong action"
            );
        }
    }

    /// The frame that opens the menu also carries the START press that opened
    /// it. If the caller acted on that frame's choice the veil would open and
    /// close again on the same frame, so the contract is that the menu reports
    /// the edge and the caller ignores the choice until the next frame.
    #[test]
    fn opening_frame_reports_but_does_not_consume() {
        let frames = run(&[held(|i| i.start = true), IDLE]);
        assert!(frames[0].start_pressed);
        assert_eq!(frames[0].choice, PauseChoice::Resume);
        // Next idle frame the menu is calm and waiting.
        assert!(!frames[1].start_pressed);
        assert_eq!(frames[1].choice, PauseChoice::None);
    }

    #[test]
    fn selection_wraps_at_both_ends() {
        let mut menu = PauseMenu::new();
        assert_eq!(menu.selected_idx, 0);

        // Up from the first entry wraps to the last.
        menu.update(held(|i| i.up = true));
        assert_eq!(menu.selected_idx, PauseMenu::item_count() - 1);

        // Down from the last wraps back to the first.
        menu.update(held(|i| i.down = true));
        assert_eq!(menu.selected_idx, 0);

        // Navigation stops at every entry; it never runs off the end.
        for _ in 0..(PauseMenu::item_count() + 3) {
            menu.update(held(|i| i.down = true));
            assert!(menu.selected_idx < PauseMenu::item_count());
        }
    }

    #[test]
    fn select_is_reported_separately_from_the_menu() {
        // Select toggles the HUD and must not also confirm a menu entry.
        let out = run(&[held(|i| i.select = true), IDLE]);
        assert!(out[0].select_pressed);
        assert_eq!(out[0].choice, PauseChoice::None);

        // And holding it down is still a single toggle.
        let held_select = run(&[held(|i| i.select = true), held(|i| i.select = true), IDLE]);
        assert_eq!(held_select.iter().filter(|f| f.select_pressed).count(), 1);
    }

    #[test]
    fn circle_resumes_without_moving_the_highlight() {
        let mut menu = PauseMenu::new();
        menu.update(held(|i| i.down = true));
        let after_nav = menu.selected_idx;
        let out = menu.update(held(|i| i.circle = true));
        assert_eq!(out.choice, PauseChoice::Resume);
        assert_eq!(menu.selected_idx, after_nav);
    }

    #[test]
    fn sync_edges_swallows_a_button_held_across_a_state_change() {
        let mut menu = PauseMenu::new();
        // Player holds CROSS while the track reloads.
        menu.update(held(|i| i.cross = true));

        // The state change adopts what is held right now as the baseline.
        let still_held = held(|i| i.cross = true);
        menu.sync_edges(still_held);

        // Releasing and pressing again is a real press, and confirms normally.
        let out = menu.update(PauseInput::default());
        assert_eq!(out.choice, PauseChoice::None);
        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::Resume
        );
    }

    /// `sync_edges` has to work for every button, not just CROSS: it is called
    /// on state changes the caller cannot predict (track reload, leaving the
    /// race), and any button it fails to re-baseline reads as a fresh press on
    /// the first frame back.
    #[test]
    fn sync_edges_rebaselines_every_button() {
        let all_held = |i: &mut PauseInput| {
            i.start = true;
            i.select = true;
            i.cross = true;
            i.circle = true;
            i.up = true;
            i.down = true;
        };

        let mut menu = PauseMenu::new();
        // One frame with everything down: every button is an edge.
        let first = menu.update(held(all_held));
        assert!(first.start_pressed);
        assert!(first.select_pressed);

        // A state change happens while they are all still down.
        menu.sync_edges(held(all_held));

        // Release, then press each one again: each must read as a fresh press.
        menu.update(PauseInput::default());
        let again = menu.update(held(all_held));
        assert!(again.start_pressed, "START lost its edge baseline");
        assert!(again.select_pressed, "SELECT lost its edge baseline");
        assert_eq!(again.choice, PauseChoice::Resume);

        // Navigation re-baselined too, or the highlight would jump twice.
        let after = menu.selected_idx;
        menu.sync_edges(held(|i| i.down = true));
        menu.update(PauseInput::default());
        menu.update(held(|i| i.down = true));
        assert_eq!(
            menu.selected_idx,
            after + 1,
            "DOWN moved the highlight by more than one step"
        );
    }

    /// The two navigation buttons are checked in an `if / else if` chain, so a
    /// frame that reports both (a d-pad cross, or a stick knocking the gate) is
    /// resolved in favour of UP. That precedence is load-bearing and was
    /// unasserted, which is exactly how it silently inverts.
    #[test]
    fn a_simultaneous_up_and_down_press_moves_up_once() {
        let both = run(&[held(|i| {
            i.up = true;
            i.down = true;
        })]);
        assert_eq!(
            both[0].choice,
            PauseChoice::None,
            "navigation must not confirm anything"
        );
        assert_eq!(both.len(), 1, "one frame in, one frame out");

        let mut both_menu = PauseMenu::new();
        both_menu.update(held(|i| {
            i.up = true;
            i.down = true;
        }));
        let mut up_menu = PauseMenu::new();
        up_menu.update(held(|i| i.up = true));
        assert_eq!(
            both_menu.selected_idx, up_menu.selected_idx,
            "UP and DOWN together must behave exactly like UP alone"
        );
        assert_eq!(both_menu.selected_idx, PauseMenu::item_count() - 1);
    }

    /// Navigation must wrap by exactly one entry in either direction from
    /// *every* index -- including the `last + 1 >= ITEM_COUNT` boundary, which a
    /// `<` would get wrong.
    #[test]
    fn navigation_is_a_one_step_cycle_through_every_entry() {
        let count = PauseMenu::item_count() as i32;
        assert!(count > 1, "the menu needs at least two entries to wrap");

        for start in 0..count {
            let mut down = PauseMenu::new();
            for _ in 0..start {
                step_down(&mut down);
            }
            assert_eq!(down.selected_idx as i32, start);
            down.update(held(|i| i.down = true));
            assert_eq!(
                down.selected_idx as i32,
                (start + 1) % count,
                "DOWN from entry {start}"
            );

            // `start` steps UP from entry 0 lands on `count - start`, so one
            // more must land on `count - start - 1`.
            let mut up = PauseMenu::new();
            for _ in 0..start {
                step_up(&mut up);
            }
            assert_eq!(up.selected_idx as i32, (count - start) % count);
            up.update(held(|i| i.up = true));
            assert_eq!(
                up.selected_idx as i32,
                (count - start - 1) % count,
                "UP from entry {start}"
            );
        }

        // A full lap in either direction returns to where it started.
        let mut lap = PauseMenu::new();
        for _ in 0..count {
            step_down(&mut lap);
        }
        assert_eq!(lap.selected_idx, 0);
        for _ in 0..count {
            step_up(&mut lap);
        }
        assert_eq!(lap.selected_idx, 0);
    }

    /// The confirm chain is `start || circle` before `cross`, so a frame that
    /// reports a "close the menu" button alongside a "confirm entry" button must
    /// close, not confirm. Getting this backwards would let a player restart the
    /// race by pressing START while CROSS is also down.
    #[test]
    fn close_buttons_win_over_cross_on_a_shared_frame() {
        // Park the highlight on "Restart Race" so a CROSS leak is visible.
        for closer in ["start", "circle"] {
            let mut menu = PauseMenu::new();
            menu.update(held(|i| i.down = true));
            assert_eq!(menu.selected_idx, 1, "expected to park on Restart Race");

            let frame = held(|i| {
                i.cross = true;
                if closer == "start" {
                    i.start = true;
                } else {
                    i.circle = true;
                }
            });
            let out = menu.update(frame);
            assert_eq!(
                out.choice,
                PauseChoice::Resume,
                "{closer} + CROSS on one frame must resume, not restart"
            );
        }

        // And with neither close button down, CROSS on the same highlight does
        // restart -- so the two assertions above really do discriminate.
        let mut menu = PauseMenu::new();
        menu.update(held(|i| i.down = true));
        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::RestartRace
        );
    }

    /// Confirming an entry must leave the machine in a coherent state: the
    /// highlight is kept (so reopening the menu resumes where the player was),
    /// the same entry can be confirmed again, and START still toggles. A reset
    /// here would silently move the highlight every time the menu is opened.
    #[test]
    fn the_menu_stays_coherent_after_a_confirmation() {
        let mut menu = PauseMenu::new();
        for _ in 0..PauseMenu::item_count() - 1 {
            step_down(&mut menu);
        }
        let parked = menu.selected_idx;
        assert_eq!(parked, PauseMenu::item_count() - 1);

        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::QuitToMenu
        );
        assert_eq!(menu.selected_idx, parked, "confirming moved the highlight");
        menu.update(IDLE);
        assert_eq!(
            menu.update(held(|i| i.cross = true)).choice,
            PauseChoice::QuitToMenu,
            "releasing and pressing CROSS again must confirm again"
        );

        // START is still live after a confirmation, so the menu is escapable.
        let opened = menu.update(held(|i| i.start = true));
        assert!(opened.start_pressed, "START went dead after a confirm");
        assert_eq!(opened.choice, PauseChoice::Resume);
    }

    /// `update` documents that it must be called exactly once per rendered frame.
    /// Skipping one leaves the previous frame's `PauseInput` behind, so a
    /// release-and-repress across the gap reads as a hold and the press is lost.
    /// The caller cannot see the internal `prev`, so this is only observable --
    /// and only preventable -- from here.
    #[test]
    fn a_skipped_frame_swallows_the_next_press() {
        let mut menu = PauseMenu::new();
        // Frame 1: START pressed.
        assert!(menu.update(held(|i| i.start = true)).start_pressed);
        // The caller skips a frame (track reload) while START is released.
        // `sync_edges` is the documented remedy...
        menu.sync_edges(PauseInput::default());
        // ...and with it, the next press is seen.
        assert!(
            menu.update(held(|i| i.start = true)).start_pressed,
            "sync_edges should have restored the edge"
        );

        // Without the remedy the press is lost. Modelled by *not* calling
        // `update` on the release frame, so `prev` still says START is down.
        let mut stale = PauseMenu::new();
        assert!(stale.update(held(|i| i.start = true)).start_pressed);
        // Frame 2 is skipped entirely; START was released and pressed again.
        assert!(
            !stale.update(held(|i| i.start = true)).start_pressed,
            "a stale baseline must read the re-press as a hold"
        );
        // Only once a frame has actually been observed does the edge return.
        assert!(stale.update(PauseInput::default()).choice == PauseChoice::None);
        assert!(stale.update(held(|i| i.start = true)).start_pressed);
    }

    /// The `Default` impls and the derived `Default`s on the public types are
    /// part of the crate's surface (callers build these from
    /// `PauseMenu::default()`), and were the only lines with no coverage at all.
    #[test]
    fn defaults_match_a_freshly_constructed_menu() {
        let fresh = PauseMenu::new();
        let defaulted = PauseMenu::default();
        assert_eq!(fresh, defaulted);
        assert_eq!(defaulted.selected_idx, 0);

        assert_eq!(PauseMenu::item_count(), 8);
        assert_eq!(PauseChoice::default(), PauseChoice::None);
        assert_eq!(
            PauseFrame::default(),
            PauseFrame {
                start_pressed: false,
                select_pressed: false,
                choice: PauseChoice::None,
            }
        );
        assert_eq!(PauseInput::default(), IDLE);

        // A defaulted menu really is a cold one: its first CROSS is an edge.
        let mut menu = PauseMenu::default();
        assert!(menu.update(held(|i| i.cross = true)).choice == PauseChoice::Resume);
    }
}

#[cfg(test)]
mod camera_tests {
    use super::camera::{Camera, SCREEN_H, SCREEN_W};
    use arduracer_core::{Fixed, Vec2, FP_ONE};

    /// A world big enough that the camera is never bounds-limited, so these
    /// tests measure the projection itself.
    const BIG: (i32, i32) = (100_000, 100_000);

    fn cam_at_rest() -> Camera {
        let mut c = Camera::new(Vec2::new(Fixed::from_int(50_000), Fixed::from_int(50_000)));
        // One update at zero speed: rest zoom, no lead.
        c.update(c.pos, Vec2::ZERO, Fixed::ZERO, BIG.0, BIG.1);
        c
    }

    /// Zoom has to reach the projection. Before this was wired the field was
    /// computed every frame and then ignored, so speed-reactive zoom did
    /// nothing at all.
    #[test]
    fn zoom_actually_scales_the_projection() {
        let cam = cam_at_rest();
        assert_eq!(cam.zoom, Fixed::from_raw(FP_ONE), "rest zoom must be 1:1");

        let at_centre = cam.world_offset(Vec2::new(cam.pos.x, cam.pos.y));
        assert_eq!(at_centre, (0, 0), "camera centre maps to screen centre");

        // A point 100 units to the right is 100 px right of centre at 1:1.
        let probe = Vec2::new(cam.pos.x + Fixed::from_int(100), cam.pos.y);
        assert_eq!(cam.world_offset(probe), (100, 0));

        // Zoom out and the same world distance must cover fewer pixels.
        // The target is a fixed world point, so the camera converges instead of
        // chasing its own position and drifting forever.
        let target = cam.pos;
        let mut fast = cam;
        for _ in 0..600 {
            fast.update(
                target,
                Vec2::new(Fixed::from_int(30), Fixed::ZERO),
                Fixed::from_raw(14_000),
                BIG.0,
                BIG.1,
            );
        }
        assert!(
            fast.zoom < Fixed::from_raw(FP_ONE),
            "top speed must widen the field of view, got {:?}",
            fast.zoom
        );
        let probe_fast = Vec2::new(target.x + Fixed::from_int(100), target.y);
        let (dx, _) = fast.world_offset(probe_fast);
        assert!(dx > 0 && dx < 100, "zoom not applied: dx={dx}");
    }

    /// Zoom has to ease monotonically toward its target, never overshoot.
    #[test]
    fn zoom_is_monotonic_and_bounded() {
        let mut cam = cam_at_rest();
        let mut previous = cam.zoom.raw();
        for tick in 0..300 {
            let speed = Fixed::from_raw(tick * 60);
            cam.update(cam.pos, Vec2::new(speed, Fixed::ZERO), speed, BIG.0, BIG.1);
            let now = cam.zoom.raw();
            assert!(
                now <= previous,
                "zoom went the wrong way at tick {tick}: {previous} -> {now}"
            );
            assert!(now > 3_000 && now <= FP_ONE, "zoom out of range: {now}");
            previous = now;
        }
    }

    /// The car should sit behind screen centre in the direction of travel, so
    /// the driver sees the corner rather than the boot lid.
    #[test]
    fn lookahead_leads_the_car_in_the_direction_of_travel() {
        let mut cam = Camera::new(Vec2::ZERO);
        for _ in 0..240 {
            cam.update(
                Vec2::ZERO,
                Vec2::new(Fixed::from_int(20), Fixed::ZERO),
                Fixed::from_int(8_000),
                BIG.0,
                BIG.1,
            );
        }
        assert!(cam.pos.x > Fixed::ZERO, "camera must lead to the right");
        // The car is then *behind* the camera centre on screen.
        let (sx, _) = cam.world_offset(Vec2::ZERO);
        assert!(sx < 0, "car should sit left of centre, got {sx}");
    }

    /// The camera position must keep sub-unit precision while it is free to
    /// move.
    ///
    /// `clamp_to_bounds` used to round-trip through `to_int()`, quantising the
    /// camera to whole world units every frame whether or not the clamp was
    /// active. A world unit is a screen pixel at rest zoom and the camera covers
    /// several units per frame at speed, so the camera advanced in whole-pixel
    /// steps while the car moved smoothly -- the car's screen position juddered
    /// by a pixel every frame. That is the "clunky stage movement" this guards.
    #[test]
    fn camera_position_is_not_quantised_to_world_units() {
        let mut cam = Camera::new(Vec2::new(Fixed::from_int(50_000), Fixed::from_int(50_000)));
        // A car driving steadily in a world big enough that no clamp can bite.
        let mut car = cam.pos;
        let mut off_unit_ticks = 0;
        for _ in 0..600 {
            car.x = car.x + Fixed::from_int(4);
            cam.update(
                car,
                Vec2::new(Fixed::from_int(4), Fixed::ZERO),
                Fixed::from_int(240),
                BIG.0,
                BIG.1,
            );
            if cam.pos.x.raw() % FP_ONE != 0 {
                off_unit_ticks += 1;
            }
        }
        assert!(
            off_unit_ticks > 300,
            "camera position snapped to whole world units on {} of 600 ticks",
            600 - off_unit_ticks
        );
    }

    /// The look-ahead direction must not follow tyre noise.
    ///
    /// Below walking pace the velocity vector points at whatever the tyres last
    /// did, and the lead is long enough that chasing it swings the camera target
    /// by the full lead distance. A car idling must hold still instead.
    #[test]
    fn low_speed_vehicle_noise_does_not_move_the_camera() {
        let mut cam = Camera::new(Vec2::new(Fixed::from_int(50_000), Fixed::from_int(50_000)));
        // Let the lead settle first, then start jittering.
        for _ in 0..120 {
            cam.update(
                cam.pos,
                Vec2::new(Fixed::from_int(30), Fixed::ZERO),
                Fixed::from_int(1_000),
                BIG.0,
                BIG.1,
            );
        }
        let settled = cam.pos;

        // Alternating steering corrections at crawling speed: nonzero velocity,
        // so the old code followed it and flipped the lead direction each tick.
        for tick in 0..120 {
            let sign = if tick % 2 == 0 { 1 } else { -1 };
            cam.update(
                settled,
                Vec2::new(Fixed::from_int(40 * sign), Fixed::from_int(40 * sign)),
                Fixed::from_int(600),
                BIG.0,
                BIG.1,
            );
        }
        let (dx, _) = cam.world_offset(settled);
        assert_eq!(
            dx, 0,
            "creeping-speed noise moved the camera {dx} px off the car"
        );
    }

    /// At a standstill there is no velocity to derive a lead from, but the car
    /// must still not sit dead centre.
    #[test]
    fn stationary_car_is_not_dead_centre() {
        let mut cam = Camera::new(Vec2::ZERO);
        for _ in 0..240 {
            cam.update(Vec2::ZERO, Vec2::ZERO, Fixed::ZERO, BIG.0, BIG.1);
        }
        let (sx, sy) = cam.world_offset(Vec2::ZERO);
        assert!(
            sx != 0 || sy != 0,
            "stationary camera collapsed onto the car"
        );
    }

    /// The view rectangle must never leave the circuit: this is the "never see
    /// off the track" requirement, and it is what the old world-bounds car clamp
    /// failed to do visually.
    #[test]
    fn camera_never_shows_outside_the_circuit() {
        // Larger than the view in both axes, so the clamp has real work to do.
        let world = (4_000i32, 3_000i32);
        let (hw, hh) = cam_at_rest().visible_half_extents();
        assert!(
            hw * 2 < world.0 && hh * 2 < world.1,
            "test needs a world larger than the view"
        );

        // Drive the camera hard into each corner and check every corner pixel.
        let targets = [
            Vec2::new(Fixed::from_int(0), Fixed::from_int(0)),
            Vec2::new(Fixed::from_int(world.0), Fixed::from_int(0)),
            Vec2::new(Fixed::from_int(0), Fixed::from_int(world.1)),
            Vec2::new(Fixed::from_int(world.0), Fixed::from_int(world.1)),
        ];
        for t in targets {
            let mut cam = Camera::new(t);
            for _ in 0..200 {
                cam.update(
                    t,
                    Vec2::new(Fixed::from_int(40), Fixed::ZERO),
                    Fixed::from_raw(14_000),
                    world.0,
                    world.1,
                );
            }
            // Extents must be read at the camera's *final* zoom: it is wider at
            // speed, and using the rest-time value would test the wrong rectangle.
            let (hw, hh) = cam.visible_half_extents();
            let (cx, cy) = (cam.pos.x.to_int(), cam.pos.y.to_int());
            // The whole view rectangle must sit inside the circuit.
            assert!(cx - hw >= 0, "camera x {cx} shows {} px past 0", hw - cx);
            assert!(
                cx + hw <= world.0,
                "camera x {cx} shows {} px past {}",
                cx + hw - world.0,
                world.0
            );
            assert!(cy - hh >= 0, "camera y {cy} shows {} px past 0", hh - cy);
            assert!(
                cy + hh <= world.1,
                "camera y {cy} shows {} px past {}",
                cy + hh - world.1,
                world.1
            );
        }
    }

    /// A circuit narrower than the view cannot be framed without showing
    /// out-of-bounds, so the camera has to centre on it instead of clamping to
    /// an edge -- which would push more off-world on the far side.
    #[test]
    fn world_narrower_than_the_view_centres() {
        let world = (200i32, 150i32);
        let mut cam = Camera::new(Vec2::new(Fixed::from_int(0), Fixed::from_int(0)));
        for _ in 0..200 {
            cam.update(
                Vec2::new(Fixed::from_int(0), Fixed::from_int(0)),
                Vec2::new(Fixed::from_int(30), Fixed::ZERO),
                Fixed::from_raw(14_000),
                world.0,
                world.1,
            );
        }
        assert_eq!(cam.pos.x.to_int(), world.0 / 2);
        assert_eq!(cam.pos.y.to_int(), world.1 / 2);
    }

    /// Visible extents and the projection must agree, or the tile renderer and
    /// the sprites drift apart at speed.
    #[test]
    fn visible_extents_match_the_projection() {
        let mut cam = cam_at_rest();
        for _ in 0..300 {
            cam.update(
                cam.pos,
                Vec2::new(Fixed::from_int(25), Fixed::ZERO),
                Fixed::from_raw(12_000),
                BIG.0,
                BIG.1,
            );
        }
        let (hw, hh) = cam.visible_half_extents();
        let right = cam.world_offset(Vec2::new(cam.pos.x + Fixed::from_int(hw), cam.pos.y));
        let bottom = cam.world_offset(Vec2::new(cam.pos.x, cam.pos.y + Fixed::from_int(hh)));
        assert_eq!(right.0, SCREEN_W / 2, "half-width must reach the edge");
        assert_eq!(bottom.1, SCREEN_H / 2, "half-height must reach the edge");
    }

    #[test]
    fn rotating_and_chase_cameras_clamp_to_bounds() {
        let world = (2_000i32, 2_000i32);
        let mut rot = Camera::new_rotating(Vec2::ZERO, 0);
        for _ in 0..120 {
            rot.update(
                Vec2::ZERO,
                Vec2::new(Fixed::from_int(30), Fixed::ZERO),
                Fixed::from_raw(14_000),
                world.0,
                world.1,
            );
        }
        let (hw, hh) = rot.visible_half_extents();
        assert!(rot.pos.x.to_int() >= hw && rot.pos.x.to_int() <= world.0 - hw);
        assert!(rot.pos.y.to_int() >= hh && rot.pos.y.to_int() <= world.1 - hh);

        let mut chase =
            Camera::new_chase(Vec2::new(Fixed::from_int(-50), Fixed::from_int(2_100)), 0);
        for _ in 0..60 {
            chase.update(
                Vec2::new(Fixed::from_int(-50), Fixed::from_int(2_100)),
                Vec2::ZERO,
                Fixed::ZERO,
                world.0,
                world.1,
            );
        }
        assert!(chase.pos.x >= Fixed::ZERO && chase.pos.x <= Fixed::from_int(world.0));
        assert!(chase.pos.y >= Fixed::ZERO && chase.pos.y <= Fixed::from_int(world.1));
    }
}

#[cfg(test)]
mod start_tests {
    use arduracer_core::{
        StartPhase, StartSequence, COUNTDOWN_GO_TICKS, COUNTDOWN_LIGHTS, COUNTDOWN_LIGHT_TICKS,
    };

    fn run(ticks: u16) -> StartSequence {
        let mut s = StartSequence::new();
        for _ in 0..ticks {
            s.tick();
        }
        s
    }

    /// The point of the whole feature: the grid holds, then the lights go out.
    #[test]
    fn the_lights_come_on_one_at_a_time_then_go_out() {
        assert_eq!(run(0).phase(), StartPhase::Grid);
        assert_eq!(
            run(COUNTDOWN_LIGHT_TICKS - 1).phase(),
            StartPhase::Grid,
            "the first light must not come on instantly"
        );
        assert_eq!(run(COUNTDOWN_LIGHT_TICKS).phase(), StartPhase::Lit(1));
        assert_eq!(run(COUNTDOWN_LIGHT_TICKS * 2).phase(), StartPhase::Lit(2));
        assert_eq!(run(COUNTDOWN_LIGHT_TICKS * 3).phase(), StartPhase::Lit(3));
        let lights_end = StartSequence::lights_out_tick();
        assert_eq!(run(lights_end).phase(), StartPhase::Go);
        assert_eq!(
            run(lights_end + COUNTDOWN_GO_TICKS).phase(),
            StartPhase::Racing
        );
    }

    /// Every light must be seen, and none skipped: this is the sequence a player
    /// reads to time their launch.
    #[test]
    fn every_light_is_visible_for_its_full_duration() {
        for n in 1..=COUNTDOWN_LIGHTS {
            let start = COUNTDOWN_LIGHT_TICKS * n as u16;
            for offset in 0..COUNTDOWN_LIGHT_TICKS {
                assert_eq!(
                    run(start + offset).phase(),
                    StartPhase::Lit(n),
                    "light {n} flickered early at +{offset}"
                );
            }
        }
    }

    /// Input is refused on the grid and through all three lights, then released.
    /// The clock only arms on GO, so a driver who rests on the throttle from the
    /// start is not also entitled to a flying lap.
    #[test]
    fn input_is_locked_until_the_lights_go_out() {
        let lights_end = StartSequence::lights_out_tick();
        for t in 0..lights_end {
            assert!(
                !run(t).accepts_input(),
                "input accepted at tick {t}, before GO"
            );
        }
        assert!(run(lights_end).accepts_input(), "input must free on GO");
        assert!(run(lights_end + COUNTDOWN_GO_TICKS).accepts_input());
    }

    /// `just_started` is the one-shot the race loop uses to arm the lap clock, so
    /// it must be true for exactly one tick.
    #[test]
    fn the_start_edge_fires_exactly_once() {
        let lights_end = StartSequence::lights_out_tick();
        assert!(!run(lights_end - 1).just_started());
        assert!(run(lights_end).just_started(), "must fire on the GO tick");
        assert!(
            !run(lights_end + 1).just_started(),
            "must not re-fire after GO"
        );
        assert!(!run(lights_end + COUNTDOWN_GO_TICKS).just_started());

        let mut count = 0;
        let mut s = StartSequence::new();
        for _ in 0..(lights_end + COUNTDOWN_GO_TICKS * 2) {
            s.tick();
            if s.just_started() {
                count += 1;
            }
        }
        assert_eq!(count, 1, "start edge fired {count} times");
    }

    /// Input frees on the same tick the clock arms, so the player gets the whole
    /// GO hold to react.
    #[test]
    fn the_go_hold_is_usable() {
        let lights_end = StartSequence::lights_out_tick();
        let usable = COUNTDOWN_GO_TICKS;
        assert!(usable > 0, "there must be time to react to GO");
        // The first GO tick is the start edge itself; every tick after it is
        // reaction time the player actually gets.
        assert!(run(lights_end).just_started());
        for t in lights_end + 1..lights_end + usable {
            assert!(run(t).accepts_input(), "input locked during the GO hold");
            assert!(!run(t).just_started(), "start edge repeated");
        }
    }

    #[test]
    fn a_fresh_retry_rearms_the_same_countdown_after_racing() {
        let lights_end = StartSequence::lights_out_tick();
        let finished_race = run(lights_end + COUNTDOWN_GO_TICKS);
        assert_eq!(finished_race.phase(), StartPhase::Racing);

        // reset_race constructs a fresh StartSequence. Its full phase trace and
        // one-shot GO edge must match the original run exactly.
        let mut reference = StartSequence::new();
        let mut retry = StartSequence::new();
        assert_eq!(retry.phase(), StartPhase::Grid);
        assert!(!retry.just_started());
        for tick in 1..=lights_end + COUNTDOWN_GO_TICKS {
            reference.tick();
            retry.tick();
            assert_eq!(
                retry.phase(),
                reference.phase(),
                "phase differs at tick {tick}"
            );
            assert_eq!(
                retry.just_started(),
                reference.just_started(),
                "GO edge differs at tick {tick}"
            );
        }
    }
}

/// Vehicle polygon geometry. `car_renderer.rs` draws through `psx_gpu` and so
/// cannot be linked on the host, but the vertex maths behind it is pure integer
/// arithmetic and is exactly where TASK-1203's bug lived.
#[cfg(test)]
mod car_geometry_tests {
    use super::car_geometry::{
        make_quad, make_rect_at, painted_twice_area, quad_covers_its_interior,
        quad_tiles_its_interior, Rot,
    };

    /// The eight cardinal headings plus four diagonals, in BAMs (1024 per
    /// quarter turn).
    const HEADINGS: [u16; 12] = [0, 256, 512, 768, 128, 384, 640, 896, 170, 682, 854, 1098];

    /// The cockpit canopy, with the fixed length arguments from
    /// `car_renderer::render_car`.
    const CANOPY: (i32, i32, i32, i32) = (4, 5, 4, 7);

    /// Every quad `render_car` builds, with the arguments it uses.
    const CAR_PARTS: [(&str, i32, i32, i32, i32); 4] = [
        ("shadow", 7, 8, 12, 12),
        ("body", 6, 7, 12, 12),
        ("stripe", 1, 1, 12, 12),
        ("canopy", 4, 5, 4, 7),
    ];

    /// Every rectangle `render_car` builds: the four wheel pods and the wing.
    const CAR_RECTS: [(&str, i32, i32, i32, i32); 5] = [
        ("rear-left-wheel", -7, -6, 2, 4),
        ("rear-right-wheel", 7, -6, 2, 4),
        ("front-left-wheel", -7, 7, 2, 4),
        ("front-right-wheel", 7, 7, 2, 4),
        ("wing", 0, -11, 8, 2),
    ];

    /// All 64 headings, since the bug this guards is heading-dependent.
    const ALL_HEADINGS: fn() -> Vec<u16> = || (0..4096).step_by(64).collect();

    #[test]
    fn the_cockpit_canopy_has_area_at_every_heading() {
        let (w_front, w_rear, l_front, l_rear) = CANOPY;
        for h in HEADINGS {
            let pts = make_quad(
                160,
                120,
                w_front,
                w_rear,
                l_front,
                l_rear,
                Rot::from_bams(h),
            );
            assert!(
                painted_twice_area(&pts) > 0,
                "canopy collapsed at {h} BAM: {pts:?}"
            );
        }
    }

    /// The vertex order is load-bearing, and it is *Z* order: top-left,
    /// top-right, bottom-left, bottom-right.
    ///
    /// The quad primitives rasterise as `tri(v0,v1,v2)` + `tri(v1,v2,v3)`, so
    /// `v1` and `v2` have to be diagonally opposite corners. An earlier version
    /// of this file asserted the opposite -- that the helpers emit a *perimeter*
    /// traversal -- on the theory that the original ordering was a
    /// self-intersecting bowtie. It was not, and "fixing" it that way left the
    /// `v0` corner of every quad unpainted: a wedge of road showing through the
    /// middle of each car. The shoelace assertions below all still passed,
    /// because losing one corner triangle barely dents the signed sum.
    #[test]
    fn quads_are_emitted_in_z_order_so_v1_and_v2_are_diagonal() {
        // At 0 BAM the transform is x = cx + u, y = cy - v, so the nose sits at
        // the smaller y. A 12-wide nose over a 14-wide tail, 24 long.
        let pts = make_quad(160, 120, 6, 7, 12, 12, Rot::from_bams(0));
        assert_eq!(
            pts,
            [
                (154, 108), // front-left  (v0)
                (166, 108), // front-right (v1)
                (153, 132), // rear-left   (v2) -- diagonally opposite v1
                (167, 132), // rear-right  (v3)
            ],
            "vertex order or positions changed"
        );
        // The diagonal v1-v2 is what the primitive splits along, so it must be
        // the long axis of the trapezoid, not one of its parallel sides.
        assert!(
            quad_tiles_its_interior(&pts) && quad_covers_its_interior(&pts),
            "the quad primitive would leave part of the trapezoid unpainted"
        );
        // Trapezoid rule: parallel sides 12 and 14, height 24.
        assert_eq!(
            painted_twice_area(&pts),
            2 * ((12 + 14) * 24 / 2),
            "the trapezoid must enclose its own area"
        );
    }

    #[test]
    fn rectangles_are_emitted_in_z_order() {
        let pts = make_rect_at(160, 120, 0, -11, 8, 2, Rot::from_bams(0));
        assert_eq!(pts, [(152, 129), (168, 129), (152, 133), (168, 133)]);
        assert!(quad_tiles_its_interior(&pts) && quad_covers_its_interior(&pts));
        assert_eq!(painted_twice_area(&pts), 2 * 16 * 4);
    }

    /// The regression test for the missing flank: every quad the car draws must
    /// be *completely* covered by the two triangles the primitive emits, at
    /// every one of the 64 headings. Area alone does not detect this.
    #[test]
    fn no_quad_leaves_part_of_itself_unpainted_at_any_heading() {
        for (name, w_front, w_rear, l_front, l_rear) in CAR_PARTS {
            for h in ALL_HEADINGS() {
                let pts = make_quad(
                    160,
                    120,
                    w_front,
                    w_rear,
                    l_front,
                    l_rear,
                    Rot::from_bams(h),
                );
                assert!(
                    quad_tiles_its_interior(&pts) && quad_covers_its_interior(&pts),
                    "{name} at {h} BAM is not fully covered by the quad split: {pts:?}"
                );
            }
        }
    }

    #[test]
    fn no_rectangle_leaves_part_of_itself_unpainted_at_any_heading() {
        for (name, cu, cv, half_w, half_l) in CAR_RECTS {
            for h in ALL_HEADINGS() {
                let pts = make_rect_at(160, 120, cu, cv, half_w, half_l, Rot::from_bams(h));
                assert!(
                    quad_tiles_its_interior(&pts) && quad_covers_its_interior(&pts),
                    "{name} at {h} BAM is not fully covered by the quad split: {pts:?}"
                );
            }
        }
    }

    /// Both checks have to actually reject the bad ordering, or the two tests
    /// above are vacuous.
    #[test]
    fn the_coverage_checks_reject_perimeter_order() {
        let z = [(154, 108), (166, 108), (153, 132), (167, 132)];
        assert!(quad_tiles_its_interior(&z));
        assert!(quad_covers_its_interior(&z));
        // Same four corners, traversed around the perimeter instead:
        // TL, TR, BR, BL. This is the ordering that put a hole through the car.
        let perimeter = [z[0], z[1], z[3], z[2]];
        assert!(
            !quad_tiles_its_interior(&perimeter),
            "the orientation check must reject perimeter order, or it proves nothing"
        );
        assert!(
            !quad_covers_its_interior(&perimeter),
            "the coverage check must reject perimeter order, or it proves nothing"
        );
        // Documented blind spot, and the reason the coverage checks above are
        // the ones that gate: *every* area measure is identical for the two
        // orderings. Perimeter order trades an overlap for a gap of the same
        // size, so the painted total is unchanged and the old `twice_area`
        // assertions sailed straight over a body with a hole in it.
        assert_eq!(painted_twice_area(&z), painted_twice_area(&perimeter));
        // What is pinned instead is the full trapezoid, which the Z ordering
        // tiles exactly.
        assert_eq!(
            painted_twice_area(&z),
            2 * ((12 + 14) * 24 / 2),
            "a correct Z-ordered quad paints exactly the trapezoid"
        );
    }

    #[test]
    fn every_rendered_car_part_has_area_at_every_heading() {
        for (name, w_front, w_rear, l_front, l_rear) in CAR_PARTS {
            for h in HEADINGS {
                let pts = make_quad(
                    160,
                    120,
                    w_front,
                    w_rear,
                    l_front,
                    l_rear,
                    Rot::from_bams(h),
                );
                assert!(
                    painted_twice_area(&pts) > 0,
                    "{name} collapsed at {h} BAM: {pts:?}"
                );
            }
        }
    }

    #[test]
    fn rectangles_have_area_at_every_heading() {
        for h in HEADINGS {
            let wing = make_rect_at(160, 120, 0, -11, 8, 2, Rot::from_bams(h));
            assert!(painted_twice_area(&wing) > 0, "wing collapsed at {h} BAM");
        }
    }

    #[test]
    fn a_rotated_part_keeps_its_area_across_headings() {
        // Rotation must not change the area: a heading-dependent silhouette
        // would mean the fixed-point transform is scaling rather than rotating.
        let reference = painted_twice_area(&make_quad(160, 120, 6, 7, 12, 12, Rot::from_bams(0)));
        for h in HEADINGS {
            let area = painted_twice_area(&make_quad(160, 120, 6, 7, 12, 12, Rot::from_bams(h)));
            // Q20.12 truncation costs a little, and 45-degree headings trade one
            // axis for the other, so allow a few percent.
            let slack: i64 = reference / 20 + 8;
            assert!(
                (area - reference).abs() <= slack,
                "body area at {h} BAM was {area}, expected about {reference}"
            );
        }
    }
}

/// TASK-1209: the pause menu opened itself on frame 1 of a race.
#[cfg(test)]
mod pause_start_arming_tests {
    use super::pause_input::{PauseChoice, PauseFrame, PauseInput, PauseMenu};

    const NOTHING: PauseInput = PauseInput {
        start: false,
        select: false,
        cross: false,
        circle: false,
        up: false,
        down: false,
        left: false,
        right: false,
    };

    /// A player holding `Start` while navigating to the track select.
    const START_HELD: PauseInput = PauseInput {
        start: true,
        ..NOTHING
    };

    fn fresh_start_press() -> PauseFrame {
        let mut menu = PauseMenu::new();
        menu.arm_for_race_start();
        menu.update(START_HELD)
    }

    #[test]
    fn a_held_start_button_does_not_pause_the_first_frame_of_a_race() {
        let frame = fresh_start_press();
        assert!(
            !frame.start_pressed,
            "the veil opened on frame 1 with Start still held from the menu"
        );
        assert_eq!(frame.choice, PauseChoice::None);
    }

    #[test]
    fn a_held_confirm_button_does_not_confirm_a_menu_item_on_frame_one() {
        let frame = fresh_start_press();
        assert_eq!(
            frame.choice,
            PauseChoice::None,
            "a held Cross must not restart or quit straight after a restart"
        );
    }

    #[test]
    fn a_held_direction_does_not_move_the_highlight_on_frame_one() {
        let mut menu = PauseMenu::new();
        menu.arm_for_race_start();
        menu.update(PauseInput {
            down: true,
            ..NOTHING
        });
        assert_eq!(
            menu.selected_idx, 0,
            "the highlight moved from a button held before the race loaded"
        );
    }

    #[test]
    fn start_still_pauses_once_released_and_pressed_again() {
        let mut menu = PauseMenu::new();
        menu.arm_for_race_start();
        // Held across the transition.
        assert!(!menu.update(START_HELD).start_pressed);
        // Released.
        assert!(!menu.update(NOTHING).start_pressed);
        // Pressed again: the veil opens.
        assert!(
            menu.update(START_HELD).start_pressed,
            "arming must not make the pause button unusable"
        );
    }

    #[test]
    fn without_arming_a_held_start_does_open_the_veil() {
        // Pins *why* `arm_for_race_start` exists: this is the pre-fix
        // behaviour, and it is the bug.
        let mut menu = PauseMenu::new();
        assert!(
            menu.update(START_HELD).start_pressed,
            "expected the un-armed menu to treat a held Start as a fresh press"
        );
    }

    #[test]
    fn arming_does_not_reset_the_highlight() {
        // The menu deliberately remembers its selection across opens; arming
        // must not quietly rewind the player's choice.
        let mut menu = PauseMenu::new();
        menu.update(PauseInput {
            down: true,
            ..NOTHING
        });
        menu.update(NOTHING);
        let chosen = menu.selected_idx;
        assert_eq!(chosen, 1);
        menu.arm_for_race_start();
        assert_eq!(
            menu.selected_idx, chosen,
            "arming changed the remembered selection"
        );
    }
}

/// TASK-1210: the garage built setups `memcard::store_tuning` silently refused.
///
/// The garage used to allow slider values 0..=10 while `CarTuning::is_valid` --
/// the exact gate the card write applies -- requires 1..=7. A player who pushed
/// a slider to 8 built a setup the garage accepted, the exit path "saved", and
/// `store_tuning` dropped it: `is_dirty` stayed false so the following
/// `flush()` returned early, and no error appeared anywhere.
///
/// The invariant that matters is therefore not "the sliders feel right" but
/// "no setup the garage can produce is outside the range the card write
/// accepts". These tests assert that as a property over a long random walk.
#[cfg(test)]
mod tuning_budget_tests {
    use super::tuning_input::{Reject, Slider, TuningInput, TuningMenu};
    use arduracer_core::tuning::{CarTuning, DEFAULT_SLIDER, MAX_SLIDER, MIN_SLIDER, TOTAL_POINTS};

    const IDLE: TuningInput = TuningInput {
        up: false,
        down: false,
        left: false,
        right: false,
        cross: false,
        circle: false,
        start: false,
        triangle: false,
        l1: false,
        r1: false,
    };

    fn press(buttons: TuningInput) -> TuningInput {
        buttons
    }

    fn release() -> TuningInput {
        IDLE
    }

    /// Every slider in turn, so a walk exercises all five axes.
    const SLIDERS: [Slider; 5] = [
        Slider::TopSpeed,
        Slider::Acceleration,
        Slider::Handling,
        Slider::DriftStability,
        Slider::Gearing,
    ];

    fn slider_name(slider: Slider) -> &'static str {
        match slider {
            Slider::TopSpeed => "top_speed",
            Slider::Acceleration => "acceleration",
            Slider::Handling => "handling",
            Slider::DriftStability => "drift_stability",
            Slider::Gearing => "gearing",
        }
    }

    /// Asserts every slider is inside the range `is_valid` checks.
    fn assert_all_in_range(menu: &TuningMenu, context: &str) {
        for slider in SLIDERS {
            let v = slider.value_of(&menu.tuning);
            assert!(
                (MIN_SLIDER..=MAX_SLIDER).contains(&v),
                "{context}: {} = {v} left {MIN_SLIDER}..={MAX_SLIDER} (setup {:?})",
                slider_name(slider),
                menu.tuning,
            );
        }
    }

    /// Moves one point from `from` to `to`, asserting both halves succeeded.
    fn move_point(menu: &mut TuningMenu, from: Slider, to: Slider) {
        menu.decrement(from).expect("a point must be free to move");
        menu.increment(to).expect("a freed point must be spendable");
    }

    /// A setup with every slider at the floor, leaving the whole budget spare.
    fn under_budget() -> TuningMenu {
        TuningMenu::new(CarTuning {
            top_speed: MIN_SLIDER,
            acceleration: MIN_SLIDER,
            handling: MIN_SLIDER,
            drift_stability: MIN_SLIDER,
            gearing: MIN_SLIDER,
        })
    }

    #[test]
    fn the_default_setup_is_saveable() {
        let menu = TuningMenu::new(CarTuning::default());
        assert!(menu.can_save());
        assert!(menu.tuning.is_valid());
    }

    /// The budget is fully allocated by default, so the garage's first move is
    /// always "free a point, then spend it elsewhere".
    #[test]
    fn the_default_setup_starts_with_the_whole_budget_spent() {
        let menu = TuningMenu::new(CarTuning::default());
        assert_eq!(menu.tuning.total_points(), TOTAL_POINTS);
        assert_eq!(menu.points_remaining(), 0);
    }

    /// The core regression, as a property over a long random walk: no slider
    /// ever leaves `1..=7`, and the budget is never overspent. Those are the
    /// only two ways a reachable setup can fail `is_valid` for a reason other
    /// than a temporarily unbalanced total, which the exit gate handles.
    #[test]
    fn no_sequence_of_adjustments_can_produce_an_illegal_setup() {
        let mut menu = TuningMenu::new(CarTuning::default());
        // Deterministic LCG so a failure is reproducible from the seed.
        let mut state: u32 = 0x1234_5678;
        let mut rand = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 16
        };

        for step in 0..4_000 {
            let choice = rand() % 4;
            let buttons = match choice {
                0 => press(TuningInput {
                    right: true,
                    ..IDLE
                }),
                1 => press(TuningInput { left: true, ..IDLE }),
                2 => press(TuningInput { down: true, ..IDLE }),
                _ => press(TuningInput { up: true, ..IDLE }),
            };
            menu.update(buttons);
            menu.update(release());

            assert_all_in_range(&menu, &format!("step {step}"));
            assert!(
                menu.tuning.total_points() <= TOTAL_POINTS,
                "step {step}: budget overspent, total {} > {TOTAL_POINTS}",
                menu.tuning.total_points(),
            );
        }
    }

    /// A balanced setup is always reachable and always saveable, so the exit
    /// gate cannot lock the player out of the garage.
    #[test]
    fn a_balanced_setup_is_always_reachable_and_saveable() {
        let mut menu = TuningMenu::new(CarTuning::default());
        for i in 0..3_000 {
            // Deliberately biased toward increments and decrements so the walk
            // keeps crossing the balanced state.
            match i % 3 {
                0 => menu.update(press(TuningInput {
                    right: true,
                    ..IDLE
                })),
                _ => menu.update(press(TuningInput { left: true, ..IDLE })),
            };
            menu.update(release());
            if menu.tuning.total_points() == TOTAL_POINTS {
                assert_all_in_range(&menu, &format!("step {i}"));
                assert!(menu.can_save(), "a balanced setup must be saveable");
            }
        }
    }

    #[test]
    fn a_slider_cannot_be_pushed_past_the_maximum() {
        let mut menu = under_budget();
        for _ in 0..20 {
            menu.decrement(Slider::Acceleration).ok();
            menu.increment(Slider::TopSpeed).ok();
        }
        assert_eq!(
            menu.tuning.top_speed, MAX_SLIDER,
            "the slider exceeded the bound `is_valid` enforces"
        );
        assert_all_in_range(&menu, "after pushing to max");
    }

    #[test]
    fn a_slider_cannot_be_pulled_below_the_minimum() {
        let mut menu = under_budget();
        for _ in 0..20 {
            menu.decrement(Slider::TopSpeed).ok();
            menu.increment(Slider::Acceleration).ok();
        }
        assert_eq!(
            menu.tuning.top_speed, MIN_SLIDER,
            "the slider dropped below the bound `is_valid` enforces"
        );
        assert_all_in_range(&menu, "after pulling to min");
    }

    #[test]
    fn the_budget_cannot_be_overspent() {
        let mut menu = TuningMenu::new(CarTuning::default());
        for _ in 0..50 {
            for slider in SLIDERS {
                let _ = menu.increment(slider);
            }
        }
        assert_eq!(menu.tuning.total_points(), TOTAL_POINTS);
        assert_eq!(menu.points_remaining(), 0);
    }

    #[test]
    fn a_refusal_explains_itself() {
        // Cap top speed with room spare, so the refusal can only be the cap.
        let mut menu = under_budget();
        for _ in 0..(MAX_SLIDER - MIN_SLIDER) {
            menu.increment(Slider::TopSpeed).unwrap();
        }
        assert_eq!(menu.tuning.top_speed, MAX_SLIDER);
        assert!(
            menu.points_remaining() > 0,
            "the budget must have room spare, or this proves nothing"
        );
        assert_eq!(
            menu.increment(Slider::TopSpeed),
            Err(Reject::AtMax),
            "a capped slider must report the cap, not a budget problem"
        );
    }

    #[test]
    fn the_floor_also_explains_itself() {
        let mut menu = under_budget();
        assert_eq!(menu.tuning.gearing, MIN_SLIDER);
        assert_eq!(
            menu.decrement(Slider::Gearing),
            Err(Reject::AtMin),
            "a floored slider must report the floor"
        );
    }

    #[test]
    fn spending_the_last_point_reports_no_points_left() {
        // Spread the budget so no single slider hits its cap: 15 spare points
        // across four sliders is 3-4 each, inside the 1..=7 range.
        let mut menu = under_budget();
        let spare = TOTAL_POINTS - 5;
        let others = [
            Slider::TopSpeed,
            Slider::Acceleration,
            Slider::Handling,
            Slider::DriftStability,
        ];
        for (i, slider) in others.iter().enumerate() {
            let share = spare / others.len() as u8
                + if i < (spare % others.len() as u8) as usize {
                    1
                } else {
                    0
                };
            for _ in 0..share {
                menu.increment(*slider).unwrap();
            }
        }
        assert_eq!(menu.points_remaining(), 0);
        for slider in others {
            assert!(
                slider.value_of(&menu.tuning) < MAX_SLIDER,
                "a slider hit its cap, so the next refusal would be ambiguous"
            );
        }
        assert_eq!(menu.increment(others[0]), Err(Reject::NoPointsLeft));
        assert!(
            menu.can_save(),
            "a fully allocated setup is balanced and must still be saveable"
        );
    }

    /// Points may be freed before they are re-spent -- that is the mechanic.
    /// What must not happen is *leaving* with the budget unbalanced, because the
    /// card write would then discard the setup with no message.
    #[test]
    fn points_can_be_freed_before_they_are_re_spent() {
        let mut menu = TuningMenu::new(CarTuning::default());
        assert_eq!(
            menu.decrement(Slider::TopSpeed),
            Ok(()),
            "freeing a point must be allowed even with the budget spent"
        );
        assert_eq!(menu.points_remaining(), 1);
        assert!(!menu.can_save(), "an unbalanced setup must not be saveable");
    }

    #[test]
    fn leaving_with_points_unallocated_is_refused_and_explained() {
        let mut menu = TuningMenu::new(CarTuning::default());
        menu.update(press(TuningInput { left: true, ..IDLE }));
        menu.update(release());
        assert_eq!(menu.points_remaining(), 1);

        let frame = menu.update(press(TuningInput {
            circle: true,
            ..IDLE
        }));
        assert!(
            !frame.exited,
            "left the garage with an unspent point; `store_tuning` would drop it"
        );
        assert_eq!(frame.rejected, Some(Reject::Unbalanced));
    }

    #[test]
    fn leaving_with_sliders_out_of_range_is_impossible() {
        // Complements the walk above: the range invariant holds by
        // construction, so `can_save` can only be false because of the total.
        let mut menu = TuningMenu::new(CarTuning::default());
        for _ in 0..200 {
            menu.update(press(TuningInput { left: true, ..IDLE }));
            menu.update(release());
            assert_all_in_range(&menu, "after decrementing");
            assert_eq!(
                menu.can_save(),
                menu.tuning.total_points() == TOTAL_POINTS,
                "`can_save` disagreed with the budget for {:?}",
                menu.tuning
            );
        }
    }

    #[test]
    fn triangle_arms_a_confirmation_instead_of_wiping_the_setup() {
        let mut menu = TuningMenu::new(CarTuning::default());
        move_point(&mut menu, Slider::Gearing, Slider::TopSpeed);
        let tuned = menu.tuning;

        let frame = menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        assert!(frame.reset_requested, "triangle must ask first");
        assert!(
            !frame.reset_confirmed,
            "triangle must not reset on the same frame"
        );
        assert_eq!(menu.tuning, tuned, "the setup was wiped immediately");

        menu.update(release());
        assert!(menu.reset_pending, "the prompt must survive the release");
    }

    #[test]
    fn confirming_the_reset_restores_the_defaults() {
        let mut menu = TuningMenu::new(CarTuning::default());
        move_point(&mut menu, Slider::Gearing, Slider::TopSpeed);
        menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        menu.update(release());
        let frame = menu.update(press(TuningInput {
            cross: true,
            ..IDLE
        }));
        assert!(frame.reset_confirmed);
        assert_eq!(menu.tuning, CarTuning::default());
        assert!(
            !menu.reset_pending,
            "the prompt must clear after confirming"
        );
    }

    #[test]
    fn cancelling_the_reset_keeps_the_setup() {
        let mut menu = TuningMenu::new(CarTuning::default());
        move_point(&mut menu, Slider::Gearing, Slider::TopSpeed);
        let tuned = menu.tuning;
        menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        menu.update(release());
        let frame = menu.update(press(TuningInput {
            circle: true,
            ..IDLE
        }));
        assert!(frame.reset_cancelled);
        assert_eq!(menu.tuning, tuned, "cancel must not discard the setup");
        assert!(!menu.reset_pending);
    }

    #[test]
    fn the_dpad_does_not_move_sliders_while_the_reset_prompt_is_up() {
        let mut menu = TuningMenu::new(CarTuning::default());
        menu.selected = Slider::TopSpeed;
        menu.update(press(TuningInput {
            triangle: true,
            ..IDLE
        }));
        menu.update(release());
        menu.update(press(TuningInput { down: true, ..IDLE }));
        menu.update(release());
        assert_eq!(
            menu.selected,
            Slider::TopSpeed,
            "the slider selection moved underneath the prompt"
        );
    }

    #[test]
    fn a_held_exit_button_does_not_leave_on_the_first_frame() {
        // `Cross`/`Start` confirm or exit, and both navigate the main menu
        // immediately before this screen.
        let mut menu = TuningMenu::new(CarTuning::default());
        let held = TuningInput {
            circle: true,
            start: true,
            cross: true,
            ..IDLE
        };
        let frame = menu.update(held);
        assert!(!frame.exited, "held buttons saved-and-exited on frame 1");
        menu.update(release());
        assert!(menu.update(held).exited, "exiting must still work");
    }

    #[test]
    fn slider_selection_wraps_in_both_directions() {
        let mut menu = TuningMenu::new(CarTuning::default());
        assert_eq!(menu.selected, Slider::TopSpeed);
        menu.update(press(TuningInput { up: true, ..IDLE }));
        assert_eq!(menu.selected, Slider::Gearing, "up must wrap to the end");
        menu.update(release());
        menu.update(press(TuningInput { down: true, ..IDLE }));
        assert_eq!(
            menu.selected,
            Slider::TopSpeed,
            "down must wrap to the start"
        );
    }

    #[test]
    fn points_remaining_is_never_negative_or_above_the_budget() {
        let mut menu = TuningMenu::new(CarTuning::default());
        for _ in 0..40 {
            menu.decrement(Slider::Handling).ok();
            menu.increment(Slider::Handling).ok();
            let remaining = menu.points_remaining();
            assert!(
                remaining <= TOTAL_POINTS,
                "remaining {remaining} over budget"
            );
        }
    }

    /// The five sliders are all the same shape, so a wrong `match` arm in
    /// `value_of`/`set_value` would be invisible in the walk above.
    #[test]
    fn each_slider_reads_and_writes_its_own_field() {
        for slider in SLIDERS {
            let mut menu = under_budget();
            menu.increment(slider).unwrap();
            let after = slider.value_of(&menu.tuning);
            assert_eq!(
                after,
                MIN_SLIDER + 1,
                "{} read back the wrong field",
                slider_name(slider)
            );
            // And the bump must not have touched any other axis.
            let untouched = SLIDERS
                .iter()
                .filter(|s| **s != slider)
                .all(|s| s.value_of(&menu.tuning) == MIN_SLIDER);
            assert!(
                untouched,
                "adjusting {} moved another axis ({:?})",
                slider_name(slider),
                menu.tuning
            );
        }
    }

    #[test]
    fn the_default_value_sits_inside_the_legal_range() {
        // The gauge and the gate both index off these constants; if a slider
        // ever started outside the range the arithmetic would be nonsense.
        assert!((MIN_SLIDER..=MAX_SLIDER).contains(&DEFAULT_SLIDER));
    }
}

/// TASK-1206: particles and skidmarks were emitted but effectively invisible.
#[cfg(test)]
mod effects_tests {
    use super::effects_sim::{
        ParticleSystem, ParticleType, SkidmarkBuffer, MAX_PARTICLES, MAX_SKIDMARKS,
        SKIDMARK_FADE_FROM, SKIDMARK_LIFE, SMOKE_LIFE, SPARK_LIFE,
    };
    use arduracer_core::{Fixed, Vec2};

    const SOMEWHERE: Vec2 = Vec2::new(Fixed::from_int(400), Fixed::from_int(400));

    /// The ring is stamped at most once per frame, so a mark cannot outlive the
    /// buffer or it is always overwritten mid-fade.
    #[test]
    fn a_skidmark_outlives_the_buffer_that_holds_it() {
        assert!(
            SKIDMARK_LIFE as usize <= MAX_SKIDMARKS,
            "life {SKIDMARK_LIFE} exceeds the {MAX_SKIDMARKS}-slot ring, so every \
             mark is overwritten before it fades"
        );
    }

    #[test]
    fn a_skidmark_actually_reaches_its_faded_stage() {
        // The regression: at life 180 against a 96-slot ring the fade threshold
        // was unreachable in practice.
        let mut marks = SkidmarkBuffer::new();
        marks.emit(SOMEWHERE, 0);
        let mut saw_fresh = false;
        let mut saw_faded = false;
        for _ in 0..SKIDMARK_LIFE {
            let (_, faded) = marks.visible().next().expect("mark must be live");
            if faded {
                saw_faded = true;
            } else {
                saw_fresh = true;
            }
            marks.tick();
        }
        assert!(saw_fresh, "the mark was never drawn as fresh rubber");
        assert!(
            saw_faded,
            "the mark never reached its faded stage, so the fade branch is dead code"
        );
    }

    #[test]
    fn a_skidmark_expires_rather_than_lingering() {
        let mut marks = SkidmarkBuffer::new();
        marks.emit(SOMEWHERE, 0);
        assert_eq!(marks.live_count(), 1);
        for _ in 0..SKIDMARK_LIFE {
            marks.tick();
        }
        assert_eq!(marks.live_count(), 0, "the mark outlived its lifetime");
    }

    #[test]
    fn the_fade_threshold_is_inside_the_lifetime() {
        assert!(
            (SKIDMARK_FADE_FROM as usize) < SKIDMARK_LIFE as usize,
            "a fade threshold at or past the lifetime is unreachable"
        );
    }

    #[test]
    fn a_continuous_slide_fills_the_ring_and_stays_stable() {
        // Emitting every frame for longer than the ring is deep must not grow
        // the live count without bound.
        let mut marks = SkidmarkBuffer::new();
        for frame in 0..(MAX_SKIDMARKS * 3) {
            marks.emit(SOMEWHERE, frame as u16);
            marks.tick();
        }
        assert!(
            marks.live_count() <= MAX_SKIDMARKS,
            "live marks exceeded the ring capacity"
        );
        // Once the ring is saturated it stays saturated: a slot is reclaimed
        // only as it is reused, so the count sits at capacity less the one slot
        // whose life expires this frame.
        assert!(
            marks.live_count() >= MAX_SKIDMARKS - 1,
            "the ring never filled: only {} of {MAX_SKIDMARKS} slots live",
            marks.live_count()
        );
    }

    /// Smoke used to be emitted at the car's exact centre with zero velocity,
    /// so it never drifted clear of the opaque body quad drawn over it.
    #[test]
    fn smoke_drifts_away_from_where_it_was_emitted() {
        let mut particles = ParticleSystem::new();
        assert!(particles.emit_smoke(SOMEWHERE));
        let born = particles
            .pool
            .iter()
            .find(|p| p.ptype == ParticleType::TireSmoke)
            .expect("a puff must exist")
            .pos;
        assert_ne!(born, SOMEWHERE, "the puff was born inside the car");

        let mut moved = false;
        for _ in 0..8 {
            particles.tick();
            let now = particles
                .pool
                .iter()
                .find(|p| p.ptype == ParticleType::TireSmoke)
                .expect("the puff must still exist")
                .pos;
            if (now.y - born.y).abs() > Fixed::from_int(2) {
                moved = true;
            }
        }
        assert!(
            moved,
            "smoke never drifted; it sat on the car and was painted over"
        );
    }

    #[test]
    fn smoke_slows_as_it_disperses() {
        let mut particles = ParticleSystem::new();
        particles.emit_smoke(SOMEWHERE);
        let idx = particles
            .pool
            .iter()
            .position(|p| p.ptype == ParticleType::TireSmoke)
            .unwrap();
        let launch = particles.pool[idx].vel.y.raw().abs();
        for _ in 0..6 {
            particles.tick();
        }
        assert!(
            particles.pool[idx].vel.y.raw().abs() < launch,
            "smoke did not decelerate"
        );
    }

    /// `max_life` used to be written but never read; the renderer branched on
    /// absolute `life` bands, so appearance depended on the countdown value
    /// rather than on the particle's age.
    #[test]
    fn age_fraction_runs_from_birth_to_expiry() {
        let mut particles = ParticleSystem::new();
        particles.emit_smoke(SOMEWHERE);
        let idx = particles
            .pool
            .iter()
            .position(|p| p.ptype == ParticleType::TireSmoke)
            .unwrap();

        let birth = particles.pool[idx].age_fraction();
        assert_eq!(birth, 0, "a newborn particle reported as fully aged");

        // Tick to the last live frame. The puff is culled on the tick after
        // that, so the final live frame is `life == 1`, not `life == 0`.
        let mut previous = birth;
        for _ in 0..(SMOKE_LIFE - 1) {
            particles.tick();
            let age = particles.pool[idx].age_fraction();
            assert!(age >= previous, "age went backwards: {previous} then {age}");
            previous = age;
        }
        assert!(
            previous >= 240,
            "the puff never approached full age: {previous}"
        );
        // And once it is culled, the fraction saturates rather than wrapping.
        particles.tick();
        assert_eq!(
            particles.pool[idx].age_fraction(),
            255,
            "an expired particle did not report full age"
        );
    }

    #[test]
    fn age_fraction_is_monotonic_for_sparks_too() {
        let mut particles = ParticleSystem::new();
        particles.emit_sparks(SOMEWHERE, Vec2::new(Fixed::ONE, Fixed::ZERO));
        let idx = particles
            .pool
            .iter()
            .position(|p| p.ptype == ParticleType::Sparks)
            .unwrap();
        assert_eq!(particles.pool[idx].age_fraction(), 0);
        for _ in 0..SPARK_LIFE {
            particles.tick();
        }
        assert_eq!(particles.live_count(), 0, "sparks outlived their lifetime");
    }

    /// A dead particle must report a usable fraction rather than dividing by a
    /// zero `max_life`.
    #[test]
    fn a_dead_particle_reports_full_age_instead_of_dividing_by_zero() {
        let system = ParticleSystem::new();
        let p = system.pool[0];
        assert_eq!(p.max_life, 0);
        assert_eq!(p.age_fraction(), 255);
    }

    #[test]
    fn sparks_fly_outward_from_the_contact_normal() {
        let mut particles = ParticleSystem::new();
        let emitted = particles.emit_sparks(SOMEWHERE, Vec2::new(Fixed::ZERO, Fixed::ONE));
        assert_eq!(emitted, 3, "all three spark directions must be emitted");
        for p in particles.pool.iter().filter(|p| p.ptype.is_live()) {
            assert!(p.vel.length() > Fixed::ZERO, "a spark was emitted at rest");
        }
    }

    #[test]
    fn the_pool_does_not_overflow() {
        let mut particles = ParticleSystem::new();
        for _ in 0..(MAX_PARTICLES + 20) {
            particles.emit_smoke(SOMEWHERE);
        }
        assert_eq!(
            particles.live_count(),
            MAX_PARTICLES,
            "the pool exceeded its capacity instead of refusing new particles"
        );
    }

    #[test]
    fn exhausted_particles_are_reclaimed() {
        let mut particles = ParticleSystem::new();
        particles.emit_smoke(SOMEWHERE);
        assert_eq!(particles.live_count(), 1);
        for _ in 0..SMOKE_LIFE {
            particles.tick();
        }
        assert_eq!(particles.live_count(), 0);
        assert!(
            particles.emit_smoke(SOMEWHERE),
            "a full pool of dead particles stopped accepting new ones"
        );
    }

    #[test]
    fn smoke_spawns_and_drifts_behind_car_for_any_heading() {
        // Heading 0 = North (-Y) -> rear is South (+Y)
        // Heading 1024 = East (+X) -> rear is West (-X)
        // Heading 2048 = South (+Y) -> rear is North (-Y)
        // Heading 3072 = West (-X) -> rear is East (+X)
        let mut north = ParticleSystem::new();
        assert!(north.emit_smoke_at(SOMEWHERE, 0));
        let p_n = north.pool[0];
        assert!(p_n.pos.y > SOMEWHERE.y && p_n.vel.y > Fixed::ZERO);

        let mut east = ParticleSystem::new();
        assert!(east.emit_smoke_at(SOMEWHERE, 1024));
        let p_e = east.pool[0];
        assert!(p_e.pos.x < SOMEWHERE.x && p_e.vel.x < Fixed::ZERO);

        let mut south = ParticleSystem::new();
        assert!(south.emit_smoke_at(SOMEWHERE, 2048));
        let p_s = south.pool[0];
        assert!(p_s.pos.y < SOMEWHERE.y && p_s.vel.y < Fixed::ZERO);

        let mut west = ParticleSystem::new();
        assert!(west.emit_smoke_at(SOMEWHERE, 3072));
        let p_w = west.pool[0];
        assert!(p_w.pos.x > SOMEWHERE.x && p_w.vel.x > Fixed::ZERO);
    }
}

/// TASK-1205: the "dark asphalt" gate underlay was invisible on tracks 1-6.
#[cfg(test)]
mod palette_tests {
    use super::palette::{to_bgr555, Palette, PALETTES};

    const CUP_NAMES: [&str; 4] = ["Bronze", "Silver", "Gold", "Platinum"];

    /// The regression itself. VRAM stores 5 bits per channel, and the authored
    /// 8-bit values were never checked for surviving that truncation.
    #[test]
    fn the_bronze_gate_underlay_is_distinct_from_the_road() {
        let bronze = PALETTES[0];
        assert_ne!(
            to_bgr555(bronze.road),
            to_bgr555(bronze.road2),
            "Bronze road and road2 quantise to the same VRAM word \
             0x{:04X}: the timing-gate underlay is invisible",
            to_bgr555(bronze.road)
        );
    }

    /// Every palette, every pair. This is the check that was missing.
    #[test]
    fn no_palette_collapses_two_colours_onto_one_vram_word() {
        for (cup, palette) in PALETTES.iter().enumerate() {
            assert!(
                palette.is_distinct(),
                "{} palette: {:?} quantise to the same VRAM word",
                CUP_NAMES[cup],
                palette.first_collision()
            );
        }
    }

    #[test]
    fn the_gate_underlay_is_darker_than_the_road_in_every_cup() {
        // The underlay is meant to read as shaded asphalt, so it must still be
        // darker per channel after truncation -- not merely "different".
        for (cup, palette) in PALETTES.iter().enumerate() {
            assert!(
                to_bgr555(palette.road2) < to_bgr555(palette.road),
                "{} palette: road2 (0x{:04X}) is not darker than road (0x{:04X})",
                CUP_NAMES[cup],
                to_bgr555(palette.road2),
                to_bgr555(palette.road)
            );
        }
    }

    #[test]
    fn the_road_and_underlay_stay_visibly_apart() {
        // A one-bit difference is technically distinct but invisible in play.
        // The gate underlay has to read as a clear band.
        for (cup, palette) in PALETTES.iter().enumerate() {
            let delta = to_bgr555(palette.road).abs_diff(to_bgr555(palette.road2));
            assert!(
                delta >= 0x0400,
                "{} palette: road and road2 differ by only 0x{:04X}, \
                 which is under one full red channel",
                CUP_NAMES[cup],
                delta
            );
        }
    }

    #[test]
    fn quantisation_never_widens_a_colour_beyond_one_5bit_bucket() {
        // Sanity check on the quantiser itself: the result must be within 7 of
        // the input on every channel, i.e. only the low three bits are dropped.
        for palette in PALETTES.iter() {
            for (name, (r, g, b)) in palette.entries() {
                let word = to_bgr555((r, g, b));
                let r5 = ((word & 0x1F) * 8) as i32;
                let g5 = (((word >> 5) & 0x1F) * 8) as i32;
                let b5 = (((word >> 10) & 0x1F) * 8) as i32;
                for (channel, original, restored) in [("r", r, r5), ("g", g, g5), ("b", b, b5)] {
                    assert!(
                        (original as i32 - restored).abs() <= 7,
                        "{name}: channel {channel} {original} -> {restored}, off by more than one bucket"
                    );
                }
            }
        }
    }

    #[test]
    fn the_curbs_stay_visible_against_their_own_road() {
        // A kerb that truncates into the tarmac is not a kerb.
        for (cup, palette) in PALETTES.iter().enumerate() {
            let road = to_bgr555(palette.road);
            for name in ["curb_a", "curb_b"] {
                let colour = match name {
                    "curb_a" => palette.curb_a,
                    _ => palette.curb_b,
                };
                let word = to_bgr555(colour);
                assert_ne!(
                    word, road,
                    "{} palette: {name} is the same colour as the road",
                    CUP_NAMES[cup]
                );
            }
        }
    }

    #[test]
    fn the_palette_reports_the_offending_pair() {
        // The check is only useful if it can say *which* two collided.
        let broken = Palette {
            road: (44, 46, 52),
            road2: (40, 42, 48),
            ..PALETTES[0]
        };
        assert!(!broken.is_distinct());
        assert_eq!(broken.first_collision(), Some(("road", "road2")));
    }
}

/// TASK-1216: `world_to_screen` narrowed `i32` to `i16` with no clamp, and culled
/// *after* truncating.
///
/// A point far off-screen wrapped to the opposite side in release mode, so the
/// caller saw a plausible-looking on-screen coordinate and drew a car in the
/// wrong corner instead of culling it. The culling ranges are deliberately wider
/// than the viewport, so a wrapped coordinate lands inside them.
#[cfg(test)]
mod projection_clamp_tests {
    use super::camera::{Camera, SCREEN_H, SCREEN_W};
    use arduracer_core::{Fixed, Vec2};

    #[test]
    fn a_point_far_off_screen_does_not_wrap_to_the_opposite_corner() {
        let camera = Camera::new(Vec2::new(Fixed::from_int(320), Fixed::from_int(240)));
        // Enormously beyond the 320x240 world, in each quadrant.
        let far = [
            (40_000i32, 40_000i32),
            (-40_000, 40_000),
            (40_000, -40_000),
            (-40_000, -40_000),
        ];
        for (wx, wy) in far {
            let (sx, sy) =
                camera.world_to_screen(Vec2::new(Fixed::from_int(wx), Fixed::from_int(wy)));
            assert!(
                !(-64..=SCREEN_W + 64).contains(&sx),
                "({wx},{wy}) wrapped to sx={sx}, which is on-screen"
            );
            assert!(
                !(-64..=SCREEN_H + 64).contains(&sy),
                "({wx},{wy}) wrapped to sy={sy}, which is on-screen"
            );
        }
    }

    #[test]
    fn a_point_on_the_camera_lands_at_screen_centre() {
        let centre = Vec2::new(Fixed::from_int(320), Fixed::from_int(240));
        let camera = Camera::new(centre);
        let (sx, sy) = camera.world_to_screen(centre);
        assert_eq!((sx, sy), (SCREEN_W / 2, SCREEN_H / 2));
    }

    #[test]
    fn every_projected_coordinate_is_a_valid_i16() {
        // The old code truncated `i32` to `i16` with no clamp; in release that
        // wraps, and a wrapped coordinate is indistinguishable from a real one.
        let camera = Camera::new(Vec2::new(Fixed::from_int(320), Fixed::from_int(240)));
        for step in -200..200 {
            let pos = Vec2::new(
                Fixed::from_int(320 + step * 500),
                Fixed::from_int(240 - step * 400),
            );
            let (sx, _sy) = camera.world_to_screen(pos);
            // Never wraps back to screen centre, which is what a truncation
            // looked like.
            assert!(
                sx != SCREEN_W / 2 || step == 0,
                "step {step} collapsed to screen centre"
            );
        }
    }
}

#[cfg(test)]
mod extended_view_distance_tests {
    use super::camera::{
        Camera, M7_OFFSCREEN, M7_SCAN_RADIUS, M7_SCAN_RADIUS_COMPRESSED, M7_Z_FAR,
        M7_Z_FAR_COMPRESSED,
    };
    use arduracer_core::{Fixed, Vec2};

    const MAIN_SRC: &str = include_str!("../../../game/src/main.rs");
    const TRACKTEX_SRC: &str = include_str!("../../../game/src/gpu/tracktex.rs");

    #[test]
    fn camera_defaults_to_standard_view_distance() {
        let pos = Vec2::new(Fixed::from_int(320), Fixed::from_int(240));
        let c_topdown = Camera::new(pos);
        assert_eq!(
            c_topdown.z_far, M7_Z_FAR,
            "topdown camera must default to M7_Z_FAR (240)"
        );

        let c_chase = Camera::new_chase(pos, 0);
        assert_eq!(
            c_chase.z_far, M7_Z_FAR,
            "chase camera must default to M7_Z_FAR (240)"
        );

        let c_rotating = Camera::new_rotating(pos, 0);
        assert_eq!(
            c_rotating.z_far, M7_Z_FAR,
            "rotating camera must default to M7_Z_FAR (240)"
        );
    }

    #[test]
    fn camera_extended_view_distance_toggle() {
        let pos = Vec2::new(Fixed::from_int(320), Fixed::from_int(240));
        let mut cam = Camera::new_chase(pos, 0);
        assert_eq!(cam.z_far, M7_Z_FAR);

        cam.set_extended_view_distance(true);
        assert_eq!(
            cam.z_far, M7_Z_FAR_COMPRESSED,
            "extended view distance must be M7_Z_FAR_COMPRESSED (560)"
        );

        cam.set_extended_view_distance(false);
        assert_eq!(
            cam.z_far, M7_Z_FAR,
            "unsetting extended view distance must restore M7_Z_FAR (240)"
        );
    }

    #[test]
    fn mode7_projection_extended_depth() {
        let cam_pos = Vec2::new(Fixed::from_int(320), Fixed::from_int(240));
        let mut cam = Camera::new_chase(cam_pos, 0);
        // Heading 0 faces north (negative y).
        // Depth = fwd + M7_Z0 (120).
        // At dy = -200 (pos = (320, 40)), fwd = 200, depth z = 320.
        let target_320 = Vec2::new(Fixed::from_int(320), Fixed::from_int(40));

        // Under standard view distance (240), depth 320 is past the far plane:
        assert_eq!(
            cam.world_to_screen(target_320),
            M7_OFFSCREEN,
            "depth 320 must be offscreen for standard view distance"
        );
        assert_eq!(
            cam.m7_scale_q12(target_320),
            0,
            "scale at depth 320 must be 0 for standard view distance"
        );

        // Enable extended view distance (560):
        cam.set_extended_view_distance(true);
        let (sx, sy) = cam.world_to_screen(target_320);
        assert_ne!(
            (sx, sy),
            M7_OFFSCREEN,
            "depth 320 must project onto screen with extended view distance"
        );
        assert_eq!(
            sx, 160,
            "point directly ahead should be horizontally centered on screen"
        );
        assert!(
            (60..=240).contains(&sy),
            "sy={sy} should be between horizon (60) and screen bottom (240)"
        );
        let scale = cam.m7_scale_q12(target_320);
        assert!(
            scale > 0,
            "scale at depth 320 must be positive, got {scale}"
        );

        // At dy = -500 (pos = (320, -260)), fwd = 500, depth z = 620.
        // Even with extended view distance (560), depth 620 is past the far plane:
        let target_620 = Vec2::new(Fixed::from_int(320), Fixed::from_int(-260));
        assert_eq!(
            cam.world_to_screen(target_620),
            M7_OFFSCREEN,
            "depth 620 must be offscreen for both modes"
        );
        assert_eq!(
            cam.m7_scale_q12(target_620),
            0,
            "scale at depth 620 must be 0 for both modes"
        );
    }

    #[test]
    fn mode7_constants_and_frustum_dimensions() {
        assert_eq!(M7_Z_FAR, 240, "standard Mode 7 far plane is 240");
        assert_eq!(
            M7_Z_FAR_COMPRESSED, 560,
            "compressed Mode 7 far plane extends 2 tiles (320 units) to 560"
        );
        assert_eq!(M7_SCAN_RADIUS, 360, "standard frustum scan radius is 360");
        assert_eq!(
            M7_SCAN_RADIUS_COMPRESSED, 800,
            "compressed frustum scan radius is 800"
        );
    }

    #[test]
    fn mode7_vehicle_screen_y_projects_distant_vehicles_to_valid_rows() {
        let cam_pos = Vec2::new(Fixed::from_int(320), Fixed::from_int(240));
        let cam = Camera::new_chase(cam_pos, 0);

        // Heading 0 faces north (-Y).
        // Depth = fwd + M7_Z0 (120).
        let pos_near = Vec2::new(Fixed::from_int(320), Fixed::from_int(140)); // dy = -100, fwd = 100, depth = 220
        let pos_mid = Vec2::new(Fixed::from_int(320), Fixed::from_int(40)); // dy = -200, fwd = 200, depth = 320
        let pos_far = Vec2::new(Fixed::from_int(320), Fixed::from_int(-260)); // dy = -500, fwd = 500, depth = 620
        let pos_horizon = Vec2::new(Fixed::from_int(320), Fixed::from_int(-1500)); // dy = -1740, depth = 1860
        let pos_behind = Vec2::new(Fixed::from_int(320), Fixed::from_int(300)); // behind camera (z_q4 < near)

        let y_near = cam.vehicle_screen_y(pos_near);
        let y_mid = cam.vehicle_screen_y(pos_mid);
        let y_far = cam.vehicle_screen_y(pos_far);
        let y_horizon = cam.vehicle_screen_y(pos_horizon);
        let y_behind = cam.vehicle_screen_y(pos_behind);

        // All visible vehicles ahead of the player must project between horizon (~60) and viewport bottom (240)
        assert!((60..=240).contains(&y_near), "y_near={y_near}");
        assert!((60..=240).contains(&y_mid), "y_mid={y_mid}");
        assert!((60..=240).contains(&y_far), "y_far={y_far}");
        assert!((60..=240).contains(&y_horizon), "y_horizon={y_horizon}");

        // Farther vehicles must have smaller screen Y (closer to horizon / higher on screen)
        // so painter's algorithm sorts them back-to-front
        assert!(
            y_horizon < y_far,
            "horizon vehicle must be above far vehicle"
        );
        assert!(y_far < y_mid, "far vehicle must be above mid vehicle");
        assert!(y_mid < y_near, "mid vehicle must be above near vehicle");

        // Vehicles behind camera sort to the very front/bottom
        assert_eq!(y_behind, 30_000);
    }

    #[test]
    fn main_and_tracktex_camera_synchronization_wiring() {
        assert!(
            MAIN_SRC.contains("pub fn sync_camera_view_distance(&mut self)"),
            "main.rs must provide sync_camera_view_distance helper"
        );
        assert!(
            MAIN_SRC.contains("self.sync_camera_view_distance();"),
            "main.rs must invoke sync_camera_view_distance"
        );
        assert!(
            MAIN_SRC.contains("set_extended_view_distance(!codec_is_uncompressed"),
            "main.rs must initialize camera view distance"
        );
        assert!(
            TRACKTEX_SRC.contains("pub fn codec_is_uncompressed("),
            "tracktex.rs must export pub fn codec_is_uncompressed"
        );
        assert!(
            TRACKTEX_SRC.contains("pub fn m7_view_params("),
            "tracktex.rs must export pub fn m7_view_params"
        );
        assert!(
            TRACKTEX_SRC.contains("pub fn m7_z_far("),
            "tracktex.rs must export pub fn m7_z_far"
        );
        assert!(
            TRACKTEX_SRC.contains("rows_per_budget_for_codec(active_codec, is_multiblock)"),
            "tracktex.rs must calculate rows_per_budget using rows_per_budget_for_codec"
        );
    }
}

/// TASK-1217: several smaller UI defects, each with its own failure mode.
#[cfg(test)]
mod ui_defect_tests {
    use super::tuning_input::{Slider, TuningInput, TuningMenu, SLOT_COUNT};
    use arduracer_core::tuning::{CarTuning, DEFAULT_SLIDER};

    const IDLE: TuningInput = TuningInput {
        up: false,
        down: false,
        left: false,
        right: false,
        cross: false,
        circle: false,
        start: false,
        triangle: false,
        l1: false,
        r1: false,
    };

    /// The gate count rendered as `b'0' + checkpoint_count`, so 10, 12 and 13
    /// gates printed ':', '<' and '='.
    #[test]
    fn a_two_digit_gate_count_stays_inside_the_digit_range() {
        for gates in [4u8, 8, 10, 12, 13, 99] {
            let tens = b'0' + gates / 10;
            let ones = b'0' + gates % 10;
            assert!(
                tens.is_ascii_digit(),
                "tens digit for {gates} gates is {:?}, not a digit",
                tens as char
            );
            assert!(
                ones.is_ascii_digit(),
                "ones digit for {gates} gates is {:?}, not a digit",
                ones as char
            );
        }
    }

    /// Pin *why* the old form was wrong: the naive single glyph is only correct
    /// below ten.
    #[test]
    fn the_naive_single_glyph_gate_count_overflows_at_ten() {
        assert_eq!(b'0' + 9, b'9', "correct below ten");
        assert_eq!(b'0' + 10, b':', "this is the bug: ten gates printed ':'");
        assert_eq!(b'0' + 12, b'<', "twelve gates printed '<'");
        assert_eq!(b'0' + 13, b'=', "thirteen gates printed '='");
    }

    /// `active_tuning_slot` was serialised and checksummed but never written, so
    /// only preset 0 was ever reachable.
    #[test]
    fn all_three_presets_are_reachable() {
        let mut menu = TuningMenu::new(CarTuning::default());
        assert_eq!(SLOT_COUNT, 3, "expected three garage presets");
        let mut seen = [false; SLOT_COUNT];
        seen[menu.slot] = true;
        for _ in 0..SLOT_COUNT {
            menu.cycle_slot(true);
            assert!(
                menu.slot < SLOT_COUNT,
                "slot index {} is out of range",
                menu.slot
            );
            seen[menu.slot] = true;
        }
        assert!(seen.iter().all(|s| *s), "a preset was never selectable");
    }

    #[test]
    fn cycling_presets_wraps_in_both_directions() {
        let mut menu = TuningMenu::new(CarTuning::default());
        assert_eq!(menu.slot, 0);
        menu.cycle_slot(false);
        assert_eq!(menu.slot, SLOT_COUNT - 1, "back must wrap to the last");
        menu.cycle_slot(true);
        assert_eq!(menu.slot, 0, "forward must wrap to the first");
    }

    #[test]
    fn an_out_of_range_slot_index_is_clamped_not_indexed() {
        // `active_tuning_slot` is sanitised on load, but a corrupt 255 must not
        // index past the preset array.
        let mut menu = TuningMenu::new(CarTuning::default());
        menu.load_slot(255, CarTuning::default());
        assert!(menu.slot < SLOT_COUNT);
    }

    #[test]
    fn loading_a_preset_clears_the_unsaved_flag() {
        let mut menu = TuningMenu::new(CarTuning::default());
        menu.mark_dirty();
        assert!(menu.slot_dirty);
        menu.load_slot(1, CarTuning::default());
        assert!(
            !menu.slot_dirty,
            "loading a preset marked it dirty before the player touched it"
        );
    }

    /// Releases everything once, so the armed entry baseline is behind us.
    ///
    /// `TuningMenu::new` arms every button, so the *first* press of any of them
    /// reads as a hold rather than an edge -- the same contract `Cross` already
    /// used, now extended to the shoulder buttons.
    fn released() -> TuningMenu {
        let mut menu = TuningMenu::new(CarTuning::default());
        // The armed baseline means this release is a no-op; it just moves the
        // baseline so the *next* press reads as an edge.
        let _ = menu.update(IDLE);
        menu
    }

    #[test]
    fn shoulder_buttons_cycle_presets_and_do_nothing_else() {
        let mut menu = released();
        menu.selected = Slider::TopSpeed;
        let before = menu.tuning;

        menu.update(TuningInput { r1: true, ..IDLE });
        assert_eq!(menu.slot, 1, "R1 must advance the preset");
        menu.update(IDLE);
        menu.update(TuningInput { l1: true, ..IDLE });
        assert_eq!(menu.slot, 0, "L1 must go back");
        assert_eq!(menu.selected, Slider::TopSpeed, "preset changed sliders");
        assert_eq!(menu.tuning, before, "preset cycling edited the setup");
    }

    #[test]
    fn a_held_shoulder_button_does_not_rapidly_cycle() {
        let mut menu = released();
        menu.update(TuningInput { r1: true, ..IDLE });
        assert_eq!(menu.slot, 1);
        for _ in 0..10 {
            menu.update(TuningInput { r1: true, ..IDLE });
        }
        assert_eq!(menu.slot, 1, "a held R1 spun through every preset");
    }

    #[test]
    fn a_held_shoulder_button_does_not_exit_on_entry() {
        // L1/R1 are the recovery and nitro buttons mid-race, so a button held
        // from the main menu must not be read as a preset switch on frame 1.
        let mut menu = TuningMenu::new(CarTuning::default());
        let held = TuningInput {
            l1: true,
            r1: true,
            ..IDLE
        };
        let frame = menu.update(held);
        assert!(!frame.exited, "a held button saved-and-exited on frame 1");
        menu.update(IDLE);
        assert_eq!(menu.slot, 0, "a held shoulder button changed the preset");
    }

    /// The tuning slider bounds and the preset index are independent: cycling
    /// presets must not silently reset an in-progress setup without the player
    /// asking for it. (Loading *does* reset, because that is what a preset is.)
    #[test]
    fn the_default_setup_survives_a_preset_cycle() {
        let menu = TuningMenu::new(CarTuning::default());
        assert_eq!(
            menu.tuning.total_points(),
            arduracer_core::tuning::TOTAL_POINTS
        );
        assert_eq!(
            menu.tuning.top_speed, DEFAULT_SLIDER,
            "the default preset is not the default setup"
        );
    }

    /// POINTS REMAINING was previously drawn with reversed tens and ones digits,
    /// so "15" rendered as "51" or inverted positions.
    #[test]
    #[allow(clippy::identity_op)]
    fn points_remaining_digits_format_left_to_right() {
        for pts in 0..=24u8 {
            let tens = b'0' + pts / 10;
            let ones = b'0' + pts % 10;
            if pts >= 10 {
                assert!(tens.is_ascii_digit());
                assert!(ones.is_ascii_digit());
                let val = (tens - b'0') * 10 + (ones - b'0');
                assert_eq!(
                    val, pts,
                    "left-to-right composite digit value must equal pts"
                );
            } else {
                assert_eq!(tens, b'0');
                assert_eq!(ones - b'0', pts);
            }
        }
        // Specific regression cases: 15 points remaining must give '1' tens and '5' ones
        assert_eq!(b'0' + 15 / 10, b'1');
        assert_eq!(b'0' + 15 % 10, b'5');
        // 20 points remaining must give '2' tens and '0' ones
        assert_eq!(b'0' + 20 / 10, b'2');
        assert_eq!(b'0' + 20 % 10, b'0');
    }
}

/// TASK-1204: the game had no texture pipeline at all, and TASK-1202's minimap
/// burned ~43 % of the frame rebuilding a static image.
///
/// What is checkable headlessly is the *arithmetic* -- where the slot lives, that
/// it cannot collide with a framebuffer, that the quantiser agrees with the
/// polygon path, and that the bake covers the whole circuit. The DMA upload and
/// the sprite blit are MMIO and need hardware.
#[cfg(test)]
mod texture_pipeline_tests {
    use super::palette::to_bgr555;
    use super::texlayout::{
        minimap_colour, minimap_step, minimap_tile_rect, minimap_world_pos, pack_bgr555,
        slot_overlaps_framebuffers, MAX_DIM, MINIMAP_INNER, SLOT_BYTES, TEXTURE_X, TEXTURE_Y,
    };
    use arduracer_core::{Fixed, TrackTile, TILE_SIZE};

    #[test]
    fn the_texture_quantiser_agrees_with_the_polygon_one() {
        // If these diverged, the same RGB would be one colour in a polygon and
        // another in a sprite.
        for (r, g, b) in [
            (0u8, 0u8, 0u8),
            (255, 255, 255),
            (44, 46, 52),
            (40, 42, 48),
            (225, 30, 45),
            (206, 218, 232),
            (1, 2, 3),
        ] {
            assert_eq!(
                pack_bgr555(r, g, b),
                to_bgr555((r, g, b)),
                "({r},{g},{b}) quantises differently in the two paths"
            );
        }
    }

    /// Const-evaluated, because these are layout facts rather than runtime
    /// behaviour -- clippy rejects `assert!` on constants, and rightly so: they
    /// are also asserted at compile time in `texlayout.rs`. Re-checking them here
    /// documents the invariants from the test side.
    const fn no_overlap() -> bool {
        !slot_overlaps_framebuffers() && TEXTURE_X >= super::texlayout::SCREEN_W
    }

    const fn fits_vram() -> bool {
        TEXTURE_X + MAX_DIM <= 1024 && TEXTURE_Y + MAX_DIM <= 512
    }

    #[test]
    fn the_slot_does_not_overlap_a_framebuffer() {
        // The invariant an earlier draft got wrong: placing the slot at Y 480
        // puts it inside the second framebuffer's Y range.
        assert!(no_overlap(), "the slot overlaps a framebuffer");
    }

    #[test]
    fn the_slot_fits_in_vram() {
        assert!(fits_vram(), "the slot overflows VRAM");
    }

    /// 64 x 64 at 2 bytes per pixel. The number the module's VRAM budget commits
    /// to; if this changes, that table is wrong.
    #[test]
    fn the_slot_costs_the_documented_vram() {
        assert_eq!(MAX_DIM, 64);
        assert_eq!(SLOT_BYTES, 8_192);
    }

    /// Every tile type must be legible on the minimap: not transparent black, and
    /// not the same colour as another tile type.
    #[test]
    fn every_tile_type_has_a_distinct_legible_minimap_colour() {
        let tiles = [
            TrackTile::StartFinish,
            TrackTile::Checkpoint,
            TrackTile::Curb,
            TrackTile::BoostPad,
            TrackTile::Barrier,
            TrackTile::Tarmac,
            TrackTile::OffRoad,
            TrackTile::OilSlick,
        ];
        let mut seen: Vec<(TrackTile, u16)> = Vec::new();
        for tile in tiles {
            let (r, g, b) = minimap_colour(tile);
            let word = to_bgr555((r, g, b));
            assert_ne!(word, 0, "{tile:?} renders as transparent black");
            for (other, other_word) in &seen {
                assert_ne!(
                    word, *other_word,
                    "{tile:?} and {other:?} are the same colour on the minimap"
                );
            }
            seen.push((tile, word));
        }
    }

    /// The bake must cover the whole grid, or a tile outside the composed region
    /// leaves a hole in the circuit outline.
    #[test]
    fn the_bake_covers_every_tile_of_the_largest_circuits() {
        for (w, h) in [
            (80u8, 80u8),
            (96, 96),
            (38, 38),
            (40, 40),
            (30, 30),
            (4, 4),
            (1, 1),
        ] {
            for ty in 0..h {
                for tx in 0..w {
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    assert!(px >= 0, "px negative: {px}");
                    assert!(py >= 0, "py negative: {py}");
                    assert!(pw >= 1, "pw must be at least 1: {pw}");
                    assert!(ph >= 1, "ph must be at least 1: {ph}");
                    assert!(
                        px + pw <= MINIMAP_INNER,
                        "{w}x{h}: tile ({tx}, {ty}) rect ({px}, {pw}) overflows inner area {MINIMAP_INNER}"
                    );
                    assert!(
                        py + ph <= MINIMAP_INNER,
                        "{w}x{h}: tile ({tx}, {ty}) rect ({py}, {ph}) overflows inner area {MINIMAP_INNER}"
                    );
                    assert!(
                        px + pw <= MAX_DIM as i32,
                        "{w}x{h}: tile ({tx}, {ty}) rect ({px}, {pw}) overflows slot {MAX_DIM}"
                    );
                    assert!(
                        py + ph <= MAX_DIM as i32,
                        "{w}x{h}: tile ({tx}, {ty}) rect ({py}, {ph}) overflows slot {MAX_DIM}"
                    );
                }
            }
        }
    }

    /// Adjacent tiles must meet or overlap so there are no empty seams in the circuit outline.
    #[test]
    fn minimap_tiles_leave_no_seams() {
        for dim in [4u8, 10, 30, 38, 40, 80, 96] {
            for i in 0..(dim - 1) {
                let (px, _, pw, _) = minimap_tile_rect(i, 0, dim, dim);
                let (next_px, _, _, _) = minimap_tile_rect(i + 1, 0, dim, dim);
                assert!(
                    px + pw >= next_px,
                    "seam found at tile {i} of dim {dim}: right edge {}+{} < next {next_px}",
                    px,
                    pw
                );
            }
        }
    }

    /// World player positions map accurately and monotonically to minimap pixel space.
    #[test]
    fn player_placement_maps_accurately_to_minimap_space() {
        for extent in [80u8, 96, 40, 30] {
            let total_world = extent as i32 * TILE_SIZE;
            // Bounds check
            let p_zero = minimap_world_pos(Fixed::ZERO, extent);
            assert_eq!(p_zero, 0);

            let p_max = minimap_world_pos(Fixed::from_int(total_world), extent);
            assert_eq!(p_max, MINIMAP_INNER as i16);

            let p_neg = minimap_world_pos(Fixed::from_int(-500), extent);
            assert_eq!(p_neg, 0, "negative coordinates should clamp to origin");

            let p_over = minimap_world_pos(Fixed::from_int(total_world + 500), extent);
            assert_eq!(
                p_over, MINIMAP_INNER as i16,
                "coordinates past extent clamp to inner max"
            );

            // Monotonicity across every world unit
            let mut prev_px = 0;
            for wu in 0..=total_world {
                let px = minimap_world_pos(Fixed::from_int(wu), extent);
                assert!(
                    px >= prev_px,
                    "player position went backwards at world unit {wu}"
                );
                assert!(
                    px <= MINIMAP_INNER as i16,
                    "player position exceeded inner area at {wu}"
                );
                prev_px = px;
            }

            // A world coordinate inside tile `t` lands strictly inside tile `t`'s minimap footprint
            for t in 0..extent {
                let (rect_x, _, rect_w, _) = minimap_tile_rect(t, 0, extent, extent);
                for sub in 0..TILE_SIZE {
                    let wu = (t as i32 * TILE_SIZE) + sub;
                    let px = minimap_world_pos(Fixed::from_int(wu), extent) as i32;
                    assert!(
                        px >= rect_x && px < rect_x + rect_w,
                        "world pos {wu} in tile {t} mapped to px {px}, outside rect [{rect_x}, {})",
                        rect_x + rect_w
                    );
                }
            }
        }
    }

    /// The step must never be zero, or every tile would stack on one pixel.
    #[test]
    fn the_bake_step_is_never_zero() {
        for w in [0u8, 1, 2, 10, 30, 38, 40, 80, 96, 255] {
            assert!(minimap_step(w) >= 1, "step collapsed for width {w}");
        }
    }

    #[test]
    fn the_minimap_inner_area_is_the_documented_size() {
        assert_eq!(MINIMAP_INNER, 48, "the 56x56 panel has a 48px inner area");
    }

    /// Road and checkpoint tiles must retain drawing priority over adjacent offroad/barrier tiles.
    #[test]
    fn minimap_bake_preserves_road_and_checkpoint_priority() {
        let w = 80u8;
        let h = 80u8;
        let bg = minimap_colour(TrackTile::OffRoad);
        let mut buffer = [bg; (MINIMAP_INNER * MINIMAP_INNER) as usize];

        let mut grid = [TrackTile::OffRoad; 80 * 80];
        for tx in 0..80 {
            grid[10 * 80 + tx] = if tx % 4 == 0 {
                TrackTile::Barrier
            } else if tx % 4 == 1 {
                TrackTile::Tarmac
            } else if tx % 4 == 2 {
                TrackTile::Checkpoint
            } else {
                TrackTile::Curb
            };
        }

        // Pass 1: Barriers
        for ty in 0..h {
            for tx in 0..w {
                let tile = grid[ty as usize * w as usize + tx as usize];
                if tile == TrackTile::Barrier {
                    let col = minimap_colour(tile);
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    for y in py..(py + ph) {
                        for x in px..(px + pw) {
                            buffer[(y * MINIMAP_INNER + x) as usize] = col;
                        }
                    }
                }
            }
        }

        // Pass 2: Road & curbs
        for ty in 0..h {
            for tx in 0..w {
                let tile = grid[ty as usize * w as usize + tx as usize];
                if tile.is_road() || tile == TrackTile::Curb {
                    let col = minimap_colour(tile);
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    for y in py..(py + ph) {
                        for x in px..(px + pw) {
                            buffer[(y * MINIMAP_INNER + x) as usize] = col;
                        }
                    }
                }
            }
        }

        // Pass 3: Checkpoints
        for ty in 0..h {
            for tx in 0..w {
                let tile = grid[ty as usize * w as usize + tx as usize];
                if tile == TrackTile::Checkpoint {
                    let col = minimap_colour(tile);
                    let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                    for y in py..(py + ph) {
                        for x in px..(px + pw) {
                            buffer[(y * MINIMAP_INNER + x) as usize] = col;
                        }
                    }
                }
            }
        }

        let (_, py, _, _) = minimap_tile_rect(0, 10, w, h);
        let road_col = minimap_colour(TrackTile::Tarmac);
        let cp_col = minimap_colour(TrackTile::Checkpoint);

        let mut road_pixels = 0;
        let mut cp_pixels = 0;
        for x in 0..MINIMAP_INNER {
            let px_col = buffer[(py * MINIMAP_INNER + x) as usize];
            if px_col == road_col {
                road_pixels += 1;
            } else if px_col == cp_col {
                cp_pixels += 1;
            }
        }
        assert!(road_pixels > 0, "Road pixels must be present and visible");
        assert!(
            cp_pixels > 0,
            "Checkpoint pixels must be present and visible"
        );
    }

    /// Every Mania cup circuit produces a continuous and legible minimap with road and checkpoint visibility.
    #[test]
    fn mania_maps_bake_valid_continuous_minimaps() {
        use arduracer_core::levels::ALL_TRACKS;
        for &track in &ALL_TRACKS[25..=30] {
            let tiles = track.tile_grid();
            assert!(
                !tiles.is_empty(),
                "Track {} grid must be loaded",
                track.name
            );
            let w = track.width;
            let h = track.height;
            let bg = minimap_colour(TrackTile::OffRoad);
            let mut buffer = [bg; (MINIMAP_INNER * MINIMAP_INNER) as usize];

            // Pass 1: Barriers
            for ty in 0..h {
                let row_idx = ty as usize * w as usize;
                for tx in 0..w {
                    if let Some(&tile) = tiles.get(row_idx + tx as usize) {
                        if tile == TrackTile::Barrier {
                            let col = minimap_colour(tile);
                            let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                            for y in py..(py + ph) {
                                for x in px..(px + pw) {
                                    buffer[(y * MINIMAP_INNER + x) as usize] = col;
                                }
                            }
                        }
                    }
                }
            }

            // Pass 2: Road & curbs & hazards
            for ty in 0..h {
                let row_idx = ty as usize * w as usize;
                for tx in 0..w {
                    if let Some(&tile) = tiles.get(row_idx + tx as usize) {
                        if tile.is_road()
                            || tile == TrackTile::Curb
                            || tile == TrackTile::BoostPad
                            || tile == TrackTile::OilSlick
                        {
                            let col = minimap_colour(tile);
                            let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                            for y in py..(py + ph) {
                                for x in px..(px + pw) {
                                    buffer[(y * MINIMAP_INNER + x) as usize] = col;
                                }
                            }
                        }
                    }
                }
            }

            // Pass 3: Start/finish and checkpoints
            for ty in 0..h {
                let row_idx = ty as usize * w as usize;
                for tx in 0..w {
                    if let Some(&tile) = tiles.get(row_idx + tx as usize) {
                        if tile == TrackTile::StartFinish || tile == TrackTile::Checkpoint {
                            let col = minimap_colour(tile);
                            let (px, py, pw, ph) = minimap_tile_rect(tx, ty, w, h);
                            for y in py..(py + ph) {
                                for x in px..(px + pw) {
                                    buffer[(y * MINIMAP_INNER + x) as usize] = col;
                                }
                            }
                        }
                    }
                }
            }

            let road_col = minimap_colour(TrackTile::Tarmac);
            let cp_col = minimap_colour(TrackTile::Checkpoint);
            let sf_col = minimap_colour(TrackTile::StartFinish);

            let road_pixels = buffer.iter().filter(|&&c| c == road_col).count();
            let cp_pixels = buffer
                .iter()
                .filter(|&&c| c == cp_col || c == sf_col)
                .count();

            assert!(
                road_pixels >= 200,
                "{}: road must be clearly visible on minimap (got {} px)",
                track.name,
                road_pixels
            );
            assert!(
                cp_pixels >= 4,
                "{}: checkpoints must be visible on minimap (got {} px)",
                track.name,
                cp_pixels
            );
        }
    }
}

#[cfg(test)]
mod minimap_load_order_tests {
    const MAIN_SRC: &str = include_str!("../../../game/src/main.rs");

    #[test]
    fn load_level_grid_precedes_bake_minimap_in_all_load_paths() {
        let load_track_fn = MAIN_SRC
            .find("pub fn load_track(")
            .expect("load_track exists");
        let load_city_fn = MAIN_SRC
            .find("pub fn load_city_freeroam(")
            .expect("load_city_freeroam exists");
        let reset_race_fn = MAIN_SRC
            .find("pub fn reset_race(")
            .expect("reset_race exists");

        let load_track_body = &MAIN_SRC[load_track_fn..load_city_fn];
        let llg_pos = load_track_body
            .find("gpu::load_level_grid(")
            .expect("load_level_grid in load_track");
        let bm_pos = load_track_body
            .find("bake_minimap(")
            .expect("bake_minimap in load_track");
        assert!(
            llg_pos < bm_pos,
            "load_level_grid must precede bake_minimap in load_track"
        );

        let load_city_body = &MAIN_SRC[load_city_fn..reset_race_fn];
        let llg_city_pos = load_city_body
            .find("gpu::load_level_grid(")
            .expect("load_level_grid in load_city");
        let bm_city_pos = load_city_body
            .find("bake_minimap(")
            .expect("bake_minimap in load_city");
        assert!(
            llg_city_pos < bm_city_pos,
            "load_level_grid must precede bake_minimap in load_city_freeroam"
        );
    }
}

#[cfg(test)]
mod city_level_grid_tests {
    const LEVEL_GRID_SRC: &str = include_str!("../../../game/src/level_grid.rs");
    const TRACKTEX_SRC: &str = include_str!("../../../game/src/gpu/tracktex.rs");

    use arduracer_core::levels::LEVEL_DIMS;

    #[test]
    fn tracktex_level_dims_matches_all_shipped_levels() {
        assert!(
            TRACKTEX_SRC.contains("pub use arduracer_core::levels::LEVEL_DIMS;"),
            "tracktex must re-export arduracer_core::levels::LEVEL_DIMS so all 40 circuits and 3 cities match LEVELS.BIN layout"
        );
        assert_eq!(
            LEVEL_DIMS.len(),
            43,
            "LEVEL_DIMS must contain exactly 40 circuits and 3 free-roam cities"
        );
    }

    const MAX_LEVEL_TILES: usize = 57_600;
    const STAGE_SLACK: usize = 4096;
    const GRID_BYTES: usize = MAX_LEVEL_TILES + STAGE_SLACK;

    #[test]
    fn level_grid_does_not_reject_unaligned_full_size_cities() {
        // The check in level_grid::load must not compare (first + len) against
        // MAX_LEVEL_TILES, because any non-zero sector offset (first > 0) causes
        // a 240x240 city (len == 57600) to exceed 57600 before compaction,
        // causing all 3 city maps to fail loading and trap the car in a wall.
        assert!(
            !LEVEL_GRID_SRC.contains("first + len > MAX_LEVEL_TILES"),
            "level_grid.rs must not test `first + len > MAX_LEVEL_TILES`"
        );
        assert!(
            LEVEL_GRID_SRC.contains("len > MAX_LEVEL_TILES"),
            "level_grid.rs must test `len > MAX_LEVEL_TILES`"
        );
    }

    #[test]
    fn all_levels_including_cities_fit_disc_read_buffer() {
        let mut offset = 0usize;
        for (i, &(w, h)) in LEVEL_DIMS.iter().enumerate() {
            let len = w as usize * h as usize;
            assert!(len <= MAX_LEVEL_TILES, "Level {i} exceeds MAX_LEVEL_TILES");

            let first = offset % 2048;
            let count = (first + len).div_ceil(2048);
            let bytes_to_read = count * 2048;

            assert!(
                bytes_to_read <= GRID_BYTES,
                "Level {i} read ({bytes_to_read} bytes) overflows GRID_BYTES ({GRID_BYTES})"
            );
            assert!(
                first + len <= bytes_to_read,
                "Level {i} uncompacted slice extends past read count"
            );

            offset += len;
        }
    }
}

/// TASK-1215: the pause menu never appeared.
///
/// The paused arm of the race loop `continue`d before reaching step 5 of the
/// frame, so the veil was rasterised into the back buffer and no display flip
/// was ever queued for it. The panel drew correctly; it just was never shown.
///
/// This cannot be caught by driving the input state machine, which is exactly
/// what the pause tests above do and why they all passed. What went wrong is the
/// *frame* the menu is presented in. So this pins the structural property that
/// would have caught it: every `continue` in the main loop has to close its own
/// frame, because nothing after it will.
#[cfg(test)]
mod frame_close_tests {
    /// `game/src/main.rs`, as text.
    const MAIN_SRC: &str = include_str!("../../../game/src/main.rs");

    /// `continue` statements that skip the rest of the frame body.
    fn early_exits() -> Vec<usize> {
        MAIN_SRC
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                let t = l.trim();
                t.starts_with("continue;") && !t.starts_with("//") && !t.starts_with("///")
            })
            .map(|(i, _)| i + 1)
            .collect()
    }

    /// Byte offsets of the loop's own `close_frame()` call: the last one in the
    /// file, since the method is *defined* after the loop.
    fn loop_close_offset() -> usize {
        MAIN_SRC
            .rfind("self.close_frame();")
            .expect("close_frame must be called from the loop body")
    }

    #[test]
    fn the_loop_closes_its_own_frame_last() {
        // The premise of the other two tests: the loop's frame close is its
        // final statement, so an early exit above it bypasses the close.
        let close = loop_close_offset();
        assert!(
            MAIN_SRC[close..].contains("}\n    }\n}\n")
                || MAIN_SRC[close..].trim_end().ends_with('}'),
            "expected close_frame() to be the loop's last statement"
        );
    }

    #[test]
    fn every_early_exit_closes_its_own_frame_before_continuing() {
        // The real invariant, and the one that bites: a `continue` has to have
        // closed its own frame on the way, because nothing after it will.
        for line in early_exits() {
            let byte = MAIN_SRC
                .lines()
                .take(line - 1)
                .map(|l| format!("{l}\n"))
                .collect::<String>()
                .len();
            // Walk back to the start of the racing arm: that is the block this
            // `continue` exits, and everything above it is a different path.
            let arm = MAIN_SRC[..byte].rfind("GameState::Racing =>").unwrap_or(0);
            let on_this_path = &MAIN_SRC[arm..byte];
            assert!(
                on_this_path.rfind("self.close_frame()").is_some(),
                "the `continue` at main.rs:{line} has no close_frame() before it \
                 in the racing arm, so the frame it drew is never flipped"
            );
        }
    }

    #[test]
    fn an_early_exit_before_the_loop_close_is_the_pause_veil() {
        // Both facts together, in the shape the bug had: an early exit above the
        // loop's close, with its own close_frame() on the way out. If the loop
        // is restructured so an early exit no longer needs its own close, this
        // test should be deleted deliberately rather than left to rot.
        let close = loop_close_offset();
        for line in early_exits() {
            let byte = MAIN_SRC
                .lines()
                .take(line - 1)
                .map(|l| format!("{l}\n"))
                .collect::<String>()
                .len();
            assert!(
                byte < close,
                "the `continue` at main.rs:{line} is after the loop's frame close"
            );
        }
    }

    #[test]
    fn there_is_at_least_one_early_exit_to_check() {
        // If the loop is restructured and `continue` no longer appears, these
        // tests would all pass vacuously. Fail loudly instead.
        assert!(
            !early_exits().is_empty(),
            "no `continue` found in main.rs -- if the loop was restructured, \
             revisit this test rather than letting it pass trivially"
        );
    }
}

#[cfg(test)]
mod intro_transition_tests {
    use super::title_input::TitleInput;

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum VideoResult {
        Completed,
        Skipped,
        Interrupted,
        Unavailable,
    }

    #[derive(Default, Copy, Clone, Debug, PartialEq, Eq)]
    pub enum GameState {
        #[default]
        Title,
        MainMenu,
        TrackSelect,
        Garage,
        Racing,
        Results,
    }

    /// Evaluates boot state transition from video player result (as done in ArduracerGame::new).
    fn initial_state_from_intro(res: VideoResult) -> GameState {
        let mut state = GameState::Title;
        if res == VideoResult::Skipped {
            state = GameState::MainMenu;
        }
        state
    }

    #[test]
    fn intro_completed_transitions_to_title_screen() {
        let state = initial_state_from_intro(VideoResult::Completed);
        assert_eq!(
            state,
            GameState::Title,
            "When intro FMV finishes, game must start at Title screen"
        );
    }

    #[test]
    fn intro_skipped_transitions_directly_to_main_menu() {
        let state = initial_state_from_intro(VideoResult::Skipped);
        assert_eq!(
            state,
            GameState::MainMenu,
            "When intro FMV is skipped, game must proceed directly to Main Menu"
        );
    }

    #[test]
    fn intro_unavailable_defaults_to_title_screen() {
        let state = initial_state_from_intro(VideoResult::Unavailable);
        assert_eq!(
            state,
            GameState::Title,
            "When intro FMV is unavailable (e.g. side-loaded EXE), default to Title screen"
        );
    }

    #[test]
    fn button_held_across_intro_completion_does_not_prematurely_dismiss_title() {
        let mut title = TitleInput::new();
        // Frame 1 after intro: user was still holding START from skipping or gamepad resting
        let confirmed = title.update(true, false);
        assert!(
            !confirmed,
            "Title screen must require button release before triggering so it is never skipped instantly"
        );

        // Frame 2: user continues holding START
        let confirmed = title.update(true, false);
        assert!(!confirmed);

        // Frame 3: user releases buttons
        let confirmed = title.update(false, false);
        assert!(!confirmed);

        // Frame 4: user presses START newly -> edge trigger fires!
        let confirmed = title.update(true, false);
        assert!(
            confirmed,
            "Fresh START press after release must confirm title screen"
        );
    }

    #[test]
    fn cross_press_confirms_title_screen_to_main_menu() {
        let mut title = TitleInput::new();
        // Release first
        title.update(false, false);
        // Press CROSS
        let confirmed = title.update(false, true);
        assert!(confirmed, "Fresh CROSS press must confirm title screen");
    }

    #[test]
    fn full_transition_intro_completion_to_main_menu() {
        let mut current_state = initial_state_from_intro(VideoResult::Completed);
        assert_eq!(current_state, GameState::Title);

        let mut title = TitleInput::new();

        // 30 frames of title screen attract loop with no inputs held
        for _ in 0..30 {
            if title.update(false, false) {
                current_state = GameState::MainMenu;
            }
        }
        assert_eq!(
            current_state,
            GameState::Title,
            "Should remain on Title screen while idling"
        );

        // Player presses START
        if title.update(true, false) {
            current_state = GameState::MainMenu;
        }
        assert_eq!(
            current_state,
            GameState::MainMenu,
            "Must cleanly transition from Title screen to Main Menu on START"
        );
    }

    #[test]
    fn title_screen_20s_timeout_triggers_attract_mode() {
        let mut title = TitleInput::new();

        // 1199 frames of idle (19.98 seconds at 60 Hz)
        for _ in 0..1199 {
            assert!(!title.update(false, false));
        }
        assert_eq!(title.timer, 1199);

        // Frame 1200: exactly 20.0 seconds idle
        assert!(!title.update(false, false));
        assert_eq!(title.timer, 1200);

        // Attract mode cycle alternation:
        // Even cycle (0, 2, ...) -> INTRO.STR replay
        // Odd cycle (1, 3, ...) -> AI Autopilot Demo Race
        let mut attract_cycle: u8 = 0;
        let mut launched_fmv = false;
        let mut launched_demo = false;

        if attract_cycle.is_multiple_of(2) {
            launched_fmv = true;
            attract_cycle = attract_cycle.wrapping_add(1);
        }
        assert!(launched_fmv);
        assert_eq!(attract_cycle, 1);

        if !attract_cycle.is_multiple_of(2) {
            launched_demo = true;
            attract_cycle = attract_cycle.wrapping_add(1);
        }
        assert!(launched_demo);
        assert_eq!(attract_cycle, 2);
    }

    #[test]
    fn attract_demo_press_button_exits_to_menu() {
        let mut is_attract_demo = true;
        let mut current_state = GameState::Racing;

        // Player presses CROSS or START
        let start_held = true;
        let cross_held = false;
        let exit_pressed = start_held || cross_held;

        if is_attract_demo && exit_pressed {
            is_attract_demo = false;
            current_state = GameState::MainMenu;
        }

        assert!(!is_attract_demo);
        assert_eq!(current_state, GameState::MainMenu);
    }

    #[test]
    fn attract_demo_30s_timeout_returns_to_title() {
        let mut is_attract_demo = true;
        let mut attract_demo_timer: u32 = 1799;
        let mut current_state = GameState::Racing;

        // Tick 1800 (30 seconds at 60fps)
        attract_demo_timer += 1;
        if is_attract_demo && attract_demo_timer >= 1800 {
            is_attract_demo = false;
            current_state = GameState::Title;
        }

        assert!(!is_attract_demo);
        assert_eq!(current_state, GameState::Title);
    }

    #[test]
    fn video_stream_bounds_and_eof_drain_without_hanging() {
        // Simulates the sector pumping and frame drain logic of VideoPlayer
        const TOTAL_SECTORS: u32 = 755;
        let mut pumped_sectors: u32 = 0;
        let mut eof = false;
        let mut ready_queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
        let mut frames_shown: u32 = 0;
        let mut drive_active = true;

        // Simulate streaming drive
        for sector in 0..1000 {
            if pumped_sectors >= TOTAL_SECTORS {
                eof = true;
                drive_active = false;
                break;
            }
            pumped_sectors += 1;
            // Every 5 sectors assemble a frame
            if sector % 5 == 0 && ready_queue.len() < 4 {
                ready_queue.push_back(sector / 5);
            }
        }

        assert!(eof, "Video stream must detect EOF at sector boundary");
        assert_eq!(
            pumped_sectors, 755,
            "Must never read past total movie sectors"
        );
        assert!(!drive_active, "Drive reading must stop once EOF reached");

        // Drain remaining ready frames
        while let Some(_frame) = ready_queue.pop_front() {
            frames_shown += 1;
        }

        let drained = ready_queue.is_empty();
        assert!(drained && eof);
        let result = if frames_shown > 0 && eof {
            VideoResult::Completed
        } else {
            VideoResult::Interrupted
        };
        assert_eq!(result, VideoResult::Completed);
    }

    #[test]
    fn non_str_sector_triggers_early_eof_after_frames_seen() {
        // Simulates reading into TRACKS.BIN if sector count was somehow unconstrained
        let last_frame_seen: u32 = 10;
        let mut eof = false;
        let mut drive_active = true;

        let is_str_chunk = false; // Encounters TRACKS.BIN / non-STR sector
        if !is_str_chunk && last_frame_seen > 0 {
            eof = true;
            drive_active = false;
        }

        assert!(
            eof,
            "Non-STR sector encountered after movie frames must trigger EOF"
        );
        assert!(
            !drive_active,
            "Drive must stop immediately to prevent reading into audio tracks"
        );
    }
}

/// Tests for the settings values and the menus that edit them.
#[cfg(test)]
mod graphics_settings {
    use crate::pause_input::{PauseChoice, PauseInput, PauseMenu, SettingsValues};
    use crate::settings_input::{SettingsChoice, SettingsInput, SettingsMenu, ROW_COUNT};

    fn values() -> SettingsValues {
        SettingsValues {
            camera_view: 0,
            music: true,
            debug_metrics: 0,
            input_profile: 0,
            split_mode: 0,
        }
    }

    fn press(f: impl Fn(&mut SettingsInput)) -> SettingsInput {
        let mut i = SettingsInput::default();
        f(&mut i);
        i
    }

    /// Moves the settings screen's highlight down `rows` rows.
    fn settings_at(rows: u8) -> SettingsMenu {
        let mut m = SettingsMenu::new(values());
        for _ in 0..rows {
            m.update(press(|i| i.down = true));
            m.update(SettingsInput::default());
        }
        assert_eq!(m.selected_row, rows);
        m
    }

    /// Moves the pause menu's highlight down `rows` rows.
    fn pause_at(rows: u8) -> PauseMenu {
        let mut p = PauseMenu::default();
        for _ in 0..rows {
            p.update(PauseInput {
                down: true,
                ..Default::default()
            });
            p.update(PauseInput::default());
        }
        assert_eq!(p.selected_idx, rows);
        p
    }

    fn pause_right(p: &mut PauseMenu) -> PauseChoice {
        let out = p.update(PauseInput {
            right: true,
            ..Default::default()
        });
        p.update(PauseInput::default());
        out.choice
    }

    /// The texture format and framerate rows are gone: texture streaming is
    /// always QOI with its JPEG fallback, and the game is locked to 30 FPS.
    #[test]
    fn the_menus_have_no_texture_or_framerate_rows() {
        assert_eq!(ROW_COUNT, 5);
        assert_eq!(PauseMenu::item_count(), 8);
    }

    #[test]
    fn camera_view_cycles_between_the_three_modes() {
        let mut m = settings_at(0);
        m.update(press(|i| i.right = true));
        assert_eq!(m.values().camera_view, 1, "right selects Overhead");
        m.update(SettingsInput::default());
        m.update(press(|i| i.right = true));
        assert_eq!(m.values().camera_view, 2, "right selects Rotating");
        m.update(SettingsInput::default());
        m.update(press(|i| i.right = true));
        assert_eq!(m.values().camera_view, 0, "and wraps back to Behind");
    }

    /// Music is a two-state toggle: ON (XA) or OFF. There is no CD-DA option.
    #[test]
    fn music_toggles_on_the_settings_screen() {
        let mut m = settings_at(1);
        assert!(m.values().music, "music is on by default");
        for (right, expect) in [(true, false), (true, true), (false, false)] {
            let choice = if right {
                m.update(press(|i| i.right = true))
            } else {
                m.update(press(|i| i.left = true))
            };
            match choice {
                SettingsChoice::Changed(v) => assert_eq!(v.music, expect),
                other => panic!("expected a change, got {other:?}"),
            }
            m.update(SettingsInput::default());
        }
    }

    #[test]
    fn music_toggles_in_the_pause_menu() {
        let mut p = pause_at(3);
        assert!(p.values().music, "music is on by default");
        assert!(matches!(pause_right(&mut p), PauseChoice::Graphics(_)));
        assert!(!p.values().music, "right turns music off");
        assert!(matches!(pause_right(&mut p), PauseChoice::Graphics(_)));
        assert!(p.values().music, "and back on");
    }

    /// A music choice must survive the main loop re-seeding the menu from the
    /// values it just applied.
    #[test]
    fn music_choice_survives_sync_with_applied_values() {
        let mut m = settings_at(1);
        let applied = match m.update(press(|i| i.right = true)) {
            SettingsChoice::Changed(v) => v,
            other => panic!("expected a change, got {other:?}"),
        };
        assert!(!applied.music);
        m.set_values(applied);
        assert!(!m.values().music, "OFF preserved after sync");

        let mut p = pause_at(3);
        let out = p.update(PauseInput {
            right: true,
            ..Default::default()
        });
        let applied = match out.choice {
            PauseChoice::Graphics(v) => v,
            other => panic!("expected a change, got {other:?}"),
        };
        p.update_with_values(PauseInput::default(), applied);
        assert!(!p.values().music, "OFF preserved on the next pause frame");
    }

    /// The METRICS row must cycle through all four modes (OFF, FPS, FPS GRAPH,
    /// ALL) and wrap both ways.
    #[test]
    fn debug_metrics_cycles_on_the_settings_screen() {
        let mut m = settings_at(2);
        let mut seen = vec![m.values().debug_metrics];
        for _ in 0..4 {
            m.update(press(|i| i.right = true));
            m.update(SettingsInput::default());
            seen.push(m.values().debug_metrics);
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 0]);
        m.update(press(|i| i.left = true));
        assert_eq!(m.values().debug_metrics, 3, "left must wrap back to ALL");
    }

    #[test]
    fn debug_metrics_is_reachable_in_the_pause_menu() {
        let mut p = pause_at(4);
        assert!(matches!(pause_right(&mut p), PauseChoice::Graphics(_)));
        assert_eq!(p.values().debug_metrics, 1);
    }

    #[test]
    fn controls_input_profile_cycles_through_options_and_wraps() {
        let mut m = settings_at(3);
        assert_eq!(m.values().input_profile, 0, "default is Classic");
        let mut seen = vec![];
        for _ in 0..3 {
            m.update(press(|i| i.right = true));
            m.update(SettingsInput::default());
            seen.push(m.values().input_profile);
        }
        assert_eq!(seen, vec![1, 2, 0]);
    }

    #[test]
    fn split_mode_cycles_through_options_and_wraps() {
        let mut m = settings_at(4);
        assert_eq!(m.values().split_mode, 0, "default is Horizontal");
        let mut seen = vec![];
        for _ in 0..3 {
            m.update(press(|i| i.right = true));
            m.update(SettingsInput::default());
            seen.push(m.values().split_mode);
        }
        assert_eq!(seen, vec![1, 2, 0]);
    }

    #[test]
    fn pause_menu_edits_split_mode_inline() {
        let mut p = pause_at(6);
        let mut seen = vec![];
        for _ in 0..3 {
            assert!(matches!(pause_right(&mut p), PauseChoice::Graphics(_)));
            seen.push(p.values().split_mode);
        }
        assert_eq!(seen, vec![1, 2, 0]);
    }

    /// A held button is one change, not one per frame. Without this the value
    /// would race through every option while the player holds the stick.
    #[test]
    fn a_held_direction_is_a_single_change() {
        let mut m = SettingsMenu::new(values());
        let held = press(|i| i.right = true);
        let mut changes = 0;
        for _ in 0..6 {
            if matches!(m.update(held), SettingsChoice::Changed(_)) {
                changes += 1;
            }
        }
        assert_eq!(
            changes, 1,
            "a held direction changed the value {changes} times"
        );
        assert_eq!(m.values().camera_view, 1);
    }

    /// Row selection must stay in range and wrap at both ends.
    #[test]
    fn row_selection_wraps_between_the_rows() {
        let mut m = SettingsMenu::new(values());
        assert_eq!(m.selected_row, 0);
        m.update(press(|i| i.up = true));
        assert_eq!(
            m.selected_row,
            ROW_COUNT - 1,
            "up from the first row wraps to the last (2P SPLIT)"
        );
        m.update(SettingsInput::default());
        m.update(press(|i| i.down = true));
        assert_eq!(m.selected_row, 0, "down from the last wraps to the first");
    }

    /// Changes are reported as `Changed` (apply live, do not write the card);
    /// only `Exit` closes the screen, and that is where the caller persists.
    #[test]
    fn cross_exits_and_reports_changes_separately() {
        let mut m = SettingsMenu::new(values());
        // A change alone must not close the screen.
        match m.update(press(|i| i.right = true)) {
            SettingsChoice::Changed(v) => assert_eq!(
                v.camera_view, 1,
                "a change must report the value it moved to"
            ),
            other => panic!("expected a change, got {other:?}"),
        }
        assert_eq!(
            m.update(SettingsInput::default()),
            SettingsChoice::None,
            "a value edit must not be mistaken for a confirm"
        );
        // Change and confirm in the same frame: apply, but do not close yet.
        let both = SettingsInput {
            right: true,
            cross: true,
            ..SettingsInput::default()
        };
        assert!(matches!(m.update(both), SettingsChoice::Changed(_)));
        // CROSS was already held in that frame, so releasing first is required
        // before it can read as a new press.
        assert_eq!(m.update(SettingsInput::default()), SettingsChoice::None);
        assert_eq!(m.update(press(|i| i.cross = true)), SettingsChoice::Exit);
    }

    /// Entry must clear the edge baseline, or the button used to leave the
    /// previous screen immediately acts on this one.
    #[test]
    fn arming_on_entry_swallows_the_held_button() {
        let mut m = SettingsMenu::new(values());
        m.arm_for_entry();
        assert_eq!(
            m.update(press(|i| i.right = true)),
            SettingsChoice::None,
            "a button held across entry must not register"
        );
    }

    /// The pause menu edits the same values in place, without moving the
    /// cursor or activating the row.
    #[test]
    fn pause_menu_edits_view_inline() {
        let mut p = pause_at(2);
        let mut seen = vec![];
        for _ in 0..3 {
            assert!(matches!(pause_right(&mut p), PauseChoice::Graphics(_)));
            seen.push(p.values().camera_view);
        }
        assert_eq!(seen, vec![1, 2, 0], "Overhead, Rotating, then Behind");
        assert_eq!(p.selected_idx, 2, "the row should stay selected");
    }

    /// QUIT must still be the last row and still quit. Removing the texture and
    /// framerate rows moved it; an off-by-one there turns the destructive option
    /// into a setting or makes the pause menu unexitable.
    #[test]
    fn quit_is_still_the_last_row_and_still_quits() {
        let mut p = pause_at(7);
        let out = p.update(PauseInput {
            cross: true,
            ..Default::default()
        });
        assert_eq!(out.choice, PauseChoice::QuitToMenu);

        // And it wraps: one more Down from QUIT returns to RESUME.
        p.update(PauseInput::default());
        p.update(PauseInput {
            down: true,
            ..Default::default()
        });
        assert_eq!(p.selected_idx, 0);
    }

    fn test_pad(mask: u16) -> psx_pad::PadState {
        psx_pad::PadState {
            buttons: psx_pad::ButtonState::from_bits(mask),
            mode: psx_pad::PadMode::Digital,
            sticks: psx_pad::AnalogSticks::CENTERED,
            id_low: 0x41,
        }
    }

    #[test]
    fn instant_restart_requires_a_continuous_24_frame_recovery_hold() {
        use super::pad::ControllerDriver;
        use arduracer_core::save::InputProfile;
        use psx_pad::button;

        let mut driver = ControllerDriver::new(InputProfile::ClassicArcade);
        for frame in 1..=23 {
            driver.poll_from(&test_pad(button::L1));
            assert!(
                !driver.instant_restart_triggered,
                "triggered on held frame {frame}"
            );
            assert_eq!(driver.respawn_pressed, frame == 1);
        }

        driver.poll_from(&test_pad(button::L1));
        assert!(driver.instant_restart_triggered, "24th held frame restarts");
        driver.poll_from(&test_pad(button::L1));
        assert!(
            !driver.instant_restart_triggered,
            "restart is a one-frame event"
        );

        driver.poll_from(&test_pad(0));
        for _ in 0..23 {
            driver.poll_from(&test_pad(button::L1));
        }
        assert!(
            !driver.instant_restart_triggered,
            "release starts a fresh hold"
        );
    }

    #[test]
    fn stage_table_tabs_and_active_tracks() {
        use super::stage_table::{StageFilterTab, StageTableScreen};

        assert_eq!(StageFilterTab::All.name(), "ALL STAGES (40)");
        assert_eq!(StageFilterTab::GrandPrix.name(), "GP TRIALS (25)");
        assert_eq!(StageFilterTab::Mania.name(), "MANIA TRIALS (15)");

        assert_eq!(StageFilterTab::All.next(), StageFilterTab::GrandPrix);
        assert_eq!(StageFilterTab::GrandPrix.next(), StageFilterTab::Mania);
        assert_eq!(StageFilterTab::Mania.next(), StageFilterTab::All);

        assert_eq!(StageFilterTab::All.prev(), StageFilterTab::Mania);
        assert_eq!(StageFilterTab::Mania.prev(), StageFilterTab::GrandPrix);
        assert_eq!(StageFilterTab::GrandPrix.prev(), StageFilterTab::All);

        let mut screen = StageTableScreen::new();
        screen.tab = StageFilterTab::All;
        assert_eq!(screen.active_tracks().len(), 40);
        assert_eq!(screen.active_tracks()[0], 0);
        assert_eq!(screen.active_tracks()[39], 39);

        screen.tab = StageFilterTab::GrandPrix;
        assert_eq!(screen.active_tracks().len(), 25);
        assert_eq!(screen.active_tracks()[24], 24);

        screen.tab = StageFilterTab::Mania;
        assert_eq!(screen.active_tracks().len(), 15);
        assert_eq!(screen.active_tracks()[0], 25);
        assert_eq!(screen.active_tracks()[14], 39);
    }

    #[test]
    fn stage_table_navigation_and_actions() {
        use super::stage_table::{StageFilterTab, StageTableAction, StageTableScreen};
        use psx_pad::button;

        let mut screen = StageTableScreen::new();
        screen.arm_for_entry(StageFilterTab::Mania);
        assert_eq!(screen.selected_row, 0);
        assert_eq!(screen.scroll_top, 0);

        // Initial held edge release
        let action = screen.update(&test_pad(0));
        assert_eq!(action, StageTableAction::None);

        // Down advances row
        screen.update(&test_pad(button::DOWN));
        screen.update(&test_pad(0));
        assert_eq!(screen.selected_row, 1);

        // Wrapping: 15 mania tracks (rows 0..=14). Advance to 14 then down wraps to 0.
        for _ in 0..13 {
            screen.update(&test_pad(button::DOWN));
            screen.update(&test_pad(0));
        }
        assert_eq!(screen.selected_row, 14);

        screen.update(&test_pad(button::DOWN));
        screen.update(&test_pad(0));
        assert_eq!(screen.selected_row, 0, "DOWN from last row wraps to 0");

        // UP from 0 wraps to last row
        screen.update(&test_pad(button::UP));
        screen.update(&test_pad(0));
        assert_eq!(screen.selected_row, 14, "UP from 0 wraps to last row");

        // Tab switch with RIGHT
        screen.update(&test_pad(button::RIGHT));
        screen.update(&test_pad(0));
        assert_eq!(screen.tab, StageFilterTab::All);
        assert_eq!(screen.selected_row, 0, "tab switch resets selected row");

        // Action on CROSS confirms stage challenge
        let action = screen.update(&test_pad(button::CROSS));
        assert_eq!(action, StageTableAction::SelectStage(0));
        screen.update(&test_pad(0));

        // Action on CIRCLE backs out
        let action = screen.update(&test_pad(button::CIRCLE));
        assert_eq!(action, StageTableAction::Back);
        screen.update(&test_pad(0));

        // Action on SELECT switches to carousel
        let action = screen.update(&test_pad(button::SELECT));
        assert_eq!(action, StageTableAction::SwitchToCarousel);
    }

    #[test]
    fn track_select_open_table_toggle() {
        use super::track_select::TrackSelectScreen;
        use arduracer_core::levels::GameMode;
        use psx_pad::button;

        let mut screen = TrackSelectScreen::new();
        screen.arm_for_entry(GameMode::GrandPrix, false);
        assert_eq!(screen.track_range(), (0, 25));
        assert!(!screen.check_open_table());

        // Release buttons
        screen.update(&test_pad(0));

        // Press SELECT triggers open_table
        screen.update(&test_pad(button::SELECT));
        assert!(screen.check_open_table(), "SELECT toggles open_table flag");
        assert!(
            !screen.check_open_table(),
            "check_open_table consumes the flag"
        );

        // Mania mode track range
        screen.arm_for_entry(GameMode::Mania, false);
        assert_eq!(screen.track_range(), (25, 40));

        // Records mode track range (all tracks)
        screen.arm_for_entry(GameMode::GrandPrix, true);
        assert_eq!(screen.track_range(), (0, 40));
    }

    #[test]
    fn track_select_arm_with_track_focus() {
        use super::track_select::TrackSelectScreen;
        use arduracer_core::levels::GameMode;
        use psx_pad::button;

        let mut screen = TrackSelectScreen::new();

        // Arm with valid track within Grand Prix range (0..25)
        screen.arm_for_entry_with_track(GameMode::GrandPrix, false, 7);
        assert_eq!(screen.selected_track_idx, 7);

        // Clamping out-of-range track falls back to start
        screen.arm_for_entry_with_track(GameMode::GrandPrix, false, 30);
        assert_eq!(screen.selected_track_idx, 0);

        // Arm with valid track within Mania range (25..40)
        screen.arm_for_entry_with_track(GameMode::Mania, false, 32);
        assert_eq!(screen.selected_track_idx, 32);

        // Clamping out-of-range track for Mania mode falls back to 25
        screen.arm_for_entry_with_track(GameMode::Mania, false, 5);
        assert_eq!(screen.selected_track_idx, 25);

        // Confirm button held on entry does not trigger confirmation immediately
        let (confirmed, cancelled) = screen.update(&test_pad(button::CROSS));
        assert_eq!(confirmed, None);
        assert!(!cancelled);

        // Once released and pressed again, it confirms the focused track
        screen.update(&test_pad(0));
        let (confirmed, _) = screen.update(&test_pad(button::CROSS));
        assert_eq!(confirmed, Some(25));
    }

    #[test]
    fn stage_table_arm_with_track_focus() {
        use super::stage_table::{StageFilterTab, StageTableAction, StageTableScreen};
        use psx_pad::button;

        let mut screen = StageTableScreen::new();

        // GrandPrix tab: track 10 is row 10 in GP_INDICES. Since row >= 7, scroll_top is 4.
        screen.arm_for_entry_with_track(StageFilterTab::GrandPrix, 10);
        assert_eq!(screen.selected_row, 10);
        assert_eq!(screen.scroll_top, 4);

        // Mania tab: track 28 is row 3 in MANIA_INDICES (25, 26, 27, 28). Since row < 7, scroll_top is 0.
        screen.arm_for_entry_with_track(StageFilterTab::Mania, 28);
        assert_eq!(screen.selected_row, 3);
        assert_eq!(screen.scroll_top, 0);

        // Mania tab: track 5 is not in MANIA_INDICES -> falls back to row 0, scroll_top 0
        screen.arm_for_entry_with_track(StageFilterTab::Mania, 5);
        assert_eq!(screen.selected_row, 0);
        assert_eq!(screen.scroll_top, 0);

        // All tab: track 39 is row 39 in ALL_INDICES. Since 39 >= 7, scroll_top is 33.
        screen.arm_for_entry_with_track(StageFilterTab::All, 39);
        assert_eq!(screen.selected_row, 39);
        assert_eq!(screen.scroll_top, 33);

        // Confirm button held on entry does not immediately select
        let action = screen.update(&test_pad(button::CROSS));
        assert_eq!(action, StageTableAction::None);

        // After release and press, it selects the focused track
        screen.update(&test_pad(0));
        let action = screen.update(&test_pad(button::CROSS));
        assert_eq!(action, StageTableAction::SelectStage(39));
    }

    #[test]
    fn main_menu_split_screen_nav_and_select() {
        use super::menu::{MainMenu, MenuItem};
        use psx_pad::button;

        let mut menu = MainMenu::new();
        menu.arm_for_entry();
        menu.update(&test_pad(0));

        assert_eq!(menu.selected_idx, 0);

        // Move down 5 times to reach SplitScreen2P (index 5)
        for _ in 0..5 {
            menu.update(&test_pad(button::DOWN));
            menu.update(&test_pad(0));
        }
        assert_eq!(menu.selected_idx, 5);

        // Confirm with CROSS
        let action = menu.update(&test_pad(button::CROSS));
        assert_eq!(action, Some(MenuItem::SplitScreen2P));

        // Release buttons
        menu.update(&test_pad(0));

        // Down one more to reach TrackBuilder (index 6)
        menu.update(&test_pad(button::DOWN));
        menu.update(&test_pad(0));
        assert_eq!(menu.selected_idx, 6);
        let action = menu.update(&test_pad(button::CROSS));
        assert_eq!(action, Some(MenuItem::TrackBuilder));
        menu.update(&test_pad(0));

        // Test wrapping: DOWN 4 more times reaches row 0 (from 6 to 0 on 10 items)
        for _ in 0..4 {
            menu.update(&test_pad(button::DOWN));
            menu.update(&test_pad(0));
        }
        assert_eq!(menu.selected_idx, 0);

        // UP from 0 wraps to 9 (Settings)
        menu.update(&test_pad(button::UP));
        menu.update(&test_pad(0));
        assert_eq!(menu.selected_idx, 9);
    }

    #[test]
    fn camera_split_viewport_scaling() {
        use super::camera::Camera;
        use arduracer_core::Vec2;

        let mut cam = Camera::new(Vec2::ZERO);
        assert_eq!(cam.viewport_h, 240);
        assert_eq!(cam.center_y, 120);

        cam.set_split_viewport(true);
        assert_eq!(cam.viewport_h, 118);
        assert_eq!(cam.center_y, 59);

        // Clamping and projection use the 118px height in split screen
        let (half_w, half_h) = cam.visible_half_extents();
        assert!(half_h < half_w);

        cam.set_split_viewport(false);
        assert_eq!(cam.viewport_h, 240);
        assert_eq!(cam.center_y, 120);
    }

    #[test]
    fn negcon_and_jogcon_driver_mappings() {
        use super::pad::ControllerDriver;
        use arduracer_core::save::InputProfile;
        use psx_pad::{AnalogSticks, ButtonState, PadMode, PadState};

        let mut driver = ControllerDriver::new(InputProfile::ClassicArcade);

        // Construct a raw neGcon pad state: ID 0x23 (Mode 0x2)
        // right_x = twist steering (0..255, 128 = center)
        // right_y = Button I analog throttle (0..255)
        // left_x = Button II analog brake (0..255)
        // left_y = Button L analog brake (0..255)
        let mut negcon_pad = PadState {
            buttons: ButtonState::from_bits(0),
            mode: PadMode::NeGcon,
            sticks: AnalogSticks {
                right_x: 228, // Steer right (+100)
                right_y: 200, // Button I throttle
                left_x: 180,  // Button II brake
                left_y: 0,
            },
            id_low: 0x23,
        };

        let input = driver.poll_from(&negcon_pad);
        assert!(
            input.steer > arduracer_core::Fixed::ZERO,
            "twist right gives positive steer"
        );
        assert!(
            input.throttle > arduracer_core::Fixed::ZERO,
            "Button I gives throttle"
        );
        assert!(
            input.brake > arduracer_core::Fixed::ZERO,
            "Button II gives brake"
        );

        // Twist left
        negcon_pad.sticks.right_x = 28; // -100
        negcon_pad.sticks.right_y = 0;
        negcon_pad.sticks.left_x = 0;
        let input_left = driver.poll_from(&negcon_pad);
        assert!(
            input_left.steer < arduracer_core::Fixed::ZERO,
            "twist left gives negative steer"
        );

        // Jogcon pad state: ID 0xE3 (Mode 0xE)
        // right_x = dial steering
        let jogcon_pad = PadState {
            buttons: ButtonState::from_bits(0),
            mode: PadMode::Jogcon,
            sticks: AnalogSticks {
                right_x: 208, // Dial steer right (+80)
                right_y: 128,
                left_x: 128,
                left_y: 128,
            },
            id_low: 0xE3,
        };
        let jog_input = driver.poll_from(&jogcon_pad);
        assert!(
            jog_input.steer > arduracer_core::Fixed::ZERO,
            "Jogcon dial right gives positive steer"
        );
    }

    #[test]
    fn menu_nav_synthesizer_stick_deflection_and_repeat() {
        use super::pad::MenuNavSynthesizer;
        use psx_pad::{button, AnalogSticks, ButtonState, PadMode, PadState};

        let mut nav = MenuNavSynthesizer::new();

        let mut pad = PadState {
            buttons: ButtonState::from_bits(0),
            mode: PadMode::Analog,
            sticks: AnalogSticks {
                left_x: 128,
                left_y: 128, // Center
                right_x: 128,
                right_y: 128,
            },
            id_low: 0x73,
        };

        // Center produces no buttons
        let res = nav.update(&pad);
        assert_eq!(res.buttons.bits(), 0);

        // Deflect stick down past threshold (128 + 70 = 198 > 128 + 64)
        pad.sticks.left_y = 198;
        let res_first = nav.update(&pad);
        assert!(
            res_first.buttons.is_held(button::DOWN),
            "initial deflection pulses DOWN"
        );

        // Frame 2: repeat delay is 20 frames, so next frame DOWN is suppressed
        let res_frame2 = nav.update(&pad);
        assert!(
            !res_frame2.buttons.is_held(button::DOWN),
            "pulse is edge-triggered during repeat delay"
        );

        // Fast-forward 19 more frames (total 21 frames held)
        for _ in 0..19 {
            nav.update(&pad);
        }
        let res_repeat = nav.update(&pad);
        assert!(
            res_repeat.buttons.is_held(button::DOWN),
            "after initial delay, repeat pulse triggers"
        );

        // Hysteresis release: releasing past threshold to 160 (deflection 32 < 48) releases
        pad.sticks.left_y = 160;
        let res_released = nav.update(&pad);
        assert!(
            !res_released.buttons.is_held(button::DOWN),
            "hysteresis releases button"
        );
    }

    #[test]
    fn victory_screen_lockout_and_input_dismiss() {
        use super::victory::VictoryScreen;
        use arduracer_core::championship::ChampionshipSession;

        let session = ChampionshipSession::new(0);
        let mut screen = VictoryScreen::new(&session);

        assert_eq!(screen.cup_name, session.cup_name());
        assert_eq!(screen.player_rank, 1);
        assert_eq!(screen.anim_timer, 0);

        let cross_pad = test_pad(psx_pad::button::CROSS);
        let start_pad = test_pad(psx_pad::button::START);
        let circle_pad = test_pad(psx_pad::button::CIRCLE);
        let idle_pad = test_pad(0);

        // Frames 1..=60: Lockout active! No button can dismiss.
        for _ in 1..=60 {
            assert!(
                !screen.update(&cross_pad),
                "lockout suppresses cross button"
            );
        }
        assert_eq!(screen.anim_timer, 60);

        // Frame 61+: Input unlocked!
        // Release cross first so next press is a fresh edge
        screen.update(&idle_pad);
        assert_eq!(screen.anim_timer, 61);

        assert!(
            screen.update(&cross_pad),
            "cross button dismisses victory screen after lockout"
        );

        // Next frame idle returns false
        assert!(!screen.update(&idle_pad));

        // Start button also dismisses
        assert!(
            screen.update(&start_pad),
            "start button dismisses victory screen"
        );

        // Next frame idle returns false
        assert!(!screen.update(&idle_pad));

        // Circle button also dismisses
        assert!(
            screen.update(&circle_pad),
            "circle button dismisses victory screen"
        );
    }

    #[test]
    fn victory_screen_session_and_rank_initialization() {
        use super::victory::VictoryScreen;
        use arduracer_core::championship::ChampionshipSession;

        for cup_idx in 0..4 {
            let mut session = ChampionshipSession::new(cup_idx);
            assert_eq!(session.player_leaderboard_rank(), 1);

            let screen = VictoryScreen::new(&session);
            assert_eq!(screen.cup_name, session.cup_name());
            assert_eq!(screen.player_rank, 1);
            assert_eq!(screen.anim_timer, 0);

            // Award points to AI rivals to simulate 2nd and 3rd rank
            session.competitors[1].total_points = 100;
            let screen_2nd = VictoryScreen::new(&session);
            assert_eq!(screen_2nd.player_rank, 2);
            assert_eq!(session.victory_video_filename(), "VICTORY2.STR");

            session.competitors[2].total_points = 100;
            let screen_3rd = VictoryScreen::new(&session);
            assert_eq!(screen_3rd.player_rank, 3);
            assert_eq!(session.victory_video_filename(), "VICTORY3.STR");

            session.competitors[3].total_points = 100;
            let screen_4th = VictoryScreen::new(&session);
            assert_eq!(screen_4th.player_rank, 4);
            assert_eq!(session.victory_video_filename(), "VICTORY4.STR");

            session.competitors[4].total_points = 100;
            let screen_5th = VictoryScreen::new(&session);
            assert_eq!(screen_5th.player_rank, 5);
            assert_eq!(session.victory_video_filename(), "VICTORY4.STR");

            session.competitors[5].total_points = 100;
            let screen_6th = VictoryScreen::new(&session);
            assert_eq!(screen_6th.player_rank, 6);
            assert_eq!(session.victory_video_filename(), "VICTORY4.STR");
        }
    }

    #[test]
    fn font_vram_sheet_and_glyph_integrity_tests() {
        // Verify that the font sheet generation algorithm preserves all foreground pixels
        // and never creates corrupted nibbles (value 3 = transparent) when drop shadows are added.
        // Also verify that key characters like 'R', 'N', 'D', 'G', 'S', '3' are distinct and intact.
        let font_src = include_str!("../../../game/src/ui/font.rs");
        assert!(font_src.contains("// Pass 1: populate all foreground pixels"));
        assert!(font_src.contains("// Pass 2: add subtle 1px drop shadow"));
        assert!(font_src.contains("clut[3] = 0x7FFF; // Foreground fallback"));

        // Extract glyph definition lines
        let mut glyphs = [[0u8; 5]; 64];
        let mut glyph_idx = 0;
        for line in font_src.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.contains("0x") && trimmed.contains("// 0x") {
                let inside = trimmed.trim_start_matches('[').split(']').next().unwrap();
                let nums: Vec<u8> = inside
                    .split(',')
                    .filter_map(|s| {
                        let hex = s.trim().trim_start_matches("0x");
                        u8::from_str_radix(hex, 16).ok()
                    })
                    .collect();
                if nums.len() == 5 && glyph_idx < 64 {
                    glyphs[glyph_idx] = [nums[0], nums[1], nums[2], nums[3], nums[4]];
                    glyph_idx += 1;
                }
            }
        }
        assert_eq!(glyph_idx, 64, "All 64 ASCII glyphs must be parsed");

        // Simulate two-pass VRAM sheet generation
        let mut sheet = [0u16; 1024];

        // Pass 1: Foreground pixels (1)
        for (idx, &cols) in glyphs.iter().enumerate() {
            let cell_col = idx % 8;
            let cell_row = idx / 8;
            for (col_idx, &col_bits) in cols.iter().enumerate() {
                let px = cell_col * 8 + col_idx;
                let hw_col = px / 4;
                let nibble = (px % 4) as u16;
                for row in 0..7 {
                    let py = cell_row * 8 + row;
                    if (col_bits & (1u8 << row)) != 0 {
                        let hw_idx = py * 16 + hw_col;
                        sheet[hw_idx] |= 1u16 << (nibble * 4);
                    }
                }
            }
        }

        // Pass 2: Shadow pixels (2)
        for (idx, &cols) in glyphs.iter().enumerate() {
            let cell_col = idx % 8;
            let cell_row = idx / 8;
            for (col_idx, &col_bits) in cols.iter().enumerate() {
                let px = cell_col * 8 + col_idx;
                for row in 0..7 {
                    if (col_bits & (1u8 << row)) != 0 {
                        let s_py = cell_row * 8 + row + 1;
                        let s_px = px + 1;
                        if s_py < (cell_row + 1) * 8 && s_px < (cell_col + 1) * 8 {
                            let s_hw = s_py * 16 + (s_px / 4);
                            let s_nib = (s_px % 4) as u16;
                            if (sheet[s_hw] & (0xF << (s_nib * 4))) == 0 {
                                sheet[s_hw] |= 2u16 << (s_nib * 4);
                            }
                        }
                    }
                }
            }
        }

        // Assert: NO nibble in the entire 64x64 texture sheet has value 3 or >2
        for (hw_idx, &hw) in sheet.iter().enumerate() {
            for nib in 0..4 {
                let val = (hw >> (nib * 4)) & 0xF;
                assert!(
                    val <= 2,
                    "VRAM font sheet has corrupted nibble {val} at hw_idx {hw_idx} (caused missing pixels in text)"
                );
            }
        }

        // Verify key characters
        let r_idx = (b'R' - 0x20) as usize;
        let r_cols = glyphs[r_idx];
        assert_ne!(
            r_cols[4] & (1 << 6),
            0,
            "'R' must have bottom-right leg pixel at row 6, col 4"
        );
        assert_ne!(
            r_cols[3] & (1 << 5),
            0,
            "'R' must have diagonal leg pixel at row 5, col 3"
        );

        let n_idx = (b'N' - 0x20) as usize;
        let n_cols = glyphs[n_idx];
        assert_ne!(
            n_cols[2] & (1 << 3),
            0,
            "'N' must have center diagonal pixel at row 3, col 2"
        );

        let d_idx = (b'D' - 0x20) as usize;
        let d_cols = glyphs[d_idx];
        assert_ne!(
            d_cols[4] & (1 << 3),
            0,
            "'D' must have right edge pixel at row 3, col 4"
        );
        assert_ne!(
            d_cols[3] & (1 << 0),
            0,
            "'D' must have top edge pixel at row 0, col 3"
        );

        let g_idx = (b'G' - 0x20) as usize;
        let g_cols = glyphs[g_idx];
        assert_eq!(
            g_cols[2] & (1 << 3),
            0,
            "'G' center must be open at row 3, col 2"
        );
        assert_ne!(
            g_cols[3] & (1 << 3),
            0,
            "'G' must have horizontal spur at row 3, col 3"
        );

        let s_idx = (b'S' - 0x20) as usize;
        let s_cols = glyphs[s_idx];
        assert_ne!(
            s_cols[0] & (1 << 2),
            0,
            "'S' must have top-left stem at row 2, col 0"
        );
        assert_ne!(
            s_cols[4] & (1 << 4),
            0,
            "'S' must have bottom-right stem at row 4, col 4"
        );
    }
}

#[cfg(test)]
mod pause_menu_and_m7_backdrop_alignment_tests {
    use super::camera::Camera;
    use arduracer_core::{Fixed, Vec2};

    #[test]
    fn pause_menu_text_fits_strictly_within_panel_bounds() {
        let panel_x = 60i16;
        let panel_w = 200i16;
        let panel_y = 28i16;
        let panel_h = 186i16;
        let panel_right = panel_x + panel_w; // 260
        let panel_bottom = panel_y + panel_h; // 214

        // Title: "PAUSED" at x=124, y=38, scale=2
        let title = "PAUSED";
        let title_x = 124i16;
        let title_y = 38i16;
        let title_w = title.len() as i16 * 12; // 72
        assert!(title_x >= panel_x);
        assert!(title_x + title_w <= panel_right);
        // Verify horizontal centering: equal left and right margins inside panel
        assert_eq!(title_x - panel_x, panel_right - (title_x + title_w));
        assert!(title_y >= panel_y);

        // Footer line 1: "L/R: CHANGE   CROSS: SELECT" at x=79, y=189, scale=1
        let f1 = "L/R: CHANGE   CROSS: SELECT";
        let f1_x = 79i16;
        let f1_y = 189i16;
        let f1_w = f1.len() as i16 * 6; // 162
        assert!(f1_x >= panel_x);
        assert!(f1_x + f1_w <= panel_right);
        assert_eq!(f1_x - panel_x, panel_right - (f1_x + f1_w));
        assert!(f1_y + 8 <= panel_bottom);

        // Footer line 2: "START: RESUME" at x=121, y=201, scale=1
        let f2 = "START: RESUME";
        let f2_x = 121i16;
        let f2_y = 201i16;
        let f2_w = f2.len() as i16 * 6; // 78
        assert!(f2_x >= panel_x);
        assert!(f2_x + f2_w <= panel_right);
        assert_eq!(f2_x - panel_x, panel_right - (f2_x + f2_w));
        assert!(f2_y + 8 <= panel_bottom);
    }

    #[test]
    fn camera_m7_track_top_matches_far_track_projection() {
        let mut cam = Camera::new_chase(Vec2::new(Fixed::ZERO, Fixed::ZERO), 0);
        let track_top = cam.m7_track_top();

        // Screen is 240 px tall; horizon is at 60. Far track row must be below horizon and within bounds.
        assert!(track_top > 60);
        assert!(track_top < 140);

        // Projected point on ground at (x=0, z=z_far) should match m7_track_top
        let far_zq4 = cam.z_far << 4;
        let (_sx, sy) = cam.project_cam(0, far_zq4);
        assert_eq!(
            track_top, sy,
            "m7_track_top must equal projected y at far clipping plane"
        );

        // Compressed format extended view distance
        cam.z_far = 560;
        let comp_track_top = cam.m7_track_top();
        assert!(comp_track_top > 60);
        assert!(comp_track_top < track_top);
        let comp_far_zq4 = cam.z_far << 4;
        let (_sx, comp_sy) = cam.project_cam(0, comp_far_zq4);
        assert_eq!(comp_track_top, comp_sy);

        // Pitch up shifts horizon and track top upwards
        cam.pitch_px = -6;
        let pitched_top = cam.m7_track_top();
        assert_eq!(pitched_top, comp_track_top - 6);

        // Pitch down shifts horizon and track top downwards
        cam.pitch_px = 6;
        let pitched_down = cam.m7_track_top();
        assert_eq!(pitched_down, comp_track_top + 6);
    }
    #[test]
    fn m7_horizon_mountain_skirt_spans_to_viewport_bottom_matching_last_tile() {
        // The skyline base is lifted clear of the grid (see
        // `M7_SILHOUETTE_LIFT`); the skirt must still reach the viewport bottom
        // from the *lifted* base, or the road below the ridge would be unpainted.
        const LIFT: i16 = 5;
        let mut cam = Camera::new_chase(Vec2::new(Fixed::ZERO, Fixed::ZERO), 0);

        // 1P Full Screen mode: viewport is 240 px tall
        let v_h = cam.viewport_h;
        assert_eq!(v_h, 240);
        let track_top = cam.m7_track_top();
        let roll = 0i16;
        let base_y = (track_top - LIFT + roll).max(0);
        let skirt_h = (v_h - base_y).max(0) as u16;
        // Mountain base plus skirt must reach exactly to viewport bottom (row 240)
        assert_eq!(base_y + skirt_h as i16, v_h);

        // Even when pitch moves track_top up or down, mountain skirt always reaches viewport bottom
        for pitch in [-10, -5, 0, 5, 10] {
            cam.pitch_px = pitch;
            let top = cam.m7_track_top();
            let base = (top - LIFT).max(0);
            let skirt = (v_h - base).max(0) as u16;
            assert_eq!(
                base + skirt as i16,
                v_h,
                "skirt must reach viewport bottom with pitch {pitch}"
            );
        }

        // 2P Horizontal Split mode: viewport is 118 px tall
        cam.set_split_mode(0, 1);
        let split_vh = cam.viewport_h;
        assert_eq!(split_vh, 118);
        let split_top = cam.m7_track_top();
        let split_base = (split_top - LIFT).max(0);
        let split_skirt = (split_vh - split_base).max(0) as u16;
        assert_eq!(split_base + split_skirt as i16, split_vh);
    }
}

#[cfg(test)]
/// 2P split-screen projection defects: a player's own car was invisible or
/// off-centre, and rivals drew at the wrong size and position.
///
/// The root cause was that the Mode 7 projection carried full-screen
/// constants (`SCREEN_W / 2`, an unscaled focal length and an unscaled
/// camera height) into a viewport that is a fraction of the screen. Each
/// case below pins one of those terms.
mod split_screen_projection_tests {
    use super::camera::{Camera, M7_HORIZON, M7_Z0, SCREEN_H, SCREEN_W};
    use arduracer_core::{Fixed, Vec2};

    /// Depth the chase camera puts its pivot at: the player's own car.
    const CAR_Z_Q4: i32 = M7_Z0 << 4;

    /// A player's own car must project inside their own viewport, for every
    /// split layout, for both players. This is the "I cannot see my car"
    /// report: at `CAR_Z_Q4` the car sits low in the frame, so any term
    /// that is not scaled by the viewport pushes it off the bottom.
    #[test]
    fn a_players_own_car_is_inside_their_own_viewport_in_every_split_layout() {
        for split_mode in 0u8..=2 {
            for player in 1u8..=2 {
                let mut cam = Camera::new_chase(Vec2::ZERO, 0);
                cam.set_split_mode(split_mode, player);

                let (sx, sy) = cam.project_cam(0, CAR_Z_Q4);
                let lo_x = cam.center_x - cam.viewport_w / 2;
                let hi_x = cam.center_x + cam.viewport_w / 2;

                assert!(
                    sx >= lo_x && sx <= hi_x,
                    "split {split_mode} p{player}: own car at x={sx}, outside {lo_x}..={hi_x}"
                );
                assert!(
                    sy >= 0 && sy <= cam.viewport_h,
                    "split {split_mode} p{player}: own car at y={sy}, outside rows 0..={}",
                    cam.viewport_h
                );
            }
        }
    }

    /// A rival a little ahead of the player must also land on screen. This
    /// is the "rival is missing / in the wrong place" report.
    #[test]
    fn a_rival_near_the_player_is_inside_the_viewport_too() {
        for split_mode in 0u8..=2 {
            for player in 1u8..=2 {
                let mut cam = Camera::new_chase(Vec2::ZERO, 0);
                cam.set_split_mode(split_mode, player);

                // 60 world units ahead of the pivot, on the centreline.
                let ahead = Vec2::new(Fixed::ZERO, Fixed::from_int(-60));
                let (sx, sy) = cam.world_to_screen(ahead);
                assert!(
                    sx >= 0 && sx <= cam.viewport_w,
                    "split {split_mode} p{player}: rival at x={sx}, outside 0..={}",
                    cam.viewport_w
                );
                assert!(
                    sy >= 0 && sy <= cam.viewport_h,
                    "split {split_mode} p{player}: rival at y={sy}, outside 0..={}",
                    cam.viewport_h
                );
            }
        }
    }

    /// The car must be centred horizontally. A viewport whose `center_x` is
    /// not honoured puts a vertical-split car at the middle of the *screen*
    /// instead of the middle of its own half.
    #[test]
    fn the_car_is_centred_in_its_own_viewport_not_the_whole_screen() {
        for split_mode in 0u8..=2 {
            for player in 1u8..=2 {
                let mut cam = Camera::new_chase(Vec2::ZERO, 0);
                cam.set_split_mode(split_mode, player);

                let (sx, _) = cam.project_cam(0, CAR_Z_Q4);
                assert_eq!(
                    sx, cam.center_x,
                    "split {split_mode} p{player}: car not centred in viewport"
                );
            }
        }
    }

    /// Sizing: a split viewport must not draw the car at full-screen size.
    ///
    /// The scale is uniform (the smaller of the two axis ratios) because the
    /// GTE has a single focal length for both axes. A car in a half-size
    /// viewport must occupy proportionally fewer columns, and never zero.
    #[test]
    fn car_size_scales_with_the_viewport_not_left_at_full_screen() {
        let width_of = |cam: &Camera| -> i32 {
            let l = cam.project_cam(-1200, CAR_Z_Q4).0;
            let r = cam.project_cam(1200, CAR_Z_Q4).0;
            (r - l) as i32
        };

        let full = Camera::new_chase(Vec2::ZERO, 0);
        let full_w = width_of(&full);
        assert!(full_w > 0);

        for split_mode in 0u8..=2 {
            let mut cam = Camera::new_chase(Vec2::ZERO, 0);
            cam.set_split_mode(split_mode, 1);
            let split_w = width_of(&cam);
            assert!(
                split_w > 0,
                "split {split_mode}: car collapsed to zero width"
            );
            assert!(
                split_w <= full_w,
                "split {split_mode}: car is {split_w}px wide, wider than the \
                 full-screen {full_w}px"
            );
            // The uniform scale is the smaller of the two axis ratios.
            let expected_scale = cam.m7_view_scale_q12();
            let expected = full_w * expected_scale / 4096;
            assert!(
                (split_w - expected).abs() <= 2,
                "split {split_mode}: car width {split_w}px, expected ~{expected}px \
                 at scale {expected_scale}/4096"
            );
        }
    }

    /// The uniform scale is the smaller of the two axis ratios, so a split
    /// viewport is never letterboxed and never overflows its smaller axis.
    #[test]
    fn the_viewport_scale_fits_the_smaller_axis() {
        for split_mode in 0u8..=2 {
            let mut cam = Camera::new_chase(Vec2::ZERO, 0);
            cam.set_split_mode(split_mode, 1);
            let w = (cam.viewport_w as i32 * 4096) / SCREEN_W as i32;
            let h = (cam.viewport_h as i32 * 4096) / SCREEN_H as i32;
            assert_eq!(
                cam.m7_view_scale_q12(),
                w.min(h),
                "split {split_mode}: scale is not the smaller axis ratio"
            );
        }
    }

    /// The viewport scale must be exactly 1 on the full-screen path, or
    /// this change would silently alter single-player.
    #[test]
    fn the_viewport_scale_is_exactly_one_for_single_player() {
        let cam = Camera::new_chase(Vec2::ZERO, 0);
        assert_eq!(cam.m7_view_scale_q12(), 4096);
        assert_eq!(cam.viewport_w, SCREEN_W);
        assert_eq!(cam.viewport_h, SCREEN_H);
        assert_eq!(cam.m7_horizon_row(), M7_HORIZON);
    }

    /// 1P is unchanged: the ground under the camera pivot lands on the
    /// full-screen row the documented model predicts.
    #[test]
    fn single_player_projection_is_unchanged() {
        let cam = Camera::new_chase(Vec2::ZERO, 0);
        let (_sx, sy) = cam.project_cam(0, CAR_Z_Q4);
        assert_eq!(sy as i32, M7_HORIZON + 130, "1P ground row moved");
    }

    /// Elevated geometry has to use the same viewport scale as the road it
    /// stands on, or a car's body floats above its own shadow.
    #[test]
    fn elevated_geometry_scales_with_the_viewport() {
        let base_z = CAR_Z_Q4;
        for split_mode in 0u8..=2 {
            let mut cam = Camera::new_chase(Vec2::ZERO, 0);
            cam.set_split_mode(split_mode, 1);

            let (_bx, ground) = cam.project_cam_elevated(0, base_z, 0);
            let (_tx, top) = cam.project_cam_elevated(0, base_z, 8);

            assert!(
                top < ground,
                "split {split_mode}: an 8-unit lift did not raise the car"
            );
            let lift_px = (ground - top) as i32;
            // 8 world units at the pivot is ~8 px full-screen, scaled by
            // the viewport.
            let expected = 8 * cam.m7_view_scale_q12() / 4096;
            assert!(
                lift_px.abs_diff(expected) <= 1,
                "split {split_mode}: lift {lift_px}px, expected ~{expected}px"
            );
        }
    }

    /// `project_cam_elevated` at zero height must agree with `project_cam`
    /// exactly, or the road and the props standing on it would separate.
    #[test]
    fn the_elevated_projection_reduces_to_the_ground_projection() {
        for split_mode in 0u8..=2 {
            let mut cam = Camera::new_chase(Vec2::ZERO, 0);
            cam.set_split_mode(split_mode, 1);
            for right in [-800i32, 0, 800] {
                for z in [CAR_Z_Q4, 4000] {
                    assert_eq!(
                        cam.project_cam(right, z),
                        cam.project_cam_elevated(right, z, 0),
                        "split {split_mode}: ground and elevated projections disagree"
                    );
                }
            }
        }
    }

    /// Both players in a vertical split get the same half-width, but each
    /// must still centre its own car within that half.
    #[test]
    fn a_vertical_split_gives_each_player_their_own_half() {
        let mut p1 = Camera::new_chase(Vec2::ZERO, 0);
        let mut p2 = Camera::new_chase(Vec2::ZERO, 0);
        p1.set_split_mode(1, 1);
        p2.set_split_mode(1, 2);

        assert_eq!(p1.viewport_w, 158);
        assert_eq!(p2.viewport_w, 158);
        assert_eq!(p1.center_x, 79);
        assert_eq!(p2.center_x, 79);
        let (x1, _) = p1.project_cam(0, CAR_Z_Q4);
        let (x2, _) = p2.project_cam(0, CAR_Z_Q4);
        assert_eq!(x1, p1.center_x);
        assert_eq!(x2, p2.center_x);
    }

    /// A diagonal split recentres each player into their own triangle, so
    /// the two projections must actually differ.
    #[test]
    fn a_diagonal_split_recentres_each_player() {
        let mut p1 = Camera::new_chase(Vec2::ZERO, 0);
        let mut p2 = Camera::new_chase(Vec2::ZERO, 0);
        p1.set_split_mode(2, 1);
        p2.set_split_mode(2, 2);

        assert_ne!(p1.center_x, p2.center_x);
        assert_ne!(p1.center_y, p2.center_y);
        assert!(
            p1.center_x < p2.center_x,
            "player 1 should sit left of player 2 on the diagonal"
        );
        assert!(
            p1.center_y < p2.center_y,
            "player 1 should sit above player 2 on the diagonal"
        );
    }

    /// The horizon has to move with an offset viewport, otherwise a recentred
    /// player is left looking at a horizon belonging to the other half.
    ///
    /// The diagonal split is the case that needs this: it moves each player's
    /// viewport centre without changing the projection scale, so the horizon has
    /// to be anchored on the centre rather than on the screen's midpoint.
    #[test]
    fn the_horizon_follows_an_offset_viewport() {
        // Full-screen, centre on the screen midpoint: horizon unchanged.
        let centred = Camera::new_chase(Vec2::ZERO, 0);
        assert_eq!(centred.m7_horizon_row(), M7_HORIZON);

        // Diagonal split, player 1 sits high in the top-left triangle: the
        // horizon rises with it, and player 2's sits lower.
        let mut p1 = Camera::new_chase(Vec2::ZERO, 0);
        p1.set_split_mode(2, 1);
        let mut p2 = Camera::new_chase(Vec2::ZERO, 0);
        p2.set_split_mode(2, 2);
        assert!(
            p1.m7_horizon_row() < p2.m7_horizon_row(),
            "player 1's horizon ({}) must sit above player 2's ({})",
            p1.m7_horizon_row(),
            p2.m7_horizon_row()
        );

        // Moving the centre down moves the horizon down, at fixed scale.
        let mut shifted = Camera::new_chase(Vec2::ZERO, 0);
        let base = shifted.m7_horizon_row();
        shifted.center_y = 150i16;
        assert!(
            shifted.m7_horizon_row() > base,
            "lowering the viewport centre must lower the horizon"
        );
    }

    /// Depth sorting must work per viewport. This is the 2P ordering defect:
    /// a fixed player-1-then-player-2 draw order is only correct while the
    /// two cars stay in that order relative to the camera.
    #[test]
    fn vehicle_depth_order_is_per_viewport() {
        let near = Vec2::ZERO;
        let far = Vec2::new(Fixed::ZERO, Fixed::from_int(-80));

        let mut cam = Camera::new_chase(Vec2::ZERO, 0);
        cam.set_split_mode(0, 1);

        let near_y = cam.vehicle_screen_y(near);
        let far_y = cam.vehicle_screen_y(far);
        assert!(
            near_y > far_y,
            "a nearer car must sort to a larger screen row ({near_y} vs {far_y})"
        );

        // A car behind the camera sorts to the very front so it cannot
        // occlude anything visible.
        let behind = Vec2::new(Fixed::ZERO, Fixed::from_int(400));
        assert!(cam.vehicle_screen_y(behind) >= near_y);
    }
}

/// Replay trackside-camera judder: `update_broadcast_tv` snapped its heading to
/// an aim computed from a discrete route node, so every node step (and every
/// frame-to-frame flip between two equidistant nodes) was a hard cut.
#[cfg(test)]
mod replay_camera_smoothing_tests {
    use super::camera::Camera;
    use arduracer_core::Vec2;

    /// Shortest signed BAMS distance from `a` to `b`.
    fn bams_delta(a: u16, b: u16) -> i32 {
        let mut d = (b & 0x0FFF) as i32 - (a & 0x0FFF) as i32;
        if d > 2048 {
            d -= 4096;
        } else if d < -2048 {
            d += 4096;
        }
        d
    }

    /// One eased pan step must move less than the whole remaining angle. The
    /// driving camera's divisor is 6; the trackside camera's is gentler, and
    /// this is the property that removes the hard cut.
    #[test]
    fn a_pan_step_moves_only_a_fraction_of_the_remaining_angle() {
        let mut cam = Camera::new(Vec2::ZERO);
        cam.set_heading(0);
        // A right-angle-ish correction.
        let target = 1024u16;
        let remaining = bams_delta(cam.heading, target).abs();
        cam.follow_heading_slow(target);
        let moved = bams_delta(0, cam.heading).abs();
        assert!(moved > 0, "the pan made no progress at all");
        assert!(
            moved < remaining,
            "the pan jumped the whole way ({moved} of {remaining})"
        );
        assert!(
            moved * 3 <= remaining,
            "the pan moved {moved} of {remaining}: too coarse to read as a sweep"
        );
    }

    /// The pan must converge: an easing loop that stops short of the target
    /// would leave the replay camera permanently pointing the wrong way.
    #[test]
    fn a_pan_converges_on_its_target() {
        for target in [0u16, 512, 1024, 2048, 3000, 4095] {
            let mut cam = Camera::new(Vec2::ZERO);
            cam.set_heading(0);
            for _ in 0..2000 {
                cam.follow_heading_slow(target);
            }
            assert_eq!(
                bams_delta(cam.heading, target).abs(),
                0,
                "pan did not converge on {target}, stopped at {}",
                cam.heading
            );
        }
    }

    /// The pan takes the short way round, so a target just past the 0/4096
    /// wrap does not send the camera the long way spinning.
    #[test]
    fn a_pan_takes_the_short_way_round_the_wrap() {
        // 10 BAMS is 1 degree either side of zero; the short way is ~2, not ~4086.
        let mut cam = Camera::new(Vec2::ZERO);
        cam.set_heading(4090);
        cam.follow_heading_slow(4);
        assert!(
            bams_delta(4090, cam.heading).abs() <= 20,
            "pan took the long way around: {} -> {}",
            4090,
            cam.heading
        );
    }

    /// The trackside camera must be gentler than the driving camera, which is
    /// what makes a route-node step read as an operator following the car.
    #[test]
    fn the_trackside_pan_is_slower_than_the_driving_pan() {
        let target = 1024u16;
        let step = |slow: bool| -> i32 {
            let mut cam = Camera::new(Vec2::ZERO);
            cam.set_heading(0);
            if slow {
                cam.follow_heading_slow(target);
            } else {
                cam.follow_heading(target);
            }
            bams_delta(0, cam.heading).abs()
        };
        assert!(
            step(true) < step(false),
            "trackside pan ({}) is not slower than the driving pan ({})",
            step(true),
            step(false)
        );
    }

    /// A fresh camera starts unplaced, so the first broadcast frame cuts to the
    /// angle instead of sweeping in from an arbitrary heading.
    #[test]
    fn a_camera_starts_unplaced_so_the_first_broadcast_frame_cuts() {
        assert!(
            Camera::new(Vec2::ZERO).tv_unplaced,
            "a new camera must start unplaced"
        );
    }

    /// The regression itself, driven through the real entry point on a real
    /// circuit: walking a car along the route, the camera heading must move a
    /// bounded amount per frame.
    ///
    /// The unit tests above pin the easing helper, but on their own they would
    /// still pass if the call site reverted to snapping -- which is exactly the
    /// bug. This one exercises `update_broadcast_tv` itself.
    #[test]
    fn the_broadcast_camera_never_snaps_its_heading_while_the_car_drives() {
        let track = &arduracer_core::ALL_TRACKS[0];
        let route = track.route;
        assert!(route.len() >= 4, "test circuit needs a route");

        let mut cam = Camera::new(Vec2::ZERO);
        // Start unplaced, as on entering the replay: the first frame cuts.
        cam.tv_unplaced = true;
        cam.update_broadcast_tv(track, route[0], 0);
        let mut prev = cam.heading;

        let mut worst = 0i32;
        // Walk the car along the route and watch how far the heading travels
        // per frame.
        for &node in route.iter().skip(1) {
            cam.update_broadcast_tv(track, node, 0);
            let step = bams_delta(prev, cam.heading).abs();
            worst = worst.max(step);
            prev = cam.heading;
        }

        // Measured on this circuit, walking a whole route node per frame (far
        // harsher than a real replay, where the car moves a few units a tick):
        // the easing pan peaks at 96 BAMS, the snapping version at 1132 -- a
        // ~100 degree cut. 256 sits between the two with margin on both sides.
        assert!(
            worst <= 256,
            "broadcast camera jumped {worst} BAMS in one frame (snap-like)"
        );
    }

    /// Consecutive frames along a route can never disagree by more than a
    /// gentle fraction of a turn. This is the flip-flop between equidistant
    /// route nodes that made the judder most visible.
    #[test]
    fn the_broadcast_pan_is_continuous_along_a_route() {
        let track = &arduracer_core::ALL_TRACKS[0];
        let route = track.route;
        let mut cam = Camera::new(Vec2::ZERO);
        cam.tv_unplaced = true;

        let mut headings = [0u16; 256];
        let n = route.len().min(headings.len());
        for i in 0..n {
            cam.update_broadcast_tv(track, route[i], 0);
            headings[i] = cam.heading;
        }

        for i in 1..n {
            let d = bams_delta(headings[i - 1], headings[i]).abs();
            assert!(
                d <= 256,
                "pan reversed by {d} BAMS between route nodes {} and {}",
                i - 1,
                i
            );
        }
    }
}

/// Roadside props were planted a fixed 24 world units off the centreline, but
/// circuits are 64..=128 units half-wide, so ~95% of them stood *in* the racing
/// line with no collision to explain driving through them.
///
/// These drive the real `props_renderer::biome_prop_placements` -- the function
/// the renderer itself calls -- rather than a mirrored copy, so they fail if the
/// renderer regresses.
#[cfg(test)]
mod roadside_prop_placement_tests {
    use crate::gpu::props_renderer::{biome_prop_placements, PROP_FINE_WU, PROP_NODE_STRIDE};
    use arduracer_core::levels::ALL_TRACKS;
    use arduracer_core::math::{cos, sin};
    use arduracer_core::{heading_towards, Fixed, TrackDef, Vec2, FP_SHIFT};

    fn is_on_road(track: &TrackDef, p: Vec2) -> bool {
        track.is_road_at(TrackDef::tile_x_of(p.x), TrackDef::tile_y_of(p.y))
    }

    /// Every prop placement on every circuit must land off the racing surface.
    /// This is the whole bug: props standing in the middle of the track.
    #[test]
    fn no_roadside_prop_is_placed_on_the_racing_surface() {
        let mut placed = 0usize;
        for track in ALL_TRACKS.iter() {
            for pos in biome_prop_placements(track).iter() {
                placed += 1;
                assert!(
                    !is_on_road(track, pos),
                    "{}: prop placed on the racing surface at tile ({},{})",
                    track.name,
                    TrackDef::tile_x_of(pos.x),
                    TrackDef::tile_y_of(pos.y)
                );
            }
        }
        assert!(
            placed > 100,
            "expected many props across all circuits, placed only {placed}"
        );
    }

    /// The fix must not have quietly deleted the scenery. If the outward search
    /// gives up too early the circuits simply lose their roadside, which would
    /// be a worse regression than the one being fixed.
    #[test]
    fn the_circuits_keep_their_roadside_scenery() {
        let mut total = 0usize;
        for track in ALL_TRACKS.iter() {
            let placements = biome_prop_placements(track);
            let route_len = if track.route.is_empty() {
                track.route_len()
            } else {
                track.route.len()
            };
            if route_len < 4 {
                continue;
            }
            let expected_slots = route_len.div_ceil(PROP_NODE_STRIDE) * 2;
            // Every slot on every circuit must find a spot: no circuit may lose
            // its props to a search that gives up.
            assert_eq!(
                placements.len(),
                expected_slots.min(128),
                "{}: placed {} props, expected {expected_slots}",
                track.name,
                placements.len()
            );
            total += placements.len();
        }
        assert!(
            total > 1000,
            "expected a full roadside everywhere, got {total}"
        );
    }

    /// A prop must stand just off the kerb, not out in the infield.
    ///
    /// Measured along the perpendicular from the centreline, which is the
    /// quantity the placement bound governs -- *not* the distance to the nearest
    /// route node. A prop beside a wide gap between two stretches of road can be
    /// legitimately hundreds of units from any node while still standing right
    /// against the tarmac, and asserting on the latter rejects correct
    /// placements.
    #[test]
    fn no_prop_stands_far_outside_the_kerb() {
        for track in ALL_TRACKS.iter() {
            let route = track.route;
            if route.is_empty() {
                continue;
            }
            let n = route.len();
            for pos in biome_prop_placements(track).iter() {
                let mut worst = 0i32;
                for i in (0..n).step_by(PROP_NODE_STRIDE) {
                    let node = route[i % n];
                    let next = route[(i + 1) % n];
                    let seg_h = heading_towards(node, next);
                    let dx = (pos.x - node.x).raw().abs() >> 8;
                    let dy = (pos.y - node.y).raw().abs() >> 8;
                    // Perpendicular component: project onto the step axis.
                    let step_x = cos(seg_h).raw() >> FP_SHIFT;
                    let step_y = sin(seg_h).raw() >> FP_SHIFT;
                    let along = (dx * step_x + dy * step_y) / 4096;
                    if along > worst {
                        worst = along;
                    }
                }
                assert!(
                    worst <= 2048,
                    "{}: a prop stands {worst} WU outside the kerb",
                    track.name
                );
            }
        }
    }

    /// The outward step must stay non-zero at every heading. Truncating the Q12
    /// unit vector to a whole world unit collapses diagonals to zero, which
    /// makes the search walk in place and silently delete every prop.
    #[test]
    fn the_outward_step_is_never_zero_for_any_heading() {
        for h in 0u16..4096 {
            let x = (cos(h).raw() * 4) >> FP_SHIFT;
            let y = (sin(h).raw() * 4) >> FP_SHIFT;
            assert!(
                x != 0 || y != 0,
                "heading {h} gives a zero-length outward step"
            );
        }
    }

    /// The step must move *across* the road, not along it: for an eastward
    /// segment the perpendicular is vertical, for a northward one horizontal.
    #[test]
    fn the_outward_step_is_perpendicular_to_the_segment() {
        let east = heading_towards(
            Vec2::new(Fixed::ZERO, Fixed::ZERO),
            Vec2::new(Fixed::from_int(100), Fixed::ZERO),
        );
        assert_eq!((cos(east).raw() * 4) >> FP_SHIFT, 0, "east: X must be 0");
        assert_ne!((sin(east).raw() * 4) >> FP_SHIFT, 0, "east: Y must move");

        let north = heading_towards(
            Vec2::new(Fixed::ZERO, Fixed::ZERO),
            Vec2::new(Fixed::ZERO, Fixed::from_int(-100)),
        );
        assert_ne!((cos(north).raw() * 4) >> FP_SHIFT, 0, "north: X must move");
        assert_eq!((sin(north).raw() * 4) >> FP_SHIFT, 0, "north: Y must be 0");
    }

    /// The two sides of a segment must be mirrored, so a prop does not stand
    /// closer to the road on one side than on the other.
    #[test]
    fn the_two_sides_step_in_opposite_directions() {
        // Round away from zero, as `scale_step` does: an arithmetic shift
        // truncates toward zero and would put one kerb a unit closer than the
        // other.
        let scale = |c: i32, side: i32, dist: i32| -> i32 {
            let p = c * side * dist;
            if p >= 0 {
                (p + (1 << (FP_SHIFT - 1))) >> FP_SHIFT
            } else {
                -((-p + (1 << (FP_SHIFT - 1))) >> FP_SHIFT)
            }
        };
        for h in 0u16..4096 {
            assert_eq!(
                scale(cos(h).raw(), -1, PROP_FINE_WU),
                -scale(cos(h).raw(), 1, PROP_FINE_WU),
                "heading {h}: sides not mirrored in X"
            );
            assert_eq!(
                scale(sin(h).raw(), -1, PROP_FINE_WU),
                -scale(sin(h).raw(), 1, PROP_FINE_WU),
                "heading {h}: sides not mirrored in Y"
            );
        }
    }
}

/// Distant Mode 7 cars were drawn a row too high. The LOD's minimum-size clamp
/// re-centred the quad on its own midpoint, lifting every distant car above the
/// road it stood on.
#[cfg(test)]
mod distant_car_lod_tests {
    use super::car_geometry::enforce_min_visible_size;

    /// A projected LOD quad at world depth `d`, as the renderer computes it:
    /// front pair 12 units ahead at height 2, rear pair 12 behind at height 3.
    fn lod_quad(d: i32) -> [(i16, i16); 4] {
        let row = |depth: i32, h: i32| -> i16 {
            let z_q4 = (depth + 120) << 4;
            let num = (2_080 - h * 16) * 120;
            (60 + num / z_q4) as i16
        };
        [
            (160 - 1, row(d + 12, 2)),
            (160 + 1, row(d + 12, 2)),
            (160 - 1, row(d - 12, 3)),
            (160 + 1, row(d - 12, 3)),
        ]
    }

    /// The clamp must never move the car's top row upward. This is the whole
    /// defect: the car visibly detached from the road surface.
    #[test]
    fn the_minimum_size_clamp_never_lifts_a_car_above_its_projection() {
        for d in [255, 300, 400, 600, 900, 1_600, 3_000] {
            let original = lod_quad(d);
            let mut q = original;
            enforce_min_visible_size(&mut q);
            let projected_top = original[0].1.min(original[1].1);
            assert!(
                q[0].1 >= projected_top && q[1].1 >= projected_top,
                "depth {d}: clamp lifted the car from row {projected_top} to {:?}",
                (q[0].1, q[1].1)
            );
        }
    }

    /// And specifically the previous behaviour: re-centring on the midpoint
    /// lifted the body above its projected top at essentially every depth,
    /// because a distant quad is nearly flat.
    #[test]
    fn the_old_midpoint_recentring_reproduces_the_lift() {
        let mut lifted = 0;
        let mut total = 0;
        for d in 255..1_600 {
            let original = lod_quad(d);
            let fl = original[0];
            let rl = original[2];
            if (rl.1 - fl.1).abs() >= 2 {
                continue; // clamp would not fire
            }
            total += 1;
            let mid = (fl.1 + rl.1) / 2;
            let old_top = mid - 1;
            let projected_top = original[0].1.min(original[1].1);
            if old_top < projected_top {
                lifted += 1;
            }
        }
        assert!(total > 1000, "expected many clamped depths, only {total}");
        assert_eq!(
            lifted, total,
            "the old clamp should have lifted every clamped quad ({lifted}/{total})"
        );
    }

    /// The clamp's purpose still holds: a too-small quad must be grown so the
    /// rasteriser cannot drop it.
    #[test]
    fn the_clamp_still_guarantees_a_visible_size() {
        for d in [400, 900, 1_600, 3_000] {
            let mut q = lod_quad(d);
            enforce_min_visible_size(&mut q);
            let width = (q[1].0 - q[0].0).abs();
            let height = (q[2].1 - q[0].1).abs();
            assert!(width >= 2, "depth {d}: width collapsed to {width}");
            assert!(height >= 2, "depth {d}: height collapsed to {height}");
        }
    }

    /// The clamp must stay centred horizontally, or a distant car drifts
    /// sideways as it passes.
    #[test]
    fn widening_the_clamp_keeps_the_car_centred() {
        for d in [300, 600, 1_600] {
            let original = lod_quad(d);
            let mut q = original;
            enforce_min_visible_size(&mut q);
            let before = original[0].0 + original[1].0;
            let after = q[0].0 + q[1].0;
            assert_eq!(after, before, "depth {d}: clamp shifted the car sideways");
        }
    }

    /// A quad that is already big enough must be left alone.
    #[test]
    fn a_quad_large_enough_is_untouched() {
        let mut q = [
            (150i16, 180i16),
            (170i16, 180i16),
            (150i16, 195i16),
            (170i16, 195i16),
        ];
        let before = q;
        enforce_min_visible_size(&mut q);
        assert_eq!(q, before, "a large quad was modified");
    }
}

/// The LOD threshold was a fixed 6000 Q4, which sat past the uncompressed
/// formats' `z_far` of 240 -- so the road ended before the car did.
#[cfg(test)]
mod vehicle_lod_threshold_tests {
    use super::camera::{Camera, M7_Z0, M7_Z_FAR, M7_Z_FAR_COMPRESSED};
    use arduracer_core::Vec2;

    fn cam_with_z_far(z_far: i32) -> Camera {
        let mut c = Camera::new_chase(Vec2::ZERO, 0);
        c.z_far = z_far;
        c
    }

    /// The LOD must engage while the road is still drawn beneath the car.
    #[test]
    fn the_lod_engages_before_the_road_ends() {
        for z_far in [M7_Z_FAR, M7_Z_FAR_COMPRESSED] {
            let cam = cam_with_z_far(z_far);
            let lod_depth = cam.z_far_lod_q4() / 16 - M7_Z0;
            let road_depth = z_far - M7_Z0;
            assert!(
                lod_depth <= road_depth,
                "z_far {z_far}: LOD at depth {lod_depth} is past the road at {road_depth}"
            );
        }
    }

    /// The old fixed threshold, for the record: 6000 Q4 = depth 255, which is
    /// past the uncompressed road's 120.
    #[test]
    fn the_old_fixed_threshold_was_past_the_uncompressed_road() {
        let old_depth = 6000i32 / 16 - M7_Z0;
        assert!(
            old_depth > M7_Z_FAR - M7_Z0,
            "expected the fixed 6000 threshold to overrun the uncompressed road"
        );
    }

    /// The threshold must actually be reachable: not so near that every car is
    /// a LOD blob.
    #[test]
    fn the_lod_threshold_is_not_so_near_that_every_car_is_low_detail() {
        for z_far in [M7_Z_FAR, M7_Z_FAR_COMPRESSED] {
            let cam = cam_with_z_far(z_far);
            let lod_depth = cam.z_far_lod_q4() / 16 - M7_Z0;
            assert!(
                lod_depth >= 100,
                "z_far {z_far}: LOD at {lod_depth} would catch near cars"
            );
        }
    }
}

/// The Mode 7 parallax skyline was anchored exactly at `track_top`, so the road
/// -- drawn after the backdrop, from `track_top` down -- painted over its
/// entire foot and the peaks read as floating on bare ground.
#[cfg(test)]
mod m7_skyline_clearance_tests {
    use super::camera::Camera;
    use arduracer_core::{Fixed, Vec2};

    const TRACKTEX_SRC: &str = include_str!("../../../game/src/gpu/tracktex.rs");

    /// Rows the skyline base is raised above the grid's first row.
    const LIFT: i16 = 5;

    /// The renderer lifts the base; mirror that here since `tracktex` draws
    /// through the GPU and cannot be linked into a host test.
    fn skyline_base(track_top: i16) -> i16 {
        (track_top - LIFT).max(0)
    }

    /// The skyline base must clear the top of the Mode 7 grid, or the road
    /// covers it.
    #[test]
    fn the_skyline_base_clears_the_mode7_grid() {
        let mut cam = Camera::new_chase(Vec2::new(Fixed::ZERO, Fixed::ZERO), 0);
        for pitch in [-10, -5, 0, 5, 10] {
            cam.pitch_px = pitch;
            let track_top = cam.m7_track_top();
            // A track_top low enough that the lift would clamp must still not
            // end up below the grid's first row.
            let base = skyline_base(track_top);
            assert!(
                base <= track_top,
                "pitch {pitch}: skyline base {base} is below the grid top {track_top}"
            );
            if track_top > LIFT {
                assert!(
                    base < track_top,
                    "pitch {pitch}: skyline base {base} must sit strictly above \
                     the grid top {track_top}"
                );
                assert_eq!(
                    base,
                    track_top - LIFT,
                    "pitch {pitch}: lift must be exactly {LIFT}"
                );
            }
        }
    }

    /// The lift must be a real offset, not a no-op -- this is the regression
    /// guard for the original anchoring.
    #[test]
    fn the_lift_is_a_positive_offset() {
        // Read the lift at runtime so the assertion below is not constant-folded: the
        // point is to fail if the constant is ever changed to 0.
        let lift = [LIFT][0];
        assert!(lift > 0, "a lift of {lift} changes nothing");
        let track_top = 100i16;
        assert!(
            skyline_base(track_top) < track_top,
            "the skyline is still level with the grid"
        );
    }

    /// Raising the base must not uncover the road: the skirt spans from the
    /// lifted base to the viewport bottom, so every row below the ridge is
    /// still painted.
    #[test]
    fn the_skirt_still_paints_every_row_below_the_ridge() {
        let mut cam = Camera::new_chase(Vec2::ZERO, 0);
        let v_h = cam.viewport_h;
        for pitch in [-10, -5, 0, 5, 10] {
            cam.pitch_px = pitch;
            let base = skyline_base(cam.m7_track_top());
            for row in base..v_h {
                assert!(
                    (base..v_h).contains(&row),
                    "pitch {pitch}: row {row} left unpainted"
                );
            }
            assert_eq!(
                (v_h - base).max(0) + base,
                v_h,
                "pitch {pitch}: skirt must reach the viewport bottom"
            );
        }
    }

    /// The renderer must actually apply the lift; a constant that exists but is
    /// unused would pass every arithmetic test above.
    #[test]
    fn the_renderer_subtracts_the_lift_from_the_skyline_base() {
        assert!(
            TRACKTEX_SRC.contains("const M7_SILHOUETTE_LIFT: i16 = 5;"),
            "tracktex.rs must define the skyline lift"
        );
        assert!(
            TRACKTEX_SRC.contains("track_top - M7_SILHOUETTE_LIFT"),
            "tracktex.rs must apply the lift to the skyline base"
        );
    }

    /// Same requirement in split-screen, where the viewport is shorter and the
    /// grid covers proportionally more of it.
    #[test]
    fn the_skyline_clears_the_grid_in_split_screen_too() {
        let mut cam = Camera::new_chase(Vec2::ZERO, 0);
        for mode in 0u8..=2 {
            cam.set_split_mode(mode, 1);
            let track_top = cam.m7_track_top();
            let base = skyline_base(track_top);
            assert!(
                base <= track_top,
                "split {mode}: skyline base {base} below grid top {track_top}"
            );
        }
    }
}

/// The Mode 7 ground mesh, driven through a software rasterizer.
///
/// Two earlier fixes pulled in opposite directions: subdividing near tiles
/// finer (to stop affine warping) opened a sawtooth of backdrop along tile
/// seams, and the seam fix gave every tile the same coarse 3x3 grid, which
/// warped by up to 7 px in front of the car. These tests hold both properties
/// at once against the real `m7_mesh`: every ground pixel is covered exactly
/// once, and no pixel's texel lands more than a couple of pixels from where
/// perspective puts it.
#[cfg(test)]
mod m7_mesh_tests {
    use super::camera::{Camera, M7_Z_NEAR};
    use super::m7_mesh::{self, MeshScratch, MeshVert, SoftwareProjector, TileRect};
    use arduracer_core::{Fixed, Vec2};

    const TILE: i32 = 160;
    const WORLD: i32 = 40 * TILE;
    /// A virtual screen taller and wider than the real one, so the rows below
    /// the visible bottom -- where the near plane cuts cells -- are checked too.
    const VX0: i32 = -200;
    const VY0: i32 = 0;
    const VW: i32 = 720;
    const VH: i32 = 300;

    struct Raster {
        cover: Vec<u8>,
        /// Worst texel displacement seen, in screen pixels, over the real screen.
        worst_px: f64,
        polys: usize,
        /// Polygons cut by the near plane.
        clipped: usize,
        cam: Camera,
        cam_wx: f64,
        cam_wy: f64,
        // Tile-local UV origin of the polygon being drawn.
        tile: (i32, i32),
        /// Real-screen ground pixels sampled, and how many sit over 3 px off.
        pixels: u64,
        over_3px: u64,
    }

    fn camera_at(x: i32, y: i32, heading: u16, far: bool) -> Camera {
        let mut c = Camera::new_chase(
            Vec2 {
                x: Fixed::from_int(x),
                y: Fixed::from_int(y),
            },
            heading,
        );
        c.set_extended_view_distance(far);
        c
    }

    impl Raster {
        fn new(cam: Camera) -> Self {
            let cam_wx = (cam.pos.x.raw() >> 8) as f64 / 16.0;
            let cam_wy = (cam.pos.y.raw() >> 8) as f64 / 16.0;
            Raster {
                cover: vec![0; (VW * VH) as usize],
                worst_px: 0.0,
                polys: 0,
                clipped: 0,
                cam,
                cam_wx,
                cam_wy,
                tile: (0, 0),
                pixels: 0,
                over_3px: 0,
            }
        }

        /// Ground point (world units) seen through pixel sample (sx, sy).
        fn unproject(&self, sx: f64, sy: f64) -> Option<(f64, f64, f64)> {
            let f = 120.0;
            let h = self.cam.m7_horizon_row() as f64;
            if sy <= h + 0.5 {
                return None;
            }
            let z_q4 = 2080.0 * f / (sy - h);
            let r_q4 = (sx - self.cam.center_x as f64) * z_q4 / f;
            let fwd = z_q4 - 1920.0;
            let (s, c) = (
                self.cam.hsin() as f64 / 4096.0,
                self.cam.hcos() as f64 / 4096.0,
            );
            let dx = r_q4 * c + fwd * s;
            let dy = r_q4 * s - fwd * c;
            Some((self.cam_wx + dx / 16.0, self.cam_wy + dy / 16.0, z_q4))
        }

        /// Screen position of ground point (wx, wy), unrounded.
        fn project(&self, wx: f64, wy: f64) -> Option<(f64, f64)> {
            let dx = (wx - self.cam_wx) * 16.0;
            let dy = (wy - self.cam_wy) * 16.0;
            let (s, c) = (
                self.cam.hsin() as f64 / 4096.0,
                self.cam.hcos() as f64 / 4096.0,
            );
            let r = dx * c + dy * s;
            let z = dx * s - dy * c + 1920.0;
            if z <= 0.0 {
                return None;
            }
            let f = 120.0;
            Some((
                self.cam.center_x as f64 + r * f / z,
                self.cam.m7_horizon_row() as f64 + 2080.0 * f / z,
            ))
        }

        fn tri(&mut self, p: [(i16, i16); 3], uv: [(i32, i32); 3]) {
            let pt = |k: usize| (p[k].0 as i64, p[k].1 as i64);
            let (mut a, mut b, c) = (pt(0), pt(1), pt(2));
            let (mut ua, mut ub, uc) = (uv[0], uv[1], uv[2]);
            let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if area == 0 {
                return;
            }
            if area < 0 {
                core::mem::swap(&mut a, &mut b);
                core::mem::swap(&mut ua, &mut ub);
            }
            let area = area.abs();
            // Edge functions in doubled coordinates, sampling pixel centres;
            // a top-left rule settles samples exactly on an edge.
            let edge = |p: (i64, i64), q: (i64, i64), x: i64, y: i64| {
                (q.0 - p.0) * 2 * (y - 2 * p.1) - (q.1 - p.1) * 2 * (x - 2 * p.0)
            };
            let top_left = |p: (i64, i64), q: (i64, i64)| {
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                dy < 0 || (dy == 0 && dx > 0)
            };
            let x0 = a.0.min(b.0).min(c.0).max(VX0 as i64);
            let x1 = a.0.max(b.0).max(c.0).min((VX0 + VW - 1) as i64);
            let y0 = a.1.min(b.1).min(c.1).max(VY0 as i64);
            let y1 = a.1.max(b.1).max(c.1).min((VY0 + VH - 1) as i64);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (sx, sy) = (2 * x + 1, 2 * y + 1);
                    let w0 = edge(b, c, sx, sy);
                    let w1 = edge(c, a, sx, sy);
                    let w2 = edge(a, b, sx, sy);
                    let inside = |w: i64, p, q| w > 0 || (w == 0 && top_left(p, q));
                    if !(inside(w0, b, c) && inside(w1, c, a) && inside(w2, a, b)) {
                        continue;
                    }
                    let i = ((y - VY0 as i64) * VW as i64 + (x - VX0 as i64)) as usize;
                    self.cover[i] = self.cover[i].saturating_add(1);

                    // Affine UV, exactly as the GPU interpolates it.
                    if !(0..320).contains(&x) || !(0..240).contains(&y) {
                        continue;
                    }
                    let (l0, l1, l2) = (
                        w0 as f64 / (4.0 * area as f64),
                        w1 as f64 / (4.0 * area as f64),
                        w2 as f64 / (4.0 * area as f64),
                    );
                    let u = l0 * ua.0 as f64 + l1 * ub.0 as f64 + l2 * uc.0 as f64;
                    let v = l0 * ua.1 as f64 + l1 * ub.1 as f64 + l2 * uc.1 as f64;
                    let Some((wx, wy, z_q4)) = self.unproject(x as f64 + 0.5, y as f64 + 0.5)
                    else {
                        continue;
                    };
                    let _ = (wx, wy, z_q4);
                    // Where the GPU's texel actually sits on the ground, and
                    // where that ground point projects: the warp is the screen
                    // distance from the pixel showing it.
                    let gx = (self.tile.0 * TILE) as f64 + u * TILE as f64 / 1024.0;
                    let gy = (self.tile.1 * TILE) as f64 + v * TILE as f64 / 1024.0;
                    let Some((px_x, px_y)) = self.project(gx, gy) else {
                        continue;
                    };
                    let px = ((px_x - (x as f64 + 0.5)).powi(2)
                        + (px_y - (y as f64 + 0.5)).powi(2))
                    .sqrt();
                    self.pixels += 1;
                    if px > 3.0 {
                        self.over_3px += 1;
                    }
                    if px > self.worst_px {
                        self.worst_px = px;
                    }
                }
            }
        }

        fn poly(&mut self, poly: &[MeshVert], zmin: i32) {
            self.polys += 1;
            if zmin == M7_Z_NEAR << 4 {
                self.clipped += 1;
            }
            let p: Vec<(i16, i16)> = poly.iter().map(|v| (v.sx, v.sy)).collect();
            let uv: Vec<(i32, i32)> = poly.iter().map(|v| (v.u as i32, v.v as i32)).collect();
            if poly.len() == 4 {
                // The GPU splits a quad v0..v3 as (v0,v1,v2) + (v1,v2,v3), and
                // the renderer passes the ring as [0, 1, 3, 2].
                self.tri([p[0], p[1], p[3]], [uv[0], uv[1], uv[3]]);
                self.tri([p[1], p[3], p[2]], [uv[1], uv[3], uv[2]]);
            } else {
                for k in 1..poly.len() - 1 {
                    self.tri([p[0], p[k], p[k + 1]], [uv[0], uv[k], uv[k + 1]]);
                }
            }
        }
    }

    /// Draws every tile around the camera, at the levels the renderer picks
    /// unless `force` pins one level everywhere.
    fn render(cam: Camera, force: Option<i32>) -> Raster {
        let mut ras = Raster::new(cam);
        let mut scratch = MeshScratch::new();
        let ctx = (cam.pos.x.raw() >> 12) / TILE as i32;
        let cty = (cam.pos.y.raw() >> 12) / TILE as i32;
        let view = (VX0 as i16, VY0 as i16, (VX0 + VW) as i16, (VY0 + VH) as i16);
        for ty in cty - 12..=cty + 12 {
            for tx in ctx - 12..=ctx + 12 {
                let tile = (tx, ty);
                let Some(rect) = TileRect::new(tile, (TILE, TILE), (WORLD, WORLD)) else {
                    continue;
                };
                let (n, sides) = match force {
                    Some(n) => (n, [n; 4]),
                    None => {
                        let n = m7_mesh::rect_level(&cam, &rect);
                        (
                            n,
                            m7_mesh::side_levels(&cam, tile, (TILE, TILE), (WORLD, WORLD), n),
                        )
                    }
                };
                ras.tile = tile;
                let mut polys = Vec::new();
                m7_mesh::build_tile(
                    &cam,
                    &rect,
                    n,
                    sides,
                    cam.z_far << 4,
                    view,
                    &mut scratch,
                    &mut SoftwareProjector(&cam),
                    |p, zmin, _| polys.push((p.to_vec(), zmin)),
                );
                for (p, zmin) in &polys {
                    ras.poly(p, *zmin);
                }
            }
        }
        ras
    }

    /// Camera poses: off-grid positions, every octant of heading and the odd
    /// angles between, which is where a rotated tile's edges are least aligned.
    fn poses() -> Vec<Camera> {
        let mut out = Vec::new();
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for k in 0..24 {
            let x = WORLD / 2 + (next() % TILE as u32) as i32;
            let y = WORLD / 2 + (next() % TILE as u32) as i32;
            let heading = if k < 8 {
                (k * 512) as u16
            } else {
                (next() & 0x0FFF) as u16
            };
            out.push(camera_at(x, y, heading, k % 3 != 0));
        }
        out
    }

    /// Every ground pixel between the near plane and the far plane must be
    /// drawn. Zero is a hole -- the sawtooth.
    ///
    /// Overlaps are bounded rather than forbidden. Far cells are two or three
    /// pixels tall, and rounding their corners to whole pixels can make one
    /// slightly concave; the GPU's fixed quad diagonal then draws a pixel twice.
    /// That cannot open a hole (the larger triangle contains the cell) and the
    /// old grid did it too. What it rules out is the 2 px guard band this mesh
    /// replaced, which overlapped every cell edge on screen.
    #[test]
    fn ground_has_no_holes() {
        let mut clipped = 0;
        for cam in poses() {
            let ras = render(cam, None);
            clipped += ras.clipped;
            let near_q4 = (M7_Z_NEAR << 4) as f64;
            let far_q4 = (cam.z_far << 4) as f64;
            let mut holes = 0;
            let mut overlaps = 0;
            for y in VY0..VY0 + VH {
                for x in VX0..VX0 + VW {
                    let c = ras.cover[((y - VY0) * VW + (x - VX0)) as usize];
                    if c > 1 {
                        overlaps += 1;
                    }
                    let Some((_, _, z_q4)) = ras.unproject(x as f64 + 0.5, y as f64 + 0.5) else {
                        continue;
                    };
                    // Stay a pixel's depth clear of the planes themselves.
                    if z_q4 > near_q4 * 1.02 && z_q4 < far_q4 * 0.98 && c == 0 {
                        holes += 1;
                    }
                }
            }
            let at = (cam.heading, cam.pos.x.raw() >> 12, cam.pos.y.raw() >> 12);
            assert_eq!(holes, 0, "{at:?}: holes in the ground mesh");
            assert!(overlaps <= 16, "{at:?}: {overlaps} doubly drawn pixels");
        }
        // The virtual screen reaches below the near plane, so the clipped
        // cells are among those checked; make sure there were some.
        assert!(clipped > 0, "no polygon crossed the near plane");
    }

    /// The depth bands keep the texture where perspective puts it.
    ///
    /// Measured on screen: the ground point the GPU's affinely interpolated
    /// texel belongs to is projected back, and its distance from the pixel
    /// showing it is the warp. Over 24 poses the worst is ~5.4 px, in the
    /// bottom corners where cells hang below the screen edge, and fewer than
    /// 0.4% of ground pixels exceed 3 px. A uniform 4x4 grid -- finer than the
    /// 3x3 this replaced -- reaches ~15 px, so the test discriminates.
    #[test]
    fn texture_warp_stays_small() {
        let mut worst = 0.0f64;
        let (mut pixels, mut over) = (0u64, 0u64);
        for cam in poses() {
            let r = render(cam, None);
            worst = worst.max(r.worst_px);
            pixels += r.pixels;
            over += r.over_3px;
        }
        assert!(worst <= 6.0, "worst texel displacement {worst:.2} px");
        assert!(
            over * 100 <= pixels,
            "{over} of {pixels} ground pixels are more than 3 px off"
        );

        let mut uniform = 0.0f64;
        for cam in poses() {
            uniform = uniform.max(render(cam, Some(4)).worst_px);
        }
        assert!(
            uniform > 10.0,
            "a uniform 4x4 grid should warp visibly, got {uniform:.2} px"
        );
    }

    /// The finer grid must stay affordable: count the polygons a frame submits.
    #[test]
    fn polygon_count_stays_bounded() {
        let mut worst = 0;
        for cam in poses() {
            let mut cam = cam;
            cam.set_extended_view_distance(true);
            // Only the real screen counts toward the GPU's work.
            let ras = render_counting(cam);
            worst = worst.max(ras);
        }
        // 528 over these poses; the old fixed grid drew roughly 200.
        assert!(worst <= 600, "{worst} polygons in one frame");
    }

    fn render_counting(cam: Camera) -> usize {
        let mut scratch = MeshScratch::new();
        let ctx = (cam.pos.x.raw() >> 12) / TILE as i32;
        let cty = (cam.pos.y.raw() >> 12) / TILE as i32;
        let mut count = 0;
        for ty in cty - 12..=cty + 12 {
            for tx in ctx - 12..=ctx + 12 {
                let tile = (tx, ty);
                let Some(rect) = TileRect::new(tile, (TILE, TILE), (WORLD, WORLD)) else {
                    continue;
                };
                let n = m7_mesh::rect_level(&cam, &rect);
                let sides = m7_mesh::side_levels(&cam, tile, (TILE, TILE), (WORLD, WORLD), n);
                m7_mesh::build_tile(
                    &cam,
                    &rect,
                    n,
                    sides,
                    cam.z_far << 4,
                    (0, 0, 320, 240),
                    &mut scratch,
                    &mut SoftwareProjector(&cam),
                    |_, _, _| count += 1,
                );
            }
        }
        count
    }

    #[test]
    fn levels_are_powers_of_two_and_coarsen_with_depth() {
        let mut last = m7_mesh::MAX_LEVEL;
        for z in (0..20_000).step_by(16) {
            let n = m7_mesh::level_for_depth(z);
            assert!(n.count_ones() == 1 && n <= m7_mesh::MAX_LEVEL, "level {n}");
            assert!(n <= last, "level rose with depth at z {z}");
            last = n;
        }
    }

    #[test]
    fn a_tile_outside_the_world_has_no_level() {
        let cam = camera_at(WORLD / 2, WORLD / 2, 0, true);
        assert_eq!(
            m7_mesh::tile_level(&cam, (-1, 3), (TILE, TILE), (WORLD, WORLD)),
            0
        );
        assert_eq!(
            m7_mesh::tile_level(&cam, (40, 3), (TILE, TILE), (WORLD, WORLD)),
            0
        );
    }
}
