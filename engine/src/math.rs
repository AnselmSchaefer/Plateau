//! The engine's own transcendental functions (`WORLD_PLAN.md`, phase D).
//!
//! Rust's `sin`, `atan2`, `powf`, `log10` and the like call the platform's maths library, whose
//! last bits differ between machines and builds. Anything the engine stores, saves, or will one day
//! agree on with a server (where a player walks, how fast they grow, where an object is turned to)
//! uses these instead: additions, multiplications, divisions, `floor` and bit manipulation only, so
//! the same inputs give the same bits everywhere. The trigonometry is the SDK's (`plateau_sdk::sin`,
//! `atan2`), so areas and the engine agree. Only `picture` (images for people) may use the library.

pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;
const LN_2: f32 = std::f32::consts::LN_2;
const LOG10_2: f32 = std::f32::consts::LOG10_2;

/// The sine (as `plateau_sdk::sin`): to within about a ten-millionth near nought, a few millionths
/// at forty radians, as its argument's own rounding allows.
#[inline]
pub fn sin(x: f32) -> f32 {
    // Into -pi..pi, then -pi/2..pi/2 by symmetry, then the series to x^11.
    let turns = x * (1.0 / TAU);
    let mut r = (turns - (turns + 0.5).floor()) * TAU;
    if r > PI / 2.0 {
        r = PI - r;
    } else if r < -PI / 2.0 {
        r = -PI - r;
    }
    let r2 = r * r;
    r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0 + r2 * (-1.0 / 39_916_800.0))))))
}

/// The cosine (as `plateau_sdk::cos`).
#[inline]
pub fn cos(x: f32) -> f32 {
    sin(x + PI / 2.0)
}

/// The angle of `(x, y)` from the x axis, from -pi to pi, to within about a hundred-thousandth (as
/// `plateau_sdk::atan2`).
pub fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (abs(x), abs(y));
    if ax == 0.0 && ay == 0.0 {
        return 0.0;
    }
    let z = if ax < ay { ax / ay } else { ay / ax };
    let z2 = z * z;
    let mut a = z * (0.999_866 + z2 * (-0.330_299_5 + z2 * (0.180_141 + z2 * (-0.085_133 + z2 * 0.020_835_1))));
    if ay > ax {
        a = PI / 2.0 - a;
    }
    if x < 0.0 {
        a = PI - a;
    }
    if y < 0.0 { -a } else { a }
}

/// The base-2 logarithm of a positive, finite, normal `x`, to within a few parts in ten million.
/// Exact for powers of two.
pub fn log2(x: f32) -> f32 {
    if !(x > 0.0) || !x.is_finite() {
        return f32::NAN;
    }
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xff) as i32 - 127;
    // The mantissa in [1, 2), then in [sqrt(1/2), sqrt(2)) so the series below converges fast.
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > std::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    // ln m = 2 atanh s, s = (m - 1) / (m + 1), |s| < 0.172.
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let ln_m = 2.0 * s * (1.0 + s2 * (1.0 / 3.0 + s2 * (1.0 / 5.0 + s2 * (1.0 / 7.0 + s2 * (1.0 / 9.0)))));
    e as f32 + ln_m * (1.0 / LN_2)
}

/// The base-10 logarithm of a positive, finite, normal `x`.
#[inline]
pub fn log10(x: f32) -> f32 {
    log2(x) * LOG10_2
}

/// Two to the power `y`, to within a few parts in ten million; 0 below `2^-126` and infinity above
/// `2^128`. Exact for whole `y`.
pub fn exp2(y: f32) -> f32 {
    if y.is_nan() {
        return f32::NAN;
    }
    if y >= 128.0 {
        return f32::INFINITY;
    }
    if y < -126.0 {
        return 0.0;
    }
    let k = (y + 0.5).floor();
    let f = (y - k) * LN_2; // |f| <= ln 2 / 2
    let frac = 1.0 + f * (1.0 + f * (1.0 / 2.0 + f * (1.0 / 6.0 + f * (1.0 / 24.0 + f * (1.0 / 120.0 + f * (1.0 / 720.0 + f * (1.0 / 5040.0)))))));
    let k = k as i32;
    if k >= 128 {
        // `frac < 1` here (y just under 128): 2^127 * (2 frac).
        return f32::from_bits(254 << 23) * (2.0 * frac);
    }
    frac * f32::from_bits(((k + 127) as u32) << 23)
}

/// `base` to the power `t`, for a positive, finite, normal `base`. `pow(b, 0) == 1` and
/// `pow(b, 1) == b` exactly.
pub fn pow(base: f32, t: f32) -> f32 {
    if t == 0.0 {
        return 1.0;
    }
    if t == 1.0 {
        return base;
    }
    exp2(t * log2(base))
}

#[inline]
fn abs(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrees_with_the_library_closely() {
        let mut worst = [0.0f32; 5];
        for i in 0..20_000 {
            let x = -40.0 + i as f32 * 0.004;
            worst[0] = worst[0].max((sin(x) - x.sin()).abs());
            worst[1] = worst[1].max((atan2(x, 3.0 - x * 0.1) - x.atan2(3.0 - x * 0.1)).abs());
            let p = 1e-3 + i as f32 * 0.05;
            worst[2] = worst[2].max(((log2(p) - p.log2()) / p.log2().abs().max(1.0)).abs());
            let y = -30.0 + i as f32 * 0.003;
            worst[3] = worst[3].max(((exp2(y) - y.exp2()) / y.exp2()).abs());
            let b = 0.01 + i as f32 * 0.005;
            let t = (i % 97) as f32 / 96.0;
            worst[4] = worst[4].max(((pow(b, t) - b.powf(t)) / b.powf(t)).abs());
        }
        assert!(worst[0] < 5e-6, "sin {worst:?}");
        assert!(worst[1] < 2e-5, "atan2 {worst:?}");
        assert!(worst[2] < 1e-6, "log2 {worst:?}");
        assert!(worst[3] < 1e-6, "exp2 {worst:?}");
        assert!(worst[4] < 2e-6, "pow {worst:?}");
    }

    #[test]
    fn is_exact_where_it_should_be() {
        for e in -20..20 {
            let x = (e as f32).exp2();
            assert_eq!(log2(x), e as f32);
            assert_eq!(exp2(e as f32), x);
        }
        assert_eq!(pow(0.017, 1.0), 0.017);
        assert_eq!(pow(17.0, 0.0), 1.0);
        assert_eq!(log10(1000.0) - 3.0 < 1e-6, true);
    }
}
