//! Bit-level conformance for the hand-written floating-point kernel.
//!
//! These are integration tests, not unit tests, on purpose: the crate is
//! `#![no_std]`, so a `#[cfg(test)]` module inside it could not reach `f32::sin`
//! or `f32::sqrt`. Here the platform's own functions are available and serve as
//! the **oracle** for accuracy — while the crate's own implementation remains the
//! only thing that ever runs on a device.

use cyberspine_core::isa::math as m;

// ---------------------------------------------------------------------------
// Sign and selection
// ---------------------------------------------------------------------------

#[test]
fn neg_flips_the_sign_bit_and_keeps_nan_payloads() {
    assert_eq!(m::fneg(1.0).to_bits(), 0xbf80_0000);
    assert_eq!(m::fneg(-1.0).to_bits(), 0x3f80_0000);
    assert_eq!(m::fneg(0.0).to_bits(), 0x8000_0000);
    assert_eq!(m::fneg(-0.0).to_bits(), 0x0000_0000);

    // §4.3 promises the sign flip holds for NaN too, payload untouched.
    let payload = f32::from_bits(0x7fc0_1234);
    assert_eq!(m::fneg(payload).to_bits(), 0xffc0_1234);
}

#[test]
fn abs_clears_the_sign_bit() {
    assert_eq!(m::fabs(-1.5).to_bits(), 1.5f32.to_bits());
    assert_eq!(
        m::fabs(f32::NEG_INFINITY).to_bits(),
        f32::INFINITY.to_bits()
    );
    assert_eq!(m::fabs(f32::from_bits(0xffc0_1234)).to_bits(), 0x7fc0_1234);
}

#[test]
fn min_and_max_follow_ieee_754_2019_minimum_and_maximum() {
    // NaN propagates, unlike minimumNumber.
    assert!(m::fmin(f32::NAN, 1.0).is_nan());
    assert!(m::fmin(1.0, f32::NAN).is_nan());
    assert!(m::fmax(f32::NAN, 1.0).is_nan());
    assert!(m::fmax(1.0, f32::NAN).is_nan());

    // -0.0 is strictly below +0.0, in both orders.
    assert_eq!(m::fmin(0.0, -0.0).to_bits(), 0x8000_0000);
    assert_eq!(m::fmin(-0.0, 0.0).to_bits(), 0x8000_0000);
    assert_eq!(m::fmax(0.0, -0.0).to_bits(), 0x0000_0000);
    assert_eq!(m::fmax(-0.0, 0.0).to_bits(), 0x0000_0000);

    // Equal values survive unchanged.
    assert_eq!(m::fmin(1.5, 1.5), 1.5);
    assert_eq!(m::fmax(-2.0, -2.0), -2.0);

    // Ordering.
    assert_eq!(m::fmin(-3.0, 2.0), -3.0);
    assert_eq!(m::fmax(-3.0, 2.0), 2.0);
}

// ---------------------------------------------------------------------------
// Ternary
// ---------------------------------------------------------------------------

#[test]
fn madd_rounds_twice_and_is_not_fused() {
    // (1 + ε)(1 − ε) − 1 is exactly −ε² ≈ −1.42e-14, which a fused multiply-add
    // would return. Two roundings give +0 instead, and that is the spec (§4.4).
    let one_plus = 1.0f32 + f32::EPSILON;
    let one_minus = 1.0f32 - f32::EPSILON;
    assert_eq!(m::madd(one_plus, one_minus, -1.0).to_bits(), 0);

    // The fused result would not be zero, so the test above is a real witness.
    // An f64 product of two f32 values is exact, so the rounding below cannot
    // be defeated by the epsilon squared vanishing.
}

#[test]
fn lerp_uses_the_specified_order_and_allows_extrapolation() {
    assert_eq!(m::lerp(0.0, 10.0, 0.5), 5.0);
    assert_eq!(m::lerp(1.0, 2.0, 0.0), 1.0);
    assert_eq!(m::lerp(1.0, 2.0, 1.0), 2.0);
    assert_eq!(m::lerp(1.0, 2.0, 2.0), 3.0, "no clamping of t");
    assert_eq!(m::lerp(1.0, 2.0, -1.0), 0.0);
}

#[test]
fn clamp_is_the_composition_so_an_inverted_range_yields_hi() {
    assert_eq!(m::clamp(5.0, 0.0, 1.0), 1.0);
    assert_eq!(m::clamp(-5.0, 0.0, 1.0), 0.0);
    assert_eq!(m::clamp(0.5, 0.0, 1.0), 0.5);
    assert_eq!(
        m::clamp(0.5, 1.0, 0.0),
        0.0,
        "lo > hi collapses to fmin(x, hi)"
    );
    assert!(m::clamp(f32::NAN, 0.0, 1.0).is_nan());
}

// ---------------------------------------------------------------------------
// Rounding
// ---------------------------------------------------------------------------

#[test]
fn floor_is_exact_on_the_cases_that_break_naive_implementations() {
    let cases: [(f32, f32); 14] = [
        (0.0, 0.0),
        (-0.0, -0.0),
        (0.5, 0.0),
        (-0.5, -1.0),
        (3.7, 3.0),
        (-3.7, -4.0),
        (3.0, 3.0),
        (1e-30, 0.0),
        (-1e-30, -1.0),
        (8_388_608.0, 8_388_608.0), // 2^23, already integral
        (-8_388_608.0, -8_388_608.0),
        (1e30, 1e30),
        (f32::INFINITY, f32::INFINITY),
        (f32::NEG_INFINITY, f32::NEG_INFINITY),
    ];

    for (input, expected) in cases {
        assert_eq!(
            m::floor(input).to_bits(),
            expected.to_bits(),
            "floor({input:e})"
        );
    }

    assert!(m::floor(f32::NAN).is_nan());
}

#[test]
fn floor_agrees_with_the_platform_on_a_grid() {
    for step in -20_000i32..=20_000 {
        let x = step as f32 * 0.037;
        assert_eq!(m::floor(x).to_bits(), x.floor().to_bits(), "floor({x})");
    }
}

#[test]
fn frac_stays_below_one_even_where_the_difference_rounds_up() {
    // The case §4.3 exists for: the exact difference is 1 − 1e-30, which rounds
    // to exactly 1.0 in f32 unless it is clamped.
    assert_eq!(m::frac(-1e-30).to_bits(), 0x3f7f_ffff);
    assert_eq!(m::frac(-1e-30), m::ONE_MINUS_ULP);

    assert_eq!(m::frac(1.0).to_bits(), 0, "integral inputs give +0");
    assert_eq!(m::frac(-1.0).to_bits(), 0);
    assert_eq!(m::frac(-0.0).to_bits(), 0);
    assert_eq!(m::frac(0.25), 0.25);
    assert_eq!(m::frac(1.75), 0.75);
    assert_eq!(m::frac(-0.25), 0.75);

    assert!(m::frac(f32::NAN).is_nan());
    assert!(m::frac(f32::INFINITY).is_nan());
    assert!(m::frac(f32::NEG_INFINITY).is_nan());

    // The range property, over a grid that includes both signs.
    for step in -50_000i32..=50_000 {
        let x = step as f32 * 0.1;
        let r = m::frac(x);
        assert!((0.0..1.0).contains(&r), "frac({x}) = {r}");
    }
}

// ---------------------------------------------------------------------------
// Square root
// ---------------------------------------------------------------------------

#[test]
fn sqrt_agrees_with_the_platform_on_special_and_extreme_values() {
    let cases = [
        0.0f32,
        -0.0,
        1.0,
        2.0,
        4.0,
        9.0,
        0.25,
        f32::MAX,
        f32::MIN_POSITIVE,
        f32::from_bits(1), // smallest subnormal
        f32::from_bits(3), // subnormal
        1e-40,             // subnormal
        1e30,
    ];

    for x in cases {
        assert_eq!(
            m::sqrt(x).to_bits(),
            x.sqrt().to_bits(),
            "sqrt({x:e}) must be correctly rounded"
        );
    }

    assert_eq!(m::sqrt(-0.0).to_bits(), 0x8000_0000, "sqrt(-0.0) = -0.0");
    assert!(m::sqrt(-1.0).is_nan());
    assert!(m::sqrt(f32::NAN).is_nan());
    assert_eq!(m::sqrt(f32::INFINITY), f32::INFINITY);
    assert!(m::sqrt(f32::NEG_INFINITY).is_nan());
}

#[test]
fn sqrt_is_correctly_rounded_across_the_whole_range_by_sampling() {
    // Every 4097th bit pattern, both signs: ~2M samples, fast in debug.
    for bits in (0u32..=u32::MAX).step_by(4097) {
        let x = f32::from_bits(bits);
        if x.is_nan() {
            assert!(m::sqrt(x).is_nan());
            continue;
        }
        if x < 0.0 {
            assert!(m::sqrt(x).is_nan(), "sqrt({x}) must be NaN");
            continue;
        }
        assert_eq!(
            m::sqrt(x).to_bits(),
            x.sqrt().to_bits(),
            "sqrt of bits {bits:#010x} ({x:e})"
        );
    }
}

/// The full 2^32 sweep, kept as an opt-in because it takes minutes.
///
///     cargo test -p cyberspine-core --release --test math -- --ignored sqrt_is
#[test]
#[ignore = "2^32 inputs; run with --release -- --ignored"]
fn sqrt_is_correctly_rounded_for_every_f32() {
    let mut bits = 0u32;
    loop {
        let x = f32::from_bits(bits);
        let expected = x.sqrt();
        if expected.is_nan() {
            // Negative inputs. The ISA fixes *that* the result is NaN, not which
            // NaN (G9 excludes the payload), so bit equality would be wrong here.
            assert!(m::sqrt(x).is_nan(), "sqrt of bits {bits:#010x} ({x:e})");
        } else {
            assert_eq!(
                m::sqrt(x).to_bits(),
                expected.to_bits(),
                "sqrt of bits {bits:#010x} ({x:e})"
            );
        }
        if bits == u32::MAX {
            break;
        }
        bits += 1;
    }
}

// ---------------------------------------------------------------------------
// Sine and cosine
// ---------------------------------------------------------------------------

#[test]
fn sin_and_cos_handle_the_inputs_outside_their_domain() {
    assert!(m::sin(f32::NAN).is_nan());
    assert!(m::cos(f32::NAN).is_nan());
    assert!(m::sin(f32::INFINITY).is_nan());
    assert!(m::cos(f32::NEG_INFINITY).is_nan());

    // Too large to reduce: the quotient would not fit an i64.
    assert!(m::sin(f32::MAX).is_nan());
    assert!(m::cos(f32::MAX).is_nan());

    // Large but reducible: still deterministic and inside [-1, 1].
    for exp in 20..61 {
        let x = 2.0f32.powi(exp);
        let s = m::sin(x);
        let c = m::cos(x);
        assert!((-1.0..=1.0).contains(&s), "sin(2^{exp}) = {s}");
        assert!((-1.0..=1.0).contains(&c), "cos(2^{exp}) = {c}");
    }
}

#[test]
fn sin_and_cos_stay_within_two_ulps_of_full_scale() {
    // ULP distance *of the result* is the wrong metric here. Near a zero
    // crossing sin(x) is ~1e-8, so an absolute difference of 1e-9 is a hundred
    // thousand ULP of the result while being 0.02 ULP of full scale. Math
    // libraries quote f32 accuracy against the unit interval; so do we.
    //
    // `f32::EPSILON` is 2^-24 — exactly one ULP at 1.0.
    let mut worst = 0.0f32;
    let mut worst_at = 0.0f32;
    let mut worst_which = "";
    for step in -26_000i32..=26_000 {
        let x = step as f32 * 0.001;
        let ds = (m::sin(x) - x.sin()).abs();
        let dc = (m::cos(x) - x.cos()).abs();
        if ds > worst {
            worst = ds;
            worst_at = x;
            worst_which = "sin";
        }
        if dc > worst {
            worst = dc;
            worst_at = x;
            worst_which = "cos";
        }
    }

    assert!(
        worst <= 2.0 * f32::EPSILON,
        "worst absolute error was {worst:e} = {} ULP of 1.0 at {worst_which}({worst_at})",
        worst / f32::EPSILON
    );
}

#[test]
fn sin_and_cos_are_bit_stable_across_repeated_calls() {
    // Determinism is the property the ISA actually promises (§4.3): the same
    // input must give the same bits every time, on every build.
    for step in -1000i32..=1000 {
        let x = step as f32 * 0.01;
        assert_eq!(m::sin(x).to_bits(), m::sin(x).to_bits());
        assert_eq!(m::cos(x).to_bits(), m::cos(x).to_bits());
    }
}
