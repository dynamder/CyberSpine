use smol_str::SmolStr;
use winnow::{
    ModalResult, Parser,
    ascii::{dec_int, float},
    combinator::{alt, cut_err, not, opt, preceded, terminated},
    stream::AsChar,
    token::{one_of, take_while},
};

use crate::assembler::{Line, ident};

pub fn imm_int(input: &mut Line<'_>) -> ModalResult<i32> {
    let negative = opt(one_of(['+', '-']))
        .map(|c| c == Some('-'))
        .parse_next(input)?;

    let value = alt((
        prefixed_int,
        //decimal int, can't have ., e, E, it will be handled by the next float parser.
        terminated(cut_err(dec_int), not(one_of(['.', 'e', 'E']))),
    ))
    .parse_next(input)?;

    Ok(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

pub fn imm_float(input: &mut Line<'_>) -> ModalResult<f32> {
    float.parse_next(input)
}

pub fn imm_ident(input: &mut Line<'_>) -> ModalResult<SmolStr> {
    ident.map(SmolStr::new).parse_next(input)
}

fn radix_digits(input: &mut Line<'_>, radix: u32) -> ModalResult<u32> {
    let is_radix_digit = |c: char, radix: u32| match radix {
        2 => c.is_digit(2),
        8 => AsChar::is_oct_digit(c),
        16 => AsChar::is_hex_digit(c),
        _ => unreachable!(),
    };
    take_while(1.., |c: char| is_radix_digit(c, radix))
        .verify_map(move |s: &str| u32::from_str_radix(s, radix).ok())
        .parse_next(input)
}

fn hex_int(input: &mut Line<'_>) -> ModalResult<i32> {
    radix_digits(input, 16).map(|v| v as i32)
}

fn oct_int(input: &mut Line<'_>) -> ModalResult<i32> {
    radix_digits(input, 8).map(|v| v as i32)
}

fn bin_int(input: &mut Line<'_>) -> ModalResult<i32> {
    radix_digits(input, 2).map(|v| v as i32)
}

fn prefixed_int(input: &mut Line<'_>) -> ModalResult<i32> {
    alt((
        preceded(alt(("0b", "0B")), cut_err(bin_int)),
        preceded(alt(("0o", "0O")), cut_err(oct_int)),
        preceded(alt(("0x", "0X")), cut_err(hex_int)),
    ))
    .parse_next(input)
}
