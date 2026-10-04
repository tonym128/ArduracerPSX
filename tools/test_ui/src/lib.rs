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
