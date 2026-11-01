//! The floating-point kernel the ISA needs and `core` does not provide.
//!
//! `docs/isa.md` §4.2–§4.5 fixes the semantics; this module is the only place
//! allowed to implement them. Two rules shape everything here:
//!
//! * **No libm.** `FSIN`/`FCOS` must be bit-identical on every platform, and
//!   `sinf` differs in the last bits between libm versions. More basically,
//!   `#![no_std]` has no `f32::sin`, `f32::sqrt`, or `f32::floor` at all —
//!   `core` only carries the unstable `core::f32::math` free functions.
//! * **No `mul_add`.** A hardware FMA and a soft-float FMA can differ, so every
//!   expression here is spelled as separate multiply and add. Rust does not
//!   contract them; nothing here gives it the option.
//!
//! Determinism comes from the *sequence of IEEE operations*, not from the
//! precision of the approximation: the same operations in the same order land on
//! the same bits everywhere. Accuracy is a separate, weaker promise, stated per
//! function.

use core::f32;
/// `FNEG` — flips the sign bit. Exact for NaN and both zeros (`isa.md` §4.3).
#[inline]
pub fn fneg(x: f32) -> f32 {
    f32::from_bits(x.to_bits() ^ 0x8000_0000)
}

/// `FABS` — clears the sign bit. Exact for NaN (`isa.md` §4.3).
#[inline]
pub fn fabs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

/// `FMIN` — IEEE 754-2019 `minimum`: NaN propagates, `-0.0` is below `+0.0`.
///
/// `f32::min` is deliberately *not* used: that is `minimumNumber`, which ignores
/// a NaN operand and returns the other one — the opposite of what §4.2 says.
#[inline]
pub fn fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        // Only reachable for equal values and for ±0; the bitwise OR keeps the
        // negative zero when either input has it.
        f32::from_bits(a.to_bits() | b.to_bits())
    } else if a < b {
        a
    } else {
        b
    }
}

/// `FMAX` — IEEE 754-2019 `maximum`: NaN propagates, `+0.0` is above `-0.0`.
#[inline]
pub fn fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == b {
        // The bitwise AND keeps the positive zero when either input has it.
        f32::from_bits(a.to_bits() & b.to_bits())
    } else if a > b {
        a
    } else {
        b
    }
}

/// `FMADD` — two roundings, never fused (`isa.md` §4.4).
#[inline]
pub fn madd(a: f32, b: f32, c: f32) -> f32 {
    let product = a * b;
    product + c
}

/// `FLERP` — the evaluation order is part of the spec: subtract, multiply, add.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    let span = b - a;
    let scaled = span * t;
    a + scaled
}

/// `FCLAMP` — defined as `fmin(fmax(x, lo), hi)`, so `lo > hi` yields `hi`.
#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    fmin(fmax(x, lo), hi)
}

// ---------------------------------------------------------------------------
// Rounding
// ---------------------------------------------------------------------------

/// `1.0 - 2^-24`, the largest f32 below one: `next_down(1.0)`.
pub const ONE_MINUS_ULP: f32 = f32::from_bits(0x3f7f_ffff);

/// The next f32 above `x` (`f32::next_up` is 1.86; this crate's MSRV is 1.85).
#[inline]
pub fn next_up(x: f32) -> f32 {
    let bits = x.to_bits();
    if x.is_nan() || bits == f32::INFINITY.to_bits() {
        return x;
    }
    let magnitude = bits & 0x7fff_ffff;
    if magnitude == 0 {
        // ±0.0 -> the smallest positive subnormal.
        f32::from_bits(1)
    } else if bits == magnitude {
        f32::from_bits(bits + 1)
    } else {
        f32::from_bits(bits - 1)
    }
}

/// The next f32 below `x`.
#[inline]
pub fn next_down(x: f32) -> f32 {
    let bits = x.to_bits();
    if x.is_nan() || bits == f32::NEG_INFINITY.to_bits() {
        return x;
    }
    let magnitude = bits & 0x7fff_ffff;
    if magnitude == 0 {
        f32::from_bits(0x8000_0001)
    } else if bits == magnitude {
        f32::from_bits(bits - 1)
    } else {
        f32::from_bits(bits + 1)
    }
}

/// `floor` for f32, exact, without libm.
///
/// Every case is exact by construction: the mantissa bits below the binary point
/// are masked off, and a negative value with a non-zero fraction is stepped down
/// by one (which is exactly representable). `|x| >= 2^23` is already integral,
/// and NaN / infinities pass through untouched.
pub fn floor(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32;

    if exponent >= 150 {
        // 2^23 and above: no fractional part. Also catches Inf and NaN.
        return x;
    }
    if exponent < 127 {
        // |x| < 1.
        return if x == 0.0 {
            x
        } else if bits >> 31 == 1 {
            -1.0
        } else {
            0.0
        };
    }

    let integral = bits & (u32::MAX << (150 - exponent));
    if bits >> 31 == 1 && integral != bits {
        // Negative with a non-zero fraction: truncation went the wrong way.
        f32::from_bits(integral) - 1.0
    } else {
        f32::from_bits(integral)
    }
}

/// `FFRAC` — `x - floor(x)`, clamped to `[0, ONE_MINUS_ULP]` (`isa.md` §4.3).
///
/// The clamp is not cosmetic: for `x = -1e-30` the exact result is `1 - 1e-30`,
/// which rounds to exactly `1.0` in f32. A phase accumulator that receives `1.0`
/// stops wrapping. The exact difference is strictly below one, so rounding can
/// reach `1.0` but never exceed it — one comparison suffices.
#[inline]
pub fn frac(x: f32) -> f32 {
    let result = x - floor(x);
    if result == 1.0 {
        ONE_MINUS_ULP
    } else {
        result
    }
}

// ---------------------------------------------------------------------------
// Square root
// ---------------------------------------------------------------------------

/// `FSQRT` — the correctly rounded IEEE-754 binary32 square root.
///
/// The seed is computed in f64 (where every non-zero f32 — subnormals included —
/// is a normal number), refined by four Newton steps, and then **corrected
/// exactly**: `r²` for an f32 `r` is a 48-bit product and therefore exact in f64,
/// as are `4x` and `(r + next(r))²`. That makes the final choice — round down,
/// round up, or break a tie to even — a decision on exact integers, not an
/// approximation.
pub fn sqrt(x: f32) -> f32 {
    if x.is_nan() {
        return f32::NAN;
    }
    if x == 0.0 {
        return x; // +0.0 -> +0.0, -0.0 -> -0.0
    }
    if x < 0.0 {
        return f32::NAN; // includes -inf
    }
    if x == f32::INFINITY {
        return x;
    }

    let exact = x as f64;

    // Halve the exponent as a first guess, then let Newton fix it.
    let mut guess = f64::from_bits(0x1ff8_0000_0000_0000 + (exact.to_bits() >> 1));
    for _ in 0..4 {
        guess = 0.5 * (guess + exact / guess);
    }

    let square = |v: f32| (v as f64) * (v as f64);

    // `lo` becomes the largest f32 whose square is <= x.
    let mut lo = guess as f32;
    while square(lo) > exact {
        lo = next_down(lo);
    }
    let mut hi = next_up(lo);
    while square(hi) <= exact {
        lo = hi;
        hi = next_up(hi);
    }

    // lo² <= x < hi². Round to nearest; a tie goes to the even significand.
    let midpoint = lo as f64 + hi as f64; // exact: 25 significant bits
    let twice = midpoint * midpoint; // exact: 50 significant bits
    let scaled = 4.0 * exact; // exact: power-of-two scale
    if scaled > twice {
        hi
    } else if scaled < twice {
        lo
    } else if lo.to_bits() & 1 == 0 {
        lo
    } else {
        hi
    }
}

// ---------------------------------------------------------------------------
// Sine and cosine
// ---------------------------------------------------------------------------

/// `2/π`, used to pick the quadrant.
const TWO_OVER_PI: f64 = 0.636_619_772_367_581_3;

/// π/2 in three pieces (Cody–Waite). The sum carries about 100 bits, so the
/// reduction error stays below an f32 ULP well past `|x| = 2^20`.
const PIO2_HI: f64 = 1.570_796_326_734_125_6e0;
const PIO2_MID: f64 = 6.077_100_506_303_966e-11;
const PIO2_LO: f64 = 2.022_266_248_795_950_6e-21;

/// The largest reduction quotient that still fits `i64` with margin.
const MAX_QUOTIENT: f64 = 9.2e18;

/// fdlibm's `__kernel_sin` coefficients, evaluated in f64.
fn sin_kernel(x: f64) -> f64 {
    const S1: f64 = -1.666_666_666_666_663_2e-01;
    const S2: f64 = 8.333_333_333_322_489e-03;
    const S3: f64 = -1.984_126_982_985_795e-04;
    const S4: f64 = 2.755_731_370_707_006_8e-06;
    const S5: f64 = -2.505_076_025_340_686_3e-08;
    const S6: f64 = 1.589_690_995_211_55e-10;

    let z = x * x;
    let v = z * x;
    let tail = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    x + v * (S1 + z * tail)
}

/// fdlibm's `__kernel_cos` coefficients, evaluated in f64.
fn cos_kernel(x: f64) -> f64 {
    const C1: f64 = 4.166_666_666_666_602e-02;
    const C2: f64 = -1.388_888_888_887_411e-03;
    const C3: f64 = 2.480_158_728_947_673e-05;
    const C4: f64 = -2.755_731_435_139_066_4e-07;
    const C5: f64 = 2.087_572_321_298_174_8e-09;
    const C6: f64 = -1.135_964_755_778_819_4e-11;

    let z = x * x;
    // `z * (…)` is already one factor of z² in fdlibm's arrangement; the caller
    // multiplies it by z once more (`z * r`), which is where the z² comes from.
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    1.0 - (0.5 * z - z * r)
}

/// Reduce `x` to `r ∈ [-π/4, π/4]` with quadrant `k`, or `None` when the quotient
/// does not fit an `i64` — the point past which no meaningful reduction exists.
fn reduce(x: f64) -> Option<(f64, i64)> {
    // Round half away from zero, then truncate: `as i64` truncates toward zero,
    // so the ±0.5 first is exactly `round`.
    let biased = x * TWO_OVER_PI + f64::copysign(0.5, x);
    if !(biased > -MAX_QUOTIENT && biased < MAX_QUOTIENT) {
        return None;
    }
    let k = biased as i64;
    let kf = k as f64;
    let r = ((x - kf * PIO2_HI) - kf * PIO2_MID) - kf * PIO2_LO;
    Some((r, k))
}

/// Clamp to `[-1, 1]`, leaving NaN alone.
#[inline]
fn unit(v: f64) -> f32 {
    let v = v as f32;
    if v > 1.0 {
        1.0
    } else if v < -1.0 {
        -1.0
    } else {
        v
    }
}

/// `FSIN`. Non-finite inputs and inputs too large to reduce return NaN
/// (`isa.md` §4.3).
pub fn sin(x: f32) -> f32 {
    if !x.is_finite() {
        return f32::NAN;
    }
    let Some((r, quadrant)) = reduce(x as f64) else {
        return f32::NAN;
    };
    let value = match quadrant & 3 {
        0 => sin_kernel(r),
        1 => cos_kernel(r),
        2 => -sin_kernel(r),
        _ => -cos_kernel(r),
    };
    unit(value)
}

/// `FCOS`. Same domain and limits as [`sin`].
pub fn cos(x: f32) -> f32 {
    if !x.is_finite() {
        return f32::NAN;
    }
    let Some((r, quadrant)) = reduce(x as f64) else {
        return f32::NAN;
    };
    let value = match quadrant & 3 {
        0 => cos_kernel(r),
        1 => -sin_kernel(r),
        2 => -cos_kernel(r),
        _ => sin_kernel(r),
    };
    unit(value)
}
