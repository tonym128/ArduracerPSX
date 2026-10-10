//! Car-to-car contact: player <-> rival and rival <-> rival.
//!
//! Before this module existed every car on the grid was a ghost to every other
//! car. The AI's obstacle avoidance steered rivals *around* traffic, but nothing
//! stopped two cars that failed to dodge from driving straight through one
//! another, and the player could drive through the whole field.
//!
//! # Body shape
//!
//! The drawn car body is 14 units wide and 24 units long (`car_renderer.rs`:
//! `make_quad(cx, cy, 6, 7, 12, 12, rot)` plus the wheels at +/-7). A single
//! circle cannot match that: sized to the length the cars bounce off each other
//! with a visible gap side by side, sized to the width they overlap nose to
//! tail. So each car is two circles of [`CAR_BODY_RADIUS`] placed
//! [`CAR_BODY_HALF_SPAN`] ahead of and behind its centre along the drawn
//! heading, which spans exactly 14 x 24. The deepest-penetrating circle pair
//! decides the contact normal, so a nose into a flank pushes sideways and a nose
//! into a tail pushes along the track.
//!
//! # Response
//!
//! Every car weighs the same (tuning has no mass slider), so the response is
//! the equal-mass case of a standard impulse solver:
//!
//! * **Position:** the overlap is split evenly and both cars are pushed apart
//!   along the contact normal in the same tick, so cars never interpenetrate
//!   across a frame however fast they were closing.
//! * **Velocity:** only the closing component along the normal is touched, with
//!   [`CAR_RESTITUTION`]. A car that is already pulling away is never pulled
//!   back, and the tangential component is left to the tyres -- the lateral grip
//!   model in [`VehicleState::tick`] scrubs a side-swipe on its own, which is
//!   what makes a rub read as a rub rather than a pinball bounce.
//!
//! Car contacts never trigger a spin-out. A wall is a fixed hazard the driver
//! can learn; a rival tapping the player's rear quarter is not, and a race in
//! which every nudge cost a second of helpless spinning would be decided by the
//! AI's avoidance rather than by driving.
//!
//! Total for every input: the broad phase works in `i32` deltas with a fixed
//! reach, the narrow phase in 64-bit squares, and a zero-length normal falls
//! back to a deterministic axis instead of dividing by zero.

use crate::ai::AiRacer;
use crate::math::{self, Fixed, Vec2, FP_ONE, FP_SHIFT};
use crate::track::TrackDef;
use crate::vehicle::VehicleState;

/// Radius of each of the two circles that make up a car body, in world units.
/// Twice this is the drawn body width (14 units).
pub const CAR_BODY_RADIUS: i32 = 7;
/// Distance of each body circle from the car centre along its heading, in world
/// units. `2 * (CAR_BODY_HALF_SPAN + CAR_BODY_RADIUS)` is the drawn body length
/// (24 units).
pub const CAR_BODY_HALF_SPAN: i32 = 5;
/// Centre-to-centre separation beyond which two cars cannot be touching on
/// either axis. Lets the broad phase reject almost every pair with two `i32`
/// compares, which matters on a 33 MHz R3000 running 15 pairs a tick.
const BROADPHASE_REACH: i32 = 2 * (CAR_BODY_HALF_SPAN + CAR_BODY_RADIUS) + 1;
/// Coefficient of restitution between two cars (0.3). Bouncier than this and a
/// rear-end launches the car in front; deader and two cars side by side stick
/// together through a corner.
pub const CAR_RESTITUTION: Fixed = Fixed::from_raw(1229);
/// Closing speed (world units per tick) above which a contact is a *hit* worth
/// sparks, a thud and a rumble rather than a silent rub. Half a unit per tick is
/// about 15% of top speed: a deliberate rear-end, not two cars drifting
/// together on corner exit.
pub const HARD_BUMP_THRESHOLD: Fixed = Fixed::from_raw(2048);
/// Extra separation added on top of the measured overlap, in raw Q20.12, so
/// rounding cannot leave two cars a hair inside each other and re-colliding on
/// the next tick.
const SEPARATION_SLOP_RAW: i32 = 16;

/// A resolved contact between two cars.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CarContact {
    /// Unit normal pointing from the first car towards the second.
    pub normal: Vec2,
    /// World-space point where the two bodies touched (after separation).
    pub point: Vec2,
    /// Speed at which the two cars were approaching along `normal` before the
    /// impulse, in world units per tick. Zero for a contact that was already
    /// separating (resolved by position only).
    pub closing_speed: Fixed,
}

/// The two body circle centres of `car`, nose first.
#[inline]
fn body_circles(car: &VehicleState) -> [Vec2; 2] {
    let angle = car.visual_angle;
    let ahead =
        Vec2::new(math::sin(angle), -math::cos(angle)).scale(Fixed::from_int(CAR_BODY_HALF_SPAN));
    [car.position + ahead, car.position - ahead]
}

/// Squared length of `v` in raw Q20.12 units, in 64 bits.
#[inline]
fn raw_length_sq(v: Vec2) -> i64 {
    let (x, y) = (v.x.raw() as i64, v.y.raw() as i64);
    x * x + y * y
}

/// `v / |v|` with `len_raw == |v|` already known, computed per component in 64
/// bits so the result is a true unit vector rather than one rounded through a
/// Q20.12 reciprocal.
#[inline]
fn unit_from(v: Vec2, len_raw: i32) -> Vec2 {
    let len = len_raw.max(1) as i64;
    Vec2::new(
        Fixed::from_raw(((v.x.raw() as i64 * FP_ONE as i64) / len) as i32),
        Fixed::from_raw(((v.y.raw() as i64 * FP_ONE as i64) / len) as i32),
    )
}

/// Detects and resolves contact between two cars, mutating both.
///
/// Returns `None` (and leaves both cars untouched) if the bodies do not
/// overlap. On contact both cars are pushed apart so they no longer overlap,
/// the closing velocity along the contact normal is replaced by a
/// [`CAR_RESTITUTION`] bounce shared equally between them, and both cars'
/// `speed` fields are resynchronised with their velocities.
///
/// The caller is responsible for re-running track collision afterwards: the
/// separation push can move a car into a barrier. [`resolve_field`] does this.
pub fn collide_cars(a: &mut VehicleState, b: &mut VehicleState) -> Option<CarContact> {
    // --- Broad phase ------------------------------------------------------------
    let centre_delta = b.position - a.position;
    let reach = BROADPHASE_REACH << FP_SHIFT;
    // `unsigned_abs` so a pathological i32::MIN delta cannot overflow `abs`.
    if centre_delta.x.raw().unsigned_abs() >= reach as u32
        || centre_delta.y.raw().unsigned_abs() >= reach as u32
    {
        return None;
    }

    // --- Narrow phase + position ---------------------------------------------------
    // The deepest pair decides the contact. Separating along it can still leave
    // a *second* pair overlapping -- a nose driven into the middle of a flank
    // touches both of the other car's circles -- so the push is repeated on
    // whatever pair is deepest afterwards, a bounded number of times.
    let (normal, circle_a, half_push) = separate_deepest(a, b)?;
    for _ in 1..SEPARATION_PASSES {
        if separate_deepest(a, b).is_none() {
            break;
        }
    }

    // --- Velocity: equal-mass impulse along the primary normal -------------------
    let approach = (b.velocity - a.velocity).dot(normal);
    let closing_speed = if approach < Fixed::ZERO {
        let closing = -approach;
        // Each car takes half of (1 + e) * closing.
        let share = (Fixed::ONE + CAR_RESTITUTION) * closing * Fixed::HALF;
        a.velocity = a.velocity - normal.scale(share);
        b.velocity = b.velocity + normal.scale(share);
        closing
    } else {
        Fixed::ZERO
    };

    a.speed = a.velocity.length();
    b.speed = b.velocity.length();

    // The touch point sits on `a`'s body circle surface, moved with it.
    let point = circle_a - normal.scale(half_push) + normal.scale(Fixed::from_int(CAR_BODY_RADIUS));

    Some(CarContact {
        normal,
        point,
        closing_speed,
    })
}

/// Upper bound on positional separation passes per contact. Two circles a
/// side means at most two pairs can overlap at once in practice; the third
/// pass is margin.
const SEPARATION_PASSES: usize = 3;

/// Finds the deepest-overlapping body circle pair and pushes the two cars
/// apart along it, splitting the overlap evenly.
///
/// Returns `(unit normal from a to b, a's circle centre before the push, the
/// push each car received)`, or `None` if no pair overlaps.
fn separate_deepest(a: &mut VehicleState, b: &mut VehicleState) -> Option<(Vec2, Vec2, Fixed)> {
    let contact_raw = (2 * CAR_BODY_RADIUS) << FP_SHIFT;
    let contact_sq = (contact_raw as i64) * (contact_raw as i64);
    let circles_a = body_circles(a);
    let circles_b = body_circles(b);
    let mut deepest: Option<(i64, Vec2, Vec2)> = None;
    for &ca in circles_a.iter() {
        for &cb in circles_b.iter() {
            let delta = cb - ca;
            let dist_sq = raw_length_sq(delta);
            if dist_sq < contact_sq && deepest.is_none_or(|(best, _, _)| dist_sq < best) {
                deepest = Some((dist_sq, delta, ca));
            }
        }
    }
    let (_, delta, circle_a) = deepest?;

    // Fall back from the circle pair to the car centres, and from the car
    // centres to `a`'s right-hand side, so two cars spawned exactly on top of
    // each other still separate -- deterministically, and without a divide by
    // zero.
    let dist_raw = delta.length().raw();
    let normal = if dist_raw > 0 {
        unit_from(delta, dist_raw)
    } else {
        let centre_delta = b.position - a.position;
        let centre_len = centre_delta.length().raw();
        if centre_len > 0 {
            unit_from(centre_delta, centre_len)
        } else {
            a.right_dir()
        }
    };

    let penetration = (contact_raw - dist_raw).max(0);
    let half_push = Fixed::from_raw(penetration / 2 + SEPARATION_SLOP_RAW);
    a.position = a.position - normal.scale(half_push);
    b.position = b.position + normal.scale(half_push);
    Some((normal, circle_a, half_push))
}

/// What the field's contacts did to the player this tick, for the game's
/// feedback layer (sparks, rumble, impact sound).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldContacts {
    /// Whether the player touched any rival this tick.
    pub player_touched: bool,
    /// Hardest closing speed of any contact involving the player, in world
    /// units per tick. Zero if there was none, or if every contact was a rub.
    pub player_impact: Fixed,
    /// Where the hardest player contact happened.
    pub player_contact_point: Vec2,
    /// Unit normal of the hardest player contact, pointing *away* from the
    /// player towards the rival it touched.
    pub player_contact_normal: Vec2,
    /// Whether a contact shoved the player into a track barrier.
    pub player_hit_wall: bool,
    /// Number of car-to-car contacts resolved this tick (all pairs).
    pub contact_count: u8,
}

impl FieldContacts {
    /// Whether the player took a hit hard enough to deserve sparks and a thud.
    #[inline]
    pub fn player_hard_hit(&self) -> bool {
        self.player_touched && self.player_impact > HARD_BUMP_THRESHOLD
    }

    fn record_player(&mut self, contact: CarContact, normal_away_from_player: Vec2) {
        if !self.player_touched || contact.closing_speed > self.player_impact {
            self.player_impact = contact.closing_speed;
            self.player_contact_point = contact.point;
            self.player_contact_normal = normal_away_from_player;
        }
        self.player_touched = true;
    }
}

/// Whether a rival takes part in contact this tick.
///
/// A rival that has taken the chequered flag brakes to a halt just past the
/// line (`AiRacer::compute_input`) and stays there. Left solid it would be a
/// parked car on the racing line for everyone still lapping, so a finished
/// rival is retired from contact, exactly as it is retired from the race.
#[inline]
fn is_solid(rival: &AiRacer) -> bool {
    !rival.is_finished
}

/// Resolves every car-to-car contact on the grid for one tick: the player
/// against each rival, then each rival against every other.
///
/// Run after all cars have been integrated for the tick. Any car moved by a
/// contact is re-resolved against the track afterwards, so a car shoved into a
/// barrier is pushed back out onto the circuit on the same tick rather than
/// ending it inside a wall.
///
/// One pass, in a fixed order: the result is deterministic, which the ghost and
/// replay code relies on. Up to 32 rivals are considered; the game races five.
pub fn resolve_field(
    player: &mut VehicleState,
    rivals: &mut [AiRacer],
    track: &TrackDef,
) -> FieldContacts {
    let mut out = FieldContacts::default();
    let count = rivals.len().min(32);
    let rivals = &mut rivals[..count];
    let mut moved: u32 = 0;

    // Player <-> rival.
    for (i, rival) in rivals.iter_mut().enumerate() {
        if !is_solid(rival) {
            continue;
        }
        if let Some(contact) = collide_cars(player, &mut rival.state) {
            moved |= 1 << i;
            out.contact_count = out.contact_count.saturating_add(1);
            out.record_player(contact, contact.normal);
        }
    }

    // Rival <-> rival.
    for i in 0..count {
        let (head, tail) = rivals.split_at_mut(i + 1);
        let a = &mut head[i];
        if !is_solid(a) {
            continue;
        }
        for (offset, b) in tail.iter_mut().enumerate() {
            if !is_solid(b) {
                continue;
            }
            if collide_cars(&mut a.state, &mut b.state).is_some() {
                moved |= (1 << i) | (1 << (i + 1 + offset));
                out.contact_count = out.contact_count.saturating_add(1);
            }
        }
    }

    // A separation push can land a car in a barrier: put it back on the road.
    if out.player_touched {
        out.player_hit_wall = player.collide_with_track(track);
    }
    for (i, rival) in rivals.iter_mut().enumerate() {
        if moved & (1 << i) != 0 {
            rival.state.collide_with_track(track);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_profiles::AI_PROFILES;
    use crate::levels::ALL_TRACKS;
    use crate::tuning::CarTuning;

    fn car_at(x: i32, y: i32, heading: u16) -> VehicleState {
        VehicleState::new(
            Vec2::new(Fixed::from_int(x), Fixed::from_int(y)),
            heading,
            CarTuning::default(),
        )
    }

    fn with_velocity(mut car: VehicleState, vx: Fixed, vy: Fixed) -> VehicleState {
        car.velocity = Vec2::new(vx, vy);
        car.speed = car.velocity.length();
        car
    }

    /// Smallest separation between any circle of `a` and any circle of `b`,
    /// in raw Q20.12.
    fn body_gap_raw(a: &VehicleState, b: &VehicleState) -> i32 {
        let mut best = i32::MAX;
        for ca in body_circles(a) {
            for cb in body_circles(b) {
                best = best.min((cb - ca).length().raw());
            }
        }
        best - ((2 * CAR_BODY_RADIUS) << FP_SHIFT)
    }

    fn momentum(a: &VehicleState, b: &VehicleState) -> Vec2 {
        a.velocity + b.velocity
    }

    #[test]
    fn body_matches_the_drawn_car() {
        // 14 wide, 24 long: see the module docs and `car_renderer.rs`.
        assert_eq!(2 * CAR_BODY_RADIUS, 14);
        assert_eq!(2 * (CAR_BODY_HALF_SPAN + CAR_BODY_RADIUS), 24);
    }

    #[test]
    fn distant_cars_are_untouched() {
        let mut a = with_velocity(car_at(100, 100, 0), Fixed::ONE, Fixed::ZERO);
        let mut b = with_velocity(car_at(200, 100, 0), -Fixed::ONE, Fixed::ZERO);
        let (a0, b0) = (a, b);
        assert!(collide_cars(&mut a, &mut b).is_none());
        assert_eq!(a, a0);
        assert_eq!(b, b0);
    }

    #[test]
    fn side_by_side_contact_happens_at_the_body_width() {
        // Both facing north (heading 0): the body is 14 units across.
        let mut a = car_at(100, 100, 0);
        let mut b = car_at(115, 100, 0);
        assert!(collide_cars(&mut a, &mut b).is_none(), "15 apart is a gap");
        let mut a = car_at(100, 100, 0);
        let mut b = car_at(112, 100, 0);
        let contact = collide_cars(&mut a, &mut b).expect("12 apart overlaps");
        // Pushed straight apart on the x axis.
        assert!(contact.normal.x > Fixed::from_raw(4000));
        assert!(contact.normal.y.raw().abs() < 64);
        assert!(body_gap_raw(&a, &b) >= 0);
    }

    #[test]
    fn nose_to_tail_contact_happens_at_the_body_length() {
        let mut a = car_at(100, 100, 0);
        let mut b = car_at(100, 75, 0);
        assert!(collide_cars(&mut a, &mut b).is_none(), "25 apart is a gap");
        let mut a = car_at(100, 100, 0);
        let mut b = car_at(100, 80, 0);
        let contact = collide_cars(&mut a, &mut b).expect("20 apart overlaps");
        // b is ahead (north, -y) of a.
        assert!(contact.normal.y < Fixed::from_raw(-4000));
        assert!(body_gap_raw(&a, &b) >= 0);
    }

    #[test]
    fn overlap_is_fully_separated_in_one_tick() {
        for (dx, dy, ha, hb) in [
            (3, 0, 0u16, 0u16),
            (0, 3, 0, 1024),
            (5, 5, 512, 3000),
            (-9, 2, 2048, 100),
            (0, -15, 0, 0),
        ] {
            let mut a = car_at(300, 300, ha);
            let mut b = car_at(300 + dx, 300 + dy, hb);
            collide_cars(&mut a, &mut b).expect("overlapping");
            assert!(
                body_gap_raw(&a, &b) >= 0,
                "still overlapping after resolve for {:?}",
                (dx, dy, ha, hb)
            );
            assert!(collide_cars(&mut a, &mut b).is_none());
        }
    }

    #[test]
    fn coincident_cars_separate_deterministically() {
        let mut a = car_at(300, 300, 0);
        let mut b = car_at(300, 300, 0);
        let contact = collide_cars(&mut a, &mut b).expect("coincident");
        assert_eq!(contact.normal, a.right_dir());
        assert!(body_gap_raw(&a, &b) >= 0);

        let mut a2 = car_at(300, 300, 0);
        let mut b2 = car_at(300, 300, 0);
        assert_eq!(collide_cars(&mut a2, &mut b2), Some(contact));
        assert_eq!((a, b), (a2, b2));
    }

    #[test]
    fn rear_end_transfers_momentum_and_stops_the_approach() {
        // a drives north at 3 u/tick into a stationary b.
        let mut a = with_velocity(car_at(100, 100, 0), Fixed::ZERO, Fixed::from_int(-3));
        let mut b = car_at(100, 78, 0);
        let before = momentum(&a, &b);
        let contact = collide_cars(&mut a, &mut b).expect("rear-end");
        assert_eq!(contact.closing_speed, Fixed::from_int(3));

        // Momentum is conserved (equal masses) to rounding.
        let after = momentum(&a, &b);
        assert!((after.x - before.x).raw().abs() <= 4);
        assert!((after.y - before.y).raw().abs() <= 4);

        // b is shoved forward (north), a is slowed but not reversed.
        assert!(b.velocity.y < Fixed::from_int(-1));
        assert!(a.velocity.y < Fixed::ZERO);
        assert!(a.velocity.y > Fixed::from_int(-2));
        // No longer approaching.
        assert!((b.velocity - a.velocity).dot(contact.normal) >= Fixed::ZERO);
        // Speeds resynchronised.
        assert_eq!(a.speed, a.velocity.length());
        assert_eq!(b.speed, b.velocity.length());
        assert!(contact.closing_speed > HARD_BUMP_THRESHOLD);
    }

    #[test]
    fn separating_cars_are_pushed_apart_but_not_pulled_back() {
        let mut a = with_velocity(car_at(100, 100, 0), Fixed::from_int(-1), Fixed::ZERO);
        let mut b = with_velocity(car_at(110, 100, 0), Fixed::from_int(1), Fixed::ZERO);
        let (va, vb) = (a.velocity, b.velocity);
        let contact = collide_cars(&mut a, &mut b).expect("overlapping");
        assert_eq!(contact.closing_speed, Fixed::ZERO);
        assert_eq!(a.velocity, va);
        assert_eq!(b.velocity, vb);
        assert!(body_gap_raw(&a, &b) >= 0);
    }

    #[test]
    fn car_contact_never_spins_a_car_out() {
        // A head-on at a combined 7 u/tick: far beyond the barrier threshold.
        let mut a = with_velocity(car_at(100, 100, 0), Fixed::ZERO, Fixed::from_raw(-14000));
        let mut b = with_velocity(car_at(100, 79, 2048), Fixed::ZERO, Fixed::from_raw(14000));
        collide_cars(&mut a, &mut b).expect("head-on");
        assert!(!a.drift.is_spinning());
        assert!(!b.drift.is_spinning());
    }

    #[test]
    fn broadphase_is_total_for_extreme_positions() {
        let mut a = car_at(0, 0, 0);
        let mut b = car_at(0, 0, 0);
        a.position = Vec2::new(Fixed::from_raw(i32::MIN), Fixed::from_raw(i32::MIN));
        b.position = Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MAX));
        // Must not panic; saturating subtraction keeps the delta huge.
        assert!(collide_cars(&mut a, &mut b).is_none());
    }

    fn field_on(track: &TrackDef) -> (VehicleState, [AiRacer; 5]) {
        let grid: [(i32, i32); 5] = [(-48, 16), (-96, -16), (-144, 16), (-192, -16), (-240, 0)];
        let player = VehicleState::new(track.start_pos, track.start_heading, CarTuning::default());
        let mut rivals = [
            AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[0]),
            AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[1]),
            AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[2]),
            AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[3]),
            AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[4]),
        ];
        for (rival, (fwd, lat)) in rivals.iter_mut().zip(grid) {
            rival.offset_from_pole(track.start_pos, track.start_heading, fwd, lat);
        }
        (player, rivals)
    }

    #[test]
    fn the_starting_grid_is_contact_free() {
        // Cars parked on their grid slots must not shove each other before the
        // lights go out.
        for track in ALL_TRACKS.iter() {
            let (mut player, mut rivals) = field_on(track);
            let contacts = resolve_field(&mut player, &mut rivals, track);
            assert_eq!(contacts.contact_count, 0, "{}: grid overlaps", track.name);
        }
    }

    #[test]
    fn a_finished_rival_is_retired_from_contact() {
        let track = ALL_TRACKS[0];
        let (mut player, mut rivals) = field_on(track);
        rivals[0].state.position = player.position;
        rivals[0].is_finished = true;
        let before = rivals[0].state;
        let contacts = resolve_field(&mut player, &mut rivals, track);
        assert!(!contacts.player_touched);
        assert_eq!(rivals[0].state, before);
    }

    #[test]
    fn player_contact_is_reported_with_its_impact() {
        let track = ALL_TRACKS[0];
        let (mut player, mut rivals) = field_on(track);
        // Rival 0 parked just ahead of the player, player closing at 2 u/tick.
        let ahead = player.forward_dir();
        rivals[0].state.position = player.position + ahead.scale(Fixed::from_int(20));
        rivals[0].state.heading = player.heading;
        rivals[0].state.visual_angle = player.visual_angle;
        player.velocity = ahead.scale(Fixed::from_int(2));
        player.speed = player.velocity.length();
        let contacts = resolve_field(&mut player, &mut rivals, track);
        assert!(contacts.player_touched);
        assert!(contacts.player_hard_hit());
        assert!(contacts.player_impact > Fixed::from_raw(8000));
        // The normal points from the player towards the rival (forward).
        assert!(contacts.player_contact_normal.dot(ahead) > Fixed::from_raw(3500));
        assert!(rivals[0].state.velocity.dot(ahead) > Fixed::ONE);
    }

    /// A whole pack racing together, with contact on, on several circuits:
    /// nobody may be left overlapping, nobody may end a tick inside a wall, and
    /// the rivals must still get round the circuit -- contact must not turn the
    /// field into a permanent pile-up.
    #[test]
    fn a_full_pack_races_with_contact_and_keeps_moving() {
        for track in ALL_TRACKS.iter().take(4) {
            let (_, mut rivals) = field_on(track);
            // The "player" is a sixth AI so the whole field is driven.
            let mut player_ai = AiRacer::new(track.start_pos, track.start_heading, AI_PROFILES[2]);
            let mut total_contacts = 0u32;
            for _ in 0..3600 {
                let mut positions = [Vec2::ZERO; 6];
                positions[0] = player_ai.state.position;
                for i in 0..5 {
                    positions[i + 1] = rivals[i].state.position;
                }
                player_ai.tick(track, &positions);
                for rival in rivals.iter_mut() {
                    rival.tick(track, &positions);
                }
                let contacts = resolve_field(&mut player_ai.state, &mut rivals, track);
                total_contacts += contacts.contact_count as u32;

                let all: [&VehicleState; 6] = [
                    &player_ai.state,
                    &rivals[0].state,
                    &rivals[1].state,
                    &rivals[2].state,
                    &rivals[3].state,
                    &rivals[4].state,
                ];
                for car in all {
                    let tx = TrackDef::tile_x_of(car.position.x);
                    let ty = TrackDef::tile_y_of(car.position.y);
                    assert!(
                        !track.tile_at(tx, ty).is_solid(),
                        "{}: a car ended a tick inside a wall",
                        track.name
                    );
                }
            }
            // Contact actually happened somewhere in the pack: a test that never
            // exercises the solver proves nothing.
            assert!(
                total_contacts > 0,
                "{}: the pack never touched -- test exercises nothing",
                track.name
            );
            for (i, rival) in rivals.iter().enumerate() {
                assert!(
                    rival.current_lap > 1 || rival.target_gate_idx > 2,
                    "{}: rival {} ({}) made no progress with contact on",
                    track.name,
                    i,
                    rival.profile.name
                );
            }
        }
    }
}
