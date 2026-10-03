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
        Fixed(self.0.abs())
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

impl core::ops::Mul for Fixed {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let prod = (self.0 as i64 * rhs.0 as i64) >> FP_SHIFT;
        Fixed(prod as i32)
    }
}

impl core::ops::Div for Fixed {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        if rhs.0 == 0 {
            return if self.0 >= 0 {
                Fixed(i32::MAX)
            } else {
                Fixed(i32::MIN)
            };
        }
        let quot = ((self.0 as i64) << FP_SHIFT) / (rhs.0 as i64);
        Fixed(quot as i32)
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
        Fixed(-self.0)
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

    #[inline]
    pub fn length_squared(self) -> Fixed {
        self.dot(self)
    }

    /// Fast integer square root for fixed point length.
    pub fn length(self) -> Fixed {
        let sq = self.length_squared().raw();
        if sq <= 0 {
            return Fixed::ZERO;
        }
        Fixed(isqrt64((sq as i64) << FP_SHIFT) as i32)
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

/// Integer square root for 64-bit integer.
#[inline]
fn isqrt64(n: i64) -> i64 {
    if n <= 0 {
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
