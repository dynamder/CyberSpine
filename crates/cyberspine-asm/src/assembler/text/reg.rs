use cyberspine_core::isa::register::{RegId, RegType};
use winnow::{Parser, ascii::digit1, combinator::alt};

use crate::assembler::{
    Line,
    diagnostic::{PResult, ParseErrorKind, cut_at},
};

pub(crate) fn reg(input: &mut Line<'_>) -> PResult<(RegId, RegType)> {
    let start = *input;
    let reg_type = alt(('f', 'i')).map(map_reg_type).parse_next(input)?;
    let index: u32 = digit1.parse_to().parse_next(input)?;

    if index >= RegId::COUNT as u32 {
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

    // `index < COUNT` was checked above, so the checked constructor cannot fail.
    Ok((RegId::new(index as u8).expect("index < COUNT"), reg_type))
}

fn map_reg_type(c: char) -> RegType {
    // `alt(('f', 'i'))` only ever yields these two characters.
    if c == 'f' {
        RegType::Float
    } else {
        RegType::Int
    }
}
