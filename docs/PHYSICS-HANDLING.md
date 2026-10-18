# Vehicle handling targets and baseline

This note records the handling model used by `arduracer-core` and a repeatable
course comparison for future physics changes. The source constants remain the
runtime authority; this is the review guide, not a second tuning table.

## Handling targets

| Behavior | Current target | Runtime model |
| --- | --- | --- |
| Arcade pace | 3.42 world units per 60 Hz tick on the default car (about 205 units/s) | `BASE_TOP_SPEED`; pads and boost may exceed it |
| Acceleration | Reach about 92% of base top speed in roughly 1.8 s on tarmac | `BASE_ACCEL`, gear torque curve, and `DRAG_RETAIN` |
| Braking | Remove forward speed at about 0.044 units per tick, stopping from base top speed in about 1.3 s | `BRAKE_FORCE`; braking can transition to reverse |
| High-speed turn-in | Full-lock minimum radius near 128 world units, about two tiles | Speed-scaled steering authority, with a 25% floor |
| Grip and lateral slip | Slip cap eases from a 0.50 lateral/forward ratio at low speed to 0.25 at top speed; surface grip scales both | Friction circle in `VehicleState::tick` and each surface's `grip_factor` |
| Drift | Handbrake reduces steering authority to 75% of current grip authority; deeper slides lose more forward speed | `DRIFT_STEER_AUTHORITY`, `DRIFT_SLIDE_DRAG`, and the drift state machine |
| Drift recovery | Releasing handbrake keeps the drawn car aligned with its remaining slide; counter-steering can catch it | `slide_catch`, `DRIFT_SLIDE_RESPONSE`, and friction-circle scrub |
| Collision recovery | Hard impacts scrub 30% of speed; a spin scrubs lateral motion and remains steerable/recoverable | `HARD_IMPACT_SCRUB`, spin-out lateral hold, reverse, and cooldown |
| Boost | Standard drift/pad boost adds 0.49 units/tick of headroom; Reactor adds about 1.03 units/tick (30% of base) | Boost impulses and separate top-speed bonuses |

Surface response keeps engine traction, lateral grip, and top-speed limits
separate. That lets dirt, sand, ice, oil, curbs, and off-road terrain feel
different without turning a low-grip surface into a near-stop.

## Representative course comparison

Command: `cargo run --manifest-path tools/playtest/Cargo.toml --release`

The playtest drives production `VehicleState` on every authored circuit for
three laps, checks all five rival profiles, and compares the default tune with
the fastest of six valid garage presets. This run completed all 40 circuits.
Representative times from the run:

| Course | Type | Default | Fastest swept tune | Improvement | Laps / AI profiles |
| --- | --- | ---: | ---: | ---: | --- |
| Speedway Boulevard | Oval / high-speed | 26.50 s | 21.40 s | 19.2% | 3 / 5 |
| Devils Hairpin | Hairpin | 39.55 s | 31.96 s | 19.2% | 3 / 5 |
| Alleyway Chicanes | Chicane | 75.76 s | 64.63 s | 14.7% | 3 / 5 |
| Mirage Dunes Drift | Mixed loose-surface / drift | 27.68 s | 22.88 s | 17.3% | 3 / 5 |

Every course verdict was `ok`; the harness also reported a sampled maximum
speed of 3.91 units/tick on these four courses, which is above the base cap due
to course boost surfaces. The comparison confirms that the current default
setup is drivable on the four different shapes and that the garage sweep
produces meaningful pace changes.

## Limits and tuning rule

The harness is deterministic and exercises production vehicle and course data,
but its reference driver is scripted. It is not a substitute for human feel,
controller testing, or dedicated braking-distance and drift-recovery telemetry.
Do not change the shared constants from lap time alone: capture before/after
behavior on these four courses, then recalibrate medal par times if the measured
laps move. Add steering, braking, lateral-slip, and recovery measurements to
the harness when a future pass needs more than the existing lap-level signal.
