//! Headless autopilot for profiling runs.
//!
//! Exists only in a `profiling` build. The game normally takes its steering,
//! throttle and brake from `psx_pad::poll_port1`, which a headless emulator run
//! has no way to satisfy: without a pad the car sits on the grid and the track
//! texture streamer never sees the camera move, so there is nothing to measure.
//! The emulator can synthesise held buttons (`--hold-forward`) but cannot steer,
//! and steering is what decides which tiles come into view.
//!
//! So this builds a [`VehicleInput`] directly, from the same racing-line logic
//! the AI rivals use ([`AiRacer::compute_input`]), and hands it to the player
//! car in place of the pad-derived one. That matters for the measurement: the
//! autopilot drives the *real* physics through the *real* input type, so the
//! frame profile reflects the same work a player's frame does. Only the source
//! of the input differs.
//!
//! The AI racer is used as a steering brain rather than as a physics body. Its
//! `state` is overwritten with the player's every frame before
//! `compute_input` is called, so it reads the player's position, heading and
//! speed while retaining its own route-following bookkeeping (`target_gate_idx`
//! and friends) across frames. Driving a copy of the player's dynamic state
//! rather than the player's own struct keeps the autopilot out of the physics
//! path entirely.
//!
//! Nitro and handbrake are never asserted. They are not needed to hold a line
//! and they would add drift and boost states the measurement should not
//! confound the results with.

use arduracer_core::{AiRacer, TrackDef, VehicleInput, VehicleState};

/// Drives the player car around the circuit without a controller.
///
/// Holds one [`AiRacer`] purely for its routing state. Constructed at the grid
/// position so its first frame aims at the first route node rather than
/// somewhere arbitrary.
pub struct Autopilot {
    brain: AiRacer,
}

impl Autopilot {
    /// Creates an autopilot starting from the circuit's grid position.
    pub fn new(track: &TrackDef) -> Self {
        // Profile 0 is the shipped "balanced" rival; the autopilot only wants
        // its routing behaviour, and this keeps the choice in one place.
        Self {
            brain: AiRacer::new(
                track.start_pos,
                track.start_heading,
                arduracer_core::AI_PROFILES[0],
            ),
        }
    }

    /// Replaces the pad-derived `input` with this frame's autopilot input.
    ///
    /// Copies the player's dynamic state onto the routing brain first, so the
    /// line it aims for is computed from where the car actually is, not from
    /// where the brain last thought it was.
    ///
    /// `player` is mutable because the brain's stuck-recovery has to be mirrored
    /// onto the real car -- see the body.
    ///
    /// Takes `&mut input` rather than returning a value so the call site can
    /// discard the pad-derived input in profiling builds without the
    /// non-profiling build seeing an unused assignment.
    /// Deliberately `#[inline(never)]`.
    ///
    /// `compute_input` is a large non-inlined routine in another crate, so
    /// letting this one inline it places a call to it from inside
    /// `ArduracerGame::run`, in the largest function in the program. That is
    /// enough to push the pair past the R3000's branch range and fail the link
    /// with `out of range PC16 fixup`. Keeping the autopilot's own frame
    /// separate keeps the call site local. See the comment on
    /// `MAX_DECODES_PER_FRAME`-adjacent layout notes in `tracktex.rs` for the
    /// same class of constraint: the `.text` ordering in `psoxide.ld` is
    /// load-bearing for more than I-cache affinity.
    #[inline(never)]
    pub fn override_input(
        &mut self,
        track: &TrackDef,
        player: &mut VehicleState,
        input: &mut VehicleInput,
    ) {
        let before = player.position;
        self.brain.state = *player;
        *input = self.brain.compute_input(track, &[]);

        // Advance the brain's route target and stuck detector. Without this the
        // car aims at route node 0, reaches it in the first few seconds and then
        // sits there aiming at the same node for the rest of the run: no camera
        // motion, no streaming, nothing to measure.
        self.brain.update_routing(track);

        // Mirror a recovery onto the real car.
        //
        // `AiRacer::recover` teleports `brain.state`, and the line above copies
        // the player's state over `brain.state` every frame -- so a recovery was
        // silently discarded on the very next frame and the autopilot sat wedged
        // against whatever it hit for the rest of the run. With no pad there is
        // nothing to unstick it, which is why this measured "no streaming ever
        // happens" rather than "the car drove somewhere and stopped": the car
        // reached its first obstacle in the first few hundred frames and stayed
        // there, camera fixed, every tile already prewarmed.
        //
        // Detected by the position moving inside `compute_input`, which is the
        // only thing that call can do to the state it was handed.
        if player.position == before && self.brain.state.position != before {
            player.respawn_at(self.brain.state.position, self.brain.state.heading);
        }
    }
}
