use cyberspine_vm::register::{RegId, RegType};
use winnow::{Parser, ascii::digit1, combinator::alt};

use crate::assembler::{
    Line,
    diagnostic::{PResult, ParseErrorKind, cut_at},
};

pub(crate) fn reg(input: &mut Line<'_>) -> PResult<(RegId, RegType)> {
    let start = *input;
    let reg_type = alt(('f', 'i')).map(map_reg_type).parse_next(input)?;
    let index: u32 = digit1.parse_to().parse_next(input)?;

    if index >= 64 {
        return cut_at(
            start,
            ParseErrorKind::RegisterOutOfRange {
                prefix: match reg_type {
                    RegType::Float => 'f',
                    RegType::Int => 'i',
                },
                index,
            },
        );
    }

    // `index < 64` was checked above, so the 6-bit constructor cannot fail.
    Ok((RegId::from_6bit(index as u8), reg_type))
}

fn map_reg_type(c: char) -> RegType {
    // `alt(('f', 'i'))` only ever yields these two characters.
    if c == 'f' {
        RegType::Float
    } else {
        RegType::Int
    }
}
