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
mod audio_policy;

#[cfg(test)]
mod tests {
    use super::audio_policy::{
        AudioPolicy, UiSound, BTN_CIRCLE, BTN_CROSS, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT,
        BTN_START, BTN_UP,
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
