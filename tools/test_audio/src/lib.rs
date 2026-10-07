//! Host-side tests for the game's audio policy.
//!
//! `audio_policy.rs` is included textually (the same trick as `tools/test_ui`)
//! because it was written to have no hardware dependencies: it decides which
//! voices *should* sound from plain booleans and a raw button mask, and never
//! touches `psx_spu`.
//!
//! This exists because of a bug that lived precisely in the untested gap. The
//! engine is a continuous SPU loop keyed on at boot, and its volume register
//! was only ever rewritten from `AudioSystem::tick` -- which the race arm of
//! the state match calls and no other arm does. Every other screen, plus the
//! pause veil (which `continue`s before the tick), therefore inherited whatever
//! volume the last racing frame left behind, and the engine droned over the
//! results fanfare and under the menu music. Worse, the frame that finished the
//! stage still ran the tick once after switching state, re-latching a live
//! engine volume on top of the transition.
//!
//! `AudioPolicy` makes that class of bug a no-op: a screen that forgets to opt
//! back in stays silent rather than buzzing.

#![allow(dead_code)]

#[path = "../../../game/src/audio/audio_policy.rs"]
#[allow(unused_imports)]
mod audio_policy;

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::audio_policy::{
        engine_pitch_for, AudioPolicy, UiSound, BTN_CIRCLE, BTN_CROSS, BTN_DOWN, BTN_LEFT,
        BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP, IDLE_PITCH, MAX_ENGINE_PITCH, MAX_RPM,
        MIN_ENGINE_PITCH, MIN_RPM,
    };

    fn held(buttons: u16) -> u16 {
        buttons
    }

    #[test]
    fn a_freshly_built_policy_plays_no_race_voices() {
        let policy = AudioPolicy::new();
        assert!(
            !policy.racing_voices(),
            "boot must be silent: the title screen and intro play CD-DA only"
        );
    }

    #[test]
    fn a_stage_load_arms_the_race_voices() {
        let mut policy = AudioPolicy::new();
        policy.enter_race();
        assert!(policy.racing_voices());
    }

    #[test]
    fn leaving_a_race_cuts_the_race_voices() {
        let mut policy = AudioPolicy::new();
        policy.enter_race();
        policy.silence_race_voices();
        assert!(
            !policy.racing_voices(),
            "results screen and menus must not keep the engine armed"
        );
    }

    #[test]
    fn pausing_cuts_the_engine_and_unpausing_restores_it() {
        let mut policy = AudioPolicy::new();
        policy.enter_race();
        assert!(policy.racing_voices());

        policy.set_paused(true);
        assert!(
            !policy.racing_voices(),
            "the pause veil must silence the engine immediately"
        );

        policy.set_paused(false);
        assert!(
            policy.racing_voices(),
            "resuming must re-arm the engine"
        );
    }

    #[test]
    fn a_still_paused_race_stays_silent_even_if_a_transition_re_arms_it() {
        // `QuitToMenu` leaves `self.paused == true` while changing state, and
        // `RestartRace` re-arms the race from inside the veil. The veil has to
        // veto independently, or either path leaks the engine.
        let mut policy = AudioPolicy::new();
        policy.set_paused(true);
        policy.enter_race();
        assert!(
            !policy.racing_voices(),
            "the pause veil must veto even after enter_race re-arms the stage"
        );

        policy.set_paused(false);
        assert!(
            policy.racing_voices(),
            "lifting the veil must let the re-armed stage sound again"
        );
    }

    #[test]
    fn a_menu_navigation_press_plays_one_move_cue() {
        let mut policy = AudioPolicy::new();
        assert_eq!(policy.ui_sound(held(BTN_DOWN)), Some(UiSound::Move));
    }

    #[test]
    fn every_direction_button_moves_the_highlight() {
        for buttons in [BTN_UP, BTN_DOWN, BTN_LEFT, BTN_RIGHT] {
            let mut policy = AudioPolicy::new();
            assert_eq!(
                policy.ui_sound(held(buttons)),
                Some(UiSound::Move),
                "direction {buttons:#06x} must sound"
            );
        }
    }

    #[test]
    fn holding_a_button_does_not_retrigger_the_cue() {
        let mut policy = AudioPolicy::new();
        assert_eq!(policy.ui_sound(held(BTN_DOWN)), Some(UiSound::Move));
        // Held for the next five frames: no further cue.
        for _ in 0..5 {
            assert_eq!(
                policy.ui_sound(held(BTN_DOWN)),
                None,
                "a held button must not machine-gun the blip"
            );
        }
        // Release, then press again.
        assert_eq!(policy.ui_sound(held(0)), None);
        assert_eq!(policy.ui_sound(held(BTN_DOWN)), Some(UiSound::Move));
    }

    #[test]
    fn confirming_sounds_confirm_and_back_sounds_back() {
        let mut policy = AudioPolicy::new();
        assert_eq!(policy.ui_sound(held(BTN_CROSS)), Some(UiSound::Confirm));
        assert_eq!(policy.ui_sound(held(BTN_START)), Some(UiSound::Confirm));

        let mut policy = AudioPolicy::new();
        assert_eq!(policy.ui_sound(held(BTN_CIRCLE)), Some(UiSound::Back));
    }

    #[test]
    fn a_confirm_beats_a_move_on_the_same_frame() {
        // Pressing CROSS does not also move the highlight, and a confirm is the
        // more informative cue, so it must win.
        let mut policy = AudioPolicy::new();
        assert_eq!(
            policy.ui_sound(held(BTN_DOWN | BTN_CROSS)),
            Some(UiSound::Confirm)
        );
    }

    #[test]
    fn at_most_one_cue_per_frame() {
        let mut policy = AudioPolicy::new();
        let cue = policy.ui_sound(held(BTN_UP | BTN_CROSS | BTN_CIRCLE));
        assert_eq!(cue, Some(UiSound::Confirm));
    }

    #[test]
    fn select_alone_is_silent() {
        // Select toggles the HUD mid-race and has no menu cue of its own.
        let mut policy = AudioPolicy::new();
        assert_eq!(policy.ui_sound(held(BTN_SELECT)), None);
    }

    #[test]
    fn syncing_edges_does_not_replay_a_held_button() {
        // A transition that skips frames (a restart, or leaving the pause veil)
        // must not re-read the press that caused it as a fresh cue on the new
        // screen.
        let mut policy = AudioPolicy::new();
        policy.sync_edges(held(BTN_CROSS));
        assert_eq!(
            policy.ui_sound(held(BTN_CROSS)),
            None,
            "the same press must not fire twice across a transition"
        );
        // Still edge-tracks properly afterwards.
        assert_eq!(policy.ui_sound(held(0)), None);
        assert_eq!(policy.ui_sound(held(BTN_CROSS)), Some(UiSound::Confirm));
    }

    #[test]
    fn ui_cues_do_not_disturb_the_race_voice_gate() {
        // Menus play navigation blips while the race voices stay disarmed, and
        // a blip must never be what re-arms the engine.
        let mut policy = AudioPolicy::new();
        assert_eq!(policy.ui_sound(held(BTN_DOWN)), Some(UiSound::Move));
        assert_eq!(policy.ui_sound(held(BTN_CROSS)), Some(UiSound::Confirm));
        assert!(
            !policy.racing_voices(),
            "menu audio must not arm the race voices"
        );
    }

    #[test]
    fn a_cue_during_the_pause_veil_leaves_the_engine_silent() {
        // The veil is a menu: the player can still move the highlight, and the
        // blip is allowed, but the engine must not come back.
        let mut policy = AudioPolicy::new();
        policy.set_paused(true);
        assert_eq!(policy.ui_sound(held(BTN_DOWN)), Some(UiSound::Move));
        assert!(
            !policy.racing_voices(),
            "navigating the pause menu must not restart the engine"
        );
    }
}

/// TASK-1207b: the crash sample restarted on every frame of a sustained scrape.
///
/// `hit_wall` is true for as long as the car touches a barrier, and the old code
/// played the impact unconditionally on that signal. At 60 Hz a 0.25 s sample
/// was restarted 60 times a second, which is heard as a buzz.
#[cfg(test)]
mod crash_gate_tests {
    use super::audio_policy::{CrashGate, CRASH_REFRACTORY_FRAMES};

    /// Feeds a run of `hit_wall` values and counts the triggers.
    fn triggers(gate: &mut CrashGate, frames: &[bool]) -> usize {
        frames.iter().filter(|hit| gate.should_fire(**hit)).count()
    }

    #[test]
    fn a_single_frame_of_contact_sounds_once() {
        let mut gate = CrashGate::new();
        assert_eq!(triggers(&mut gate, &[false, false, true, false, false]), 1);
    }

    #[test]
    fn a_sustained_scrape_sounds_once_not_sixty_times() {
        // The regression: 60 frames of unbroken contact.
        let sustained = [true; 60];
        let mut gate = CrashGate::new();
        assert_eq!(
            triggers(&mut gate, &sustained),
            1,
            "a car grinding a wall retriggered the crash every frame"
        );
    }

    #[test]
    fn the_sample_length_bounds_the_retrigger_rate() {
        // Releasing and re-touching faster than the sample plays must not stack
        // impacts, or a bounce along a kerb becomes a roll of thunder.
        let mut gate = CrashGate::new();
        // Contact on frames 0, 2, 4, ... -- far faster than the refractory.
        let chatter: Vec<bool> = (0..40).map(|f| f % 2 == 0).collect();
        let count = triggers(&mut gate, &chatter);
        assert!(
            count <= 40 / CRASH_REFRACTORY_FRAMES as usize + 1,
            "{count} impacts from 20 contacts in 40 frames; the refractory \
             period is {CRASH_REFRACTORY_FRAMES} frames"
        );
    }

    #[test]
    fn a_deliberate_second_impact_after_the_refractory_sounds() {
        // The rate limit must not swallow real, separate knocks.
        let mut gate = CrashGate::new();
        assert!(gate.should_fire(true), "first contact must sound");
        // Release, then stay clear past the refractory period.
        assert!(!gate.should_fire(false));
        for _ in 0..CRASH_REFRACTORY_FRAMES {
            assert!(!gate.should_fire(false));
        }
        assert!(
            gate.should_fire(true),
            "a genuine second impact after the refractory period must sound"
        );
    }

    #[test]
    fn resetting_makes_the_next_contact_sound_immediately() {
        // `silence()` runs on menus and the pause veil; after it the car may be
        // touching a barrier again on the very next frame, and that must count.
        let mut gate = CrashGate::new();
        assert!(gate.should_fire(true));
        gate.reset();
        assert!(
            gate.should_fire(true),
            "contact right after a silence was swallowed by the stale edge state"
        );
    }

    #[test]
    fn no_contact_never_sounds() {
        let mut gate = CrashGate::new();
        assert_eq!(triggers(&mut gate, &[false; 200]), 0);
    }
}

/// TASK-1207: the engine loop was driven past its own Nyquist limit.
///
/// The pitch register is a Q12 multiplier on the sample rate, so 0x1000 replays
/// a 22.05 kHz sample at its recorded rate. The old map reached 0x3400 -- 3.25x,
/// 2.4x past Nyquist -- so the harmonics folded back over themselves and the
/// engine sounded like a buzz at high RPM rather than an engine.
#[cfg(test)]
mod engine_pitch_tests {
    use super::audio_policy::{
        engine_pitch_for, IDLE_PITCH, MAX_ENGINE_PITCH, MAX_RPM, MIN_ENGINE_PITCH, MIN_RPM,
    };

    #[test]
    fn no_rpm_can_pitch_the_loop_past_nyquist() {
        // Sweep the whole u16 range, not just the nominal band: a caller could
        // pass anything, and `update` is not the only reader.
        for rpm in 0..=u16::MAX {
            assert!(
                engine_pitch_for(rpm) <= MAX_ENGINE_PITCH,
                "rpm {rpm} produced pitch {} above the ceiling {MAX_ENGINE_PITCH}",
                engine_pitch_for(rpm),
            );
        }
    }

    #[test]
    fn the_map_stays_within_its_own_declared_band() {
        assert!(fits(MIN_ENGINE_PITCH, MAX_ENGINE_PITCH));
        assert_eq!(engine_pitch_for(MIN_RPM), MIN_ENGINE_PITCH);
        assert_eq!(engine_pitch_for(MAX_RPM), MAX_ENGINE_PITCH);
    }

    #[test]
    fn rpm_outside_the_nominal_range_is_clamped_not_wrapped() {
        // `update` clamps before calling, but the pure function must be safe
        // on its own: a below-range RPM must not wrap to a huge value.
        assert_eq!(engine_pitch_for(0), MIN_ENGINE_PITCH);
        assert_eq!(engine_pitch_for(MIN_RPM - 1), MIN_ENGINE_PITCH);
        assert_eq!(engine_pitch_for(MAX_RPM + 1), MAX_ENGINE_PITCH);
        assert_eq!(engine_pitch_for(u16::MAX), MAX_ENGINE_PITCH);
    }

    #[test]
    fn pitch_rises_monotonically_with_rpm() {
        let mut previous = engine_pitch_for(MIN_RPM);
        for rpm in MIN_RPM..=MAX_RPM {
            let pitch = engine_pitch_for(rpm);
            assert!(
                pitch >= previous,
                "pitch fell from {previous} to {pitch} at rpm {rpm}; an engine \
                 that drops pitch as it revs sounds broken"
            );
            previous = pitch;
        }
    }

    /// Const-asserts a compile-time invariant. Written as a function so clippy
    /// does not flag `assert!` on constants.
    const fn fits(value: u16, ceiling: u16) -> bool {
        value <= ceiling
    }

    /// True when `value` is non-zero.
    const fn audible(value: u16) -> bool {
        value != 0
    }

    #[test]
    fn the_resting_pitch_is_inside_the_band() {
        // `EngineAudio::new()` seeds this before any RPM is known. The old seed
        // was 0x0A00, above the ceiling.
        assert!(fits(IDLE_PITCH, MAX_ENGINE_PITCH));
        assert!(audible(IDLE_PITCH), "a zero resting pitch is inaudible");
    }

    #[test]
    fn the_whole_map_is_below_the_true_sample_rate() {
        // 0x1000 is the sample's own rate. Staying under it everywhere is what
        // keeps the harmonics from folding.
        assert!(fits(MAX_ENGINE_PITCH, 0x0FFF));
    }
}

/// TASK-1215: the engine was inaudible for the whole race.
///
/// `init_spu_soundbank` gained a comment saying the engine loop "stays keyed
/// on" in the same commit that *deleted* the `Voice::key_on` call it was
/// describing. Key state and volume are independent on the SPU: a voice that
/// has never been keyed on is never decoded, so writing a volume register to it
/// produces nothing. Every test in this file still passed, because the policy
/// decides only *whether the engine should sound*, and it said yes throughout
/// the race -- into a voice the hardware was not playing.
///
/// These are source-level assertions. The bug was not a policy decision, it was
/// a missing MMIO write that no amount of policy testing can reach, so the only
/// honest place to pin it is the file that makes the write.
#[cfg(test)]
mod engine_key_state_tests {
    /// `game/src/audio/spu.rs`, as text.
    const SPU_SRC: &str = include_str!("../../../game/src/audio/spu.rs");

    #[test]
    fn the_engine_voice_is_keyed_on_at_init() {
        assert!(
            SPU_SRC.contains("Voice::key_on(VOICE_ENGINE.mask())"),
            "the engine loop must be keyed on at init: an ADPCM voice that is \
             never keyed on is never decoded, so its volume register has no \
             effect and the engine is silent for the entire race"
        );
    }

    #[test]
    fn the_engine_is_not_keyed_on_or_off_per_frame() {
        // Arming belongs at init. Re-keying a loop restarts its ADPCM decoder
        // mid-sample and clicks, so `key_on`/`key_off` for the engine must not
        // appear in the per-frame path -- only the one call at init.
        let on = SPU_SRC.matches("Voice::key_on(VOICE_ENGINE.mask())").count();
        assert_eq!(on, 1, "the engine must be armed exactly once, at init");
        assert!(
            !SPU_SRC.contains("Voice::key_off(VOICE_ENGINE.mask())"),
            "the engine must never be keyed off: silence is expressed as zero \
             volume so the decoder keeps its phase and restarting is click-free"
        );
    }

    /// The SPU init is where a *continuous* voice has to be armed. The skid and
    /// curb loops are different: they start and stop with the drift state
    /// machine in `sfx.rs`, which is correct for them and is why arming them
    /// here would be wrong. This test pins that division of labour, so the next
    /// looping voice added has to decide which side of it it belongs on instead
    /// of defaulting to whichever pattern it copied.
    #[test]
    fn only_the_engine_is_a_continuously_armed_loop() {
        let sfx = include_str!("../../../game/src/audio/sfx.rs");
        // The drift state machine owns these two, both ways.
        for voice in ["VOICE_SKID", "VOICE_CURB"] {
            assert!(
                sfx.contains(&format!("Voice::key_on({voice}.mask())"))
                    && sfx.contains(&format!("Voice::key_off({voice}.mask())")),
                "{voice} is keyed on and off by the drift state machine"
            );
            assert!(
                !SPU_SRC.contains(&format!("Voice::key_on({voice}.mask())")),
                "{voice} must not also be armed at init: it would then run \
                 silently under the state machine for the whole race"
            );
        }
        // The engine is the opposite case, and the one this bug was about.
        assert!(
            SPU_SRC.contains("Voice::key_on(VOICE_ENGINE.mask())"),
            "the engine is continuous, so init is the only place it can be armed"
        );
        assert!(
            !sfx.contains("VOICE_ENGINE"),
            "the per-frame SFX path must not touch the engine volume: \
             AudioSystem::tick owns it, behind the policy gate"
        );
    }
}
