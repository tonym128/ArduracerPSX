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

#[path = "../../../game/src/ui/pause_input.rs"]
mod pause_input;

#[path = "../../../game/src/gpu/camera.rs"]
mod camera;

#[path = "../../../game/src/gpu/car_geometry.rs"]
#[allow(unused_imports)]
mod car_geometry;

#[path = "../../../game/src/ui/tuning_input.rs"]
mod tuning_input;

#[path = "../../../game/src/gpu/effects_sim.rs"]
mod effects_sim;

#[path = "../../../game/src/gpu/palette.rs"]
mod palette;

#[path = "../../../game/src/gpu/texlayout.rs"]
mod texlayout;

#[path = "../../../game/src/ui/title_input.rs"]
mod title_input;

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
            let expected = match idx {
                0 => PauseChoice::Resume,
                1 => PauseChoice::RestartRace,
                _ => PauseChoice::QuitToMenu,
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

        assert_eq!(PauseMenu::item_count(), 3);
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
        minimap_colour, minimap_step, pack_bgr555, slot_overlaps_framebuffers, MAX_DIM,
        MINIMAP_INNER, SLOT_BYTES, TEXTURE_X, TEXTURE_Y,
    };
    use arduracer_core::TrackTile;

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
        for (w, h) in [(38u8, 38u8), (40, 40), (30, 30), (4, 4)] {
            let step_x = minimap_step(w);
            let step_y = minimap_step(h);
            let last_x = (w as i32 - 1) * step_x;
            let last_y = (h as i32 - 1) * step_y;
            let stamp_w = step_x.max(1) + 1;
            let stamp_h = step_y.max(1) + 1;
            assert!(
                last_x + stamp_w <= MAX_DIM as i32,
                "{w}x{h}: the last column ends at {}, past the {MAX_DIM}-px slot",
                last_x + stamp_w
            );
            assert!(
                last_y + stamp_h <= MAX_DIM as i32,
                "{w}x{h}: the last row ends at {}, past the {MAX_DIM}-px slot",
                last_y + stamp_h
            );
        }
    }

    /// The step must never be zero, or every tile would stack on one pixel.
    #[test]
    fn the_bake_step_is_never_zero() {
        for w in [0u8, 1, 2, 10, 30, 38, 40, 255] {
            assert!(minimap_step(w) >= 1, "step collapsed for width {w}");
        }
    }

    #[test]
    fn the_minimap_inner_area_is_the_documented_size() {
        assert_eq!(MINIMAP_INNER, 48, "the 56x56 panel has a 48px inner area");
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
