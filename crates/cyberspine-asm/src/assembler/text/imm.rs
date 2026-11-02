use smol_str::SmolStr;
use winnow::{Parser, error::ErrMode, token::take_while};

use crate::assembler::{
    Line,
    diagnostic::{PResult, ParseErrorKind, SyntaxError},
    ident,
};

/// One operand-sized lexeme: everything up to whitespace, a comma, a comment or
/// the end of the line. Parsing the token whole (instead of letting each numeric
/// parser decide where it stops) is what keeps `1abc`, `0x` and `1.0.0` from
/// being accepted as prefixes.
fn numeric_token<'i>(input: &mut Line<'i>) -> PResult<&'i str> {
    take_while(1.., |c: char| {
        !matches!(c, ' ' | '\t' | '\r' | '\n' | ',' | '#')
    })
    .parse_next(input)
}

/// Splits a leading `+`/`-` off, returning `(negative, body)`.
fn split_sign(token: &str) -> (bool, &str) {
    match token.as_bytes().first() {
        Some(b'-') => (true, &token[1..]),
        Some(b'+') => (false, &token[1..]),
        _ => (false, token),
    }
}

pub(crate) fn imm_int(input: &mut Line<'_>) -> PResult<i32> {
    let token = numeric_token.parse_next(input)?;
    parse_int(token)
        .ok_or_else(|| ErrMode::Backtrack(SyntaxError::at(input, ParseErrorKind::Unexpected)))
}

/// `docs/asm.md` §1 / §4.1:
/// - decimal is signed and must fit `i32` (`i32::MIN` included);
/// - `0x`/`0o`/`0b` are **u32 bit patterns** reinterpreted as `i32`;
/// - `_` may separate digits (`1_000_000`, `0xFF_FF`).
fn parse_int(token: &str) -> Option<i32> {
    let (negative, body) = split_sign(token);
    if body.is_empty() {
        return None;
    }

    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = body.strip_prefix(prefix) {
            let digits = strip_separators(digits)?;
            let bits = u32::from_str_radix(&digits, radix).ok()?;
            let value = bits as i32;
            return Some(if negative {
                value.wrapping_neg()
            } else {
                value
            });
        }
    }

    let body = strip_separators(body)?;
    let magnitude: i64 = body.parse().ok()?;
    let signed = if negative { -magnitude } else { magnitude };
    i32::try_from(signed).ok()
}

/// Removes `_` separators, rejecting a leading, trailing or doubled one.
fn strip_separators(body: &str) -> Option<String> {
    if body.is_empty() {
        return Some(String::new());
    }

    let mut out = String::with_capacity(body.len());
    let mut previous_underscore = true;

    for c in body.chars() {
        if c == '_' {
            if previous_underscore {
                return None;
            }
            previous_underscore = true;
        } else {
            out.push(c);
            previous_underscore = false;
        }
    }

    if previous_underscore {
        return None;
    }
    Some(out)
}

pub(crate) fn imm_float(input: &mut Line<'_>) -> PResult<f32> {
    let token = numeric_token.parse_next(input)?;
    parse_float(token)
        .ok_or_else(|| ErrMode::Backtrack(SyntaxError::at(input, ParseErrorKind::Unexpected)))
}

/// `docs/asm.md` §4.1: decimal literals go through `f32::from_str` exactly once
/// (no f64 detour), while `0x…` is the raw `imm` bit pattern.
fn parse_float(token: &str) -> Option<f32> {
    let (negative, body) = split_sign(token);

    // `docs/asm.md` §1: `_` separates digits in *integer* literals only. Refused
    // explicitly rather than left to `f32::from_str`, so the rule is a property
    // of the language instead of an accident of the parser underneath.
    if body.contains('_') {
        return None;
    }

    for prefix in ["0x", "0X"] {
        if let Some(digits) = body.strip_prefix(prefix) {
            let bits = u32::from_str_radix(digits, 16).ok()?;
            let value = f32::from_bits(bits);
            return Some(if negative { -value } else { value });
        }
    }

    token.parse::<f32>().ok()
}

pub(crate) fn imm_ident(input: &mut Line<'_>) -> PResult<SmolStr> {
    ident.map(SmolStr::new).parse_next(input)
}
