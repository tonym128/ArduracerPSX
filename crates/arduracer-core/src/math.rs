//! Fixed-point arithmetic and 2D vector mathematics for Arduracer PSX.
//!
//! Uses Q20.12 fixed point format (1.0 = 4096), directly matching the
//! PlayStation GTE coprocessor's fixed-point conventions.

/// The fixed-point representation of 1.0 (2^12 = 4096).
pub const FP_ONE: i32 = 4096;

/// Half of 1.0 in fixed point (2048).
pub const FP_HALF: i32 = 2048;

/// Number of fractional bits in Q20.12.
pub const FP_SHIFT: u32 = 12;

/// Angular circle constant: 4096 units = 360 degrees (matches GTE).
pub const ANGLE_360: u16 = 4096;
pub const ANGLE_180: u16 = 2048;
pub const ANGLE_90: u16 = 1024;
pub const ANGLE_45: u16 = 512;

/// Fixed-point value in Q20.12 format.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fixed(pub i32);

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);
    pub const ONE: Fixed = Fixed(FP_ONE);
    pub const HALF: Fixed = Fixed(FP_HALF);

    #[inline]
    pub const fn from_int(v: i32) -> Self {
        Fixed(v << FP_SHIFT)
    }

    #[inline]
    pub const fn to_int(self) -> i32 {
        self.0 >> FP_SHIFT
    }

    #[inline]
    pub const fn from_raw(raw: i32) -> Self {
        Fixed(raw)
    }

    #[inline]
    pub const fn raw(self) -> i32 {
        self.0
    }

    #[inline]
    pub fn abs(self) -> Self {
        // `i32::abs` panics on i32::MIN in debug builds.
        Fixed(self.0.saturating_abs())
    }

    /// Multiplies by a fixed-point factor (readability helper for `a * b`).
    #[inline]
    pub fn scale(self, factor: Fixed) -> Self {
        self * factor
    }

    #[inline]
    pub fn clamp(self, min_val: Fixed, max_val: Fixed) -> Self {
        if self.0 < min_val.0 {
            min_val
        } else if self.0 > max_val.0 {
            max_val
        } else {
            self
        }
    }
}

/// Clamps a 64-bit intermediate into the representable `Fixed` range.
///
/// A bare `as i32` truncates, which turns any product or quotient larger than
/// `i32::MAX` into a large *negative* number and silently poisons every value
/// derived from it. Saturating matches the behaviour of `Add`/`Sub`.
#[inline]
const fn sat_i32(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

impl core::ops::Mul for Fixed {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let prod = (self.0 as i64 * rhs.0 as i64) >> FP_SHIFT;
        Fixed(sat_i32(prod))
    }
}

impl core::ops::Div for Fixed {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        if rhs.0 == 0 {
            // Saturate to the representable extremes rather than producing a
            // wrapped value. Callers that treat the quotient as a scale factor
            // must still guard against a zero divisor.
            return if self.0 >= 0 {
                Fixed(i32::MAX)
            } else {
                Fixed(i32::MIN)
            };
        }
        let quot = ((self.0 as i64) << FP_SHIFT) / (rhs.0 as i64);
        Fixed(sat_i32(quot))
    }
}

impl core::ops::Add for Fixed {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Fixed(self.0.saturating_add(rhs.0))
    }
}

impl core::ops::Sub for Fixed {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Fixed(self.0.saturating_sub(rhs.0))
    }
}

impl core::ops::Neg for Fixed {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Fixed(self.0.saturating_neg())
    }
}

/// 2D Vector with Fixed point coordinates.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Vec2 {
    pub x: Fixed,
    pub y: Fixed,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
    };

    #[inline]
    pub const fn new(x: Fixed, y: Fixed) -> Self {
        Vec2 { x, y }
    }

    #[inline]
    pub fn dot(self, rhs: Vec2) -> Fixed {
        self.x * rhs.x + self.y * rhs.y
    }

    /// Squared magnitude as a `Fixed`. Saturates for large separations; use
    /// [`Vec2::length`] when an exact magnitude is required.
    #[inline]
    pub fn length_squared(self) -> Fixed {
        self.dot(self)
    }

    /// Euclidean length of the vector.
    ///
    /// The squared magnitude is accumulated in 64 bits from the raw Q20.12
    /// components rather than through `Fixed` multiplication: the largest
    /// legitimate world separation is a full 30x30-tile circuit (1920 units =
    /// 7_864_320 raw), whose square overflows `i32` and used to wrap negative,
    /// making `length()` report `0` for every separation of roughly 800 units or
    /// more. That value is the AI's only range measurement.
    #[inline]
    pub fn length(self) -> Fixed {
        let (x, y) = (self.x.0, self.y.0);
        let (ax, ay) = (x.unsigned_abs() as u64, y.unsigned_abs() as u64);
        // Max 2 * (2^31)^2 == 2^63, which fits in u64 with room to spare.
        let sq = ax * ax + ay * ay;
        Fixed(sat_i32(isqrt_u64(sq) as i64))
    }

    #[inline]
    pub fn scale(self, factor: Fixed) -> Vec2 {
        Vec2 {
            x: self.x * factor,
            y: self.y * factor,
        }
    }
}

impl core::ops::Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Vec2 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl core::ops::Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Vec2 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

/// 256-entry Q1.12 sine table for high-performance trigonometry.
pub const SIN_TABLE: [i16; 256] = [
    0, 101, 201, 301, 401, 501, 601, 700, 799, 897, 995, 1092, 1189, 1285, 1380, 1474, 1567, 1660,
    1751, 1842, 1931, 2019, 2106, 2191, 2276, 2359, 2440, 2520, 2598, 2675, 2751, 2824, 2896, 2967,
    3035, 3102, 3166, 3229, 3290, 3349, 3406, 3461, 3513, 3564, 3612, 3659, 3703, 3745, 3784, 3822,
    3857, 3889, 3920, 3948, 3973, 3996, 4017, 4036, 4052, 4065, 4076, 4085, 4091, 4095, 4096, 4095,
    4091, 4085, 4076, 4065, 4052, 4036, 4017, 3996, 3973, 3948, 3920, 3889, 3857, 3822, 3784, 3745,
    3703, 3659, 3612, 3564, 3513, 3461, 3406, 3349, 3290, 3229, 3166, 3102, 3035, 2967, 2896, 2824,
    2751, 2675, 2598, 2520, 2440, 2359, 2276, 2191, 2106, 2019, 1931, 1842, 1751, 1660, 1567, 1474,
    1380, 1285, 1189, 1092, 995, 897, 799, 700, 601, 501, 401, 301, 201, 101, 0, -101, -201, -301,
    -401, -501, -601, -700, -799, -897, -995, -1092, -1189, -1285, -1380, -1474, -1567, -1660,
    -1751, -1842, -1931, -2019, -2106, -2191, -2276, -2359, -2440, -2520, -2598, -2675, -2751,
    -2824, -2896, -2967, -3035, -3102, -3166, -3229, -3290, -3349, -3406, -3461, -3513, -3564,
    -3612, -3659, -3703, -3745, -3784, -3822, -3857, -3889, -3920, -3948, -3973, -3996, -4017,
    -4036, -4052, -4065, -4076, -4085, -4091, -4095, -4096, -4095, -4091, -4085, -4076, -4065,
    -4052, -4036, -4017, -3996, -3973, -3948, -3920, -3889, -3857, -3822, -3784, -3745, -3703,
    -3659, -3612, -3564, -3513, -3461, -3406, -3349, -3290, -3229, -3166, -3102, -3035, -2967,
    -2896, -2824, -2751, -2675, -2598, -2520, -2440, -2359, -2276, -2191, -2106, -2019, -1931,
    -1842, -1751, -1660, -1567, -1474, -1380, -1285, -1189, -1092, -995, -897, -799, -700, -601,
    -501, -401, -301, -201, -101,
];

/// Sine of angle (0..4096 = 0..360 deg) as Fixed Q20.12.
#[inline]
pub fn sin(angle: u16) -> Fixed {
    let a = angle & 0x0FFF;
    let idx = (a >> 4) as usize;
    let frac = (a & 0x000F) as i32;
    let v0 = SIN_TABLE[idx] as i32;
    let v1 = SIN_TABLE[(idx + 1) & 0xFF] as i32;
    Fixed(v0 + (((v1 - v0) * frac) >> 4))
}

/// Cosine of angle (0..4096 = 0..360 deg) as Fixed Q20.12.
#[inline]
pub fn cos(angle: u16) -> Fixed {
    sin(angle.wrapping_add(ANGLE_90))
}

/// Integer square root for a 64-bit unsigned integer.
///
/// Newton's method. Inputs up to `2^63` are supported, which covers the sum of
/// squares of any pair of `i32` components.
#[inline]
fn isqrt_u64(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x0 = n >> 1;
    if x0 == 0 {
        return 1;
    }
    let mut x1 = (x0 + n / x0) >> 1;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + n / x0) >> 1;
    }
    x0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest legal world separation: a 30x30-tile circuit is 1920 units.
    const BIG: i32 = 1920;

    fn v(x: i32, y: i32) -> Vec2 {
        Vec2::new(Fixed::from_int(x), Fixed::from_int(y))
    }

    #[test]
    fn mul_saturates_instead_of_wrapping_negative() {
        // The product exceeds i32::MAX. A truncating `as i32` cast turned this
        // into a large negative number and poisoned every derived value.
        let big = Fixed::from_raw(7_864_320);
        let product = big * big;
        assert!(
            product.raw() > 0,
            "mul overflowed into a negative value: {}",
            product.raw()
        );
        assert_eq!(product.raw(), i32::MAX, "must clamp to i32::MAX");
    }

    #[test]
    fn mul_never_inverts_sign() {
        let samples = [1i32, 2, 7, 100, 4096, 12_345, 524_287];
        for &a in &samples {
            for &b in &samples {
                let product = Fixed::from_int(a) * Fixed::from_int(b);
                assert!(
                    product.raw() >= 0,
                    "{a} * {b} produced a negative product ({})",
                    product.raw()
                );
            }
        }
        assert!((Fixed::from_int(-3) * Fixed::from_int(4)).raw() < 0);
    }

    #[test]
    fn mul_stays_exact_in_normal_range() {
        let half = Fixed::from_int(6) * Fixed::HALF;
        assert_eq!(half.raw(), 3 * FP_ONE);
        assert_eq!((Fixed::from_int(4) * Fixed::from_int(4)).raw(), 16 * FP_ONE);
    }

    #[test]
    fn div_saturates_instead_of_wrapping_negative() {
        // 1000 / (1/4096) is a genuine 4_096_000, which does not fit in i32.
        // A truncating cast made it negative -- a negative scale factor from a
        // positive-over-positive division.
        let quotient = Fixed::from_int(1000) / Fixed::from_raw(1);
        assert!(
            quotient.raw() > 0,
            "div overflowed into a negative value: {}",
            quotient.raw()
        );
        assert_eq!(quotient.raw(), i32::MAX);
    }

    #[test]
    fn div_by_zero_saturates_to_extremes() {
        assert_eq!((Fixed::ONE / Fixed::ZERO).raw(), i32::MAX);
        assert_eq!((Fixed::from_int(-1) / Fixed::ZERO).raw(), i32::MIN);
    }

    #[test]
    fn div_is_exact_in_normal_range() {
        assert_eq!(
            (Fixed::from_int(10) / Fixed::from_int(4)).raw(),
            2 * FP_ONE + FP_ONE / 2
        );
        assert_eq!((Fixed::ONE / Fixed::from_int(4)).raw(), FP_ONE / 4);
    }

    #[test]
    fn add_and_sub_saturate() {
        let max = Fixed::from_raw(i32::MAX);
        assert_eq!((max + Fixed::ONE).raw(), i32::MAX);
        assert_eq!((Fixed::from_raw(i32::MIN) - Fixed::ONE).raw(), i32::MIN);
    }

    #[test]
    fn abs_does_not_overflow() {
        // `i32::abs` panics on i32::MIN in debug builds.
        let min = Fixed::from_raw(i32::MIN).abs();
        assert_eq!(min.raw(), i32::MAX);
        assert_eq!(Fixed::from_int(-5).abs().raw(), 5 * FP_ONE);
    }

    #[test]
    fn neg_does_not_overflow() {
        // `-i32::MIN` panics in debug builds.
        let _ = -Fixed::from_raw(i32::MIN);
        assert_eq!((-Fixed::from_int(5)).raw(), -5 * FP_ONE);
    }

    #[test]
    fn length_is_exact_at_track_scale() {
        // Every one of these used to wrap: 800/1024/1920 reported 0.00 and
        // 1500 reported 390.96. This is the AI's only range measurement.
        for d in [1i32, 100, 400, 724, 800, 1024, 1500, 1900, BIG] {
            assert_eq!(
                v(d, 0).length().raw(),
                d * FP_ONE,
                "length({d}, 0) was wrong"
            );
            assert_eq!(v(0, d).length().raw(), d * FP_ONE);
        }
    }

    #[test]
    fn length_handles_two_axis_magnitudes() {
        // 1900^2 + 1900^2 == 2687.0056..., so the exact integer root of the raw
        // components lands 23 raw units above 2687.0. Previously this reported
        // 724.08 -- the wrapped i32 length of one axis minus the other.
        let diagonal = v(1900, 1900).length().raw();
        let expected = 2687 * FP_ONE;
        assert!(
            (diagonal - expected).abs() <= 32,
            "length(1900,1900) = {diagonal}, expected ~{expected}"
        );
        assert_eq!(v(3, 4).length().raw(), 5 * FP_ONE);
    }

    #[test]
    fn length_of_zero_is_zero() {
        assert_eq!(Vec2::ZERO.length().raw(), 0);
        assert_eq!(v(0, 0).length().raw(), 0);
    }

    #[test]
    fn length_is_never_larger_than_the_largest_axis() {
        for d in [0i32, 1, 64, 640, 1920] {
            let length = v(d, d).length();
            assert!(
                length.raw() >= d * FP_ONE && length.raw() <= 2 * d * FP_ONE,
                "length({d},{d}) out of range: {}",
                length.raw()
            );
        }
    }

    #[test]
    fn length_saturates_rather_than_wrapping_at_the_extremes() {
        let huge = Vec2::new(Fixed::from_raw(i32::MAX), Fixed::from_raw(i32::MAX));
        assert!(huge.length().raw() > 0);
        let negative = Vec2::new(Fixed::from_raw(i32::MIN), Fixed::from_raw(i32::MIN));
        assert!(negative.length().raw() > 0);
    }

    #[test]
    fn dot_and_scale_stay_exact_in_normal_range() {
        assert_eq!(v(3, 4).dot(v(5, 6)).raw(), 39 * FP_ONE);
        assert_eq!(v(2, 3).scale(Fixed::HALF).x.raw(), FP_ONE);
    }

    #[test]
    fn sin_and_cover_the_cardinal_angles() {
        assert_eq!(sin(0).raw(), 0);
        assert_eq!(sin(ANGLE_90).raw(), FP_ONE);
        assert_eq!(cos(0).raw(), FP_ONE);
        assert_eq!(cos(ANGLE_90).raw(), 0);
        assert_eq!(sin(ANGLE_180).raw(), 0);
    }

    #[test]
    fn from_int_and_to_int_round_trip() {
        for v in [0i32, 1, -1, 1920, -1920, 524_287] {
            assert_eq!(Fixed::from_int(v).to_int(), v);
        }
    }
}
