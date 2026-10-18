use cyberspine_vm::register::{RegId, RegType};
use thiserror::Error;
use winnow::{
    ModalResult, Parser,
    ascii::{digit1, space0},
    combinator::alt,
};

use crate::assembler::Line;

pub fn reg(input: &mut Line<'_>) -> ModalResult<(RegId, RegType)> {
    let reg_type = alt(('f', 'i')).map(map_reg_type).parse_next(input)?;
    let unchecked_reg_id = digit1.parse_to().parse_next(input)?;
    let reg_id = map_reg_id(unchecked_reg_id).unwrap(); //TODO: enhance the error handling later.
    Ok((reg_id, reg_type))
}

pub fn operand_delimiter(input: &mut Line<'_>) -> ModalResult<()> {
    (space0, ',', space0).void().parse_next(input)
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum RegParsingError {
    #[error("Unknown register type '{0}'.")]
    UnknownType(char),

    #[error("Register id out of range '{0}'.")]
    IdOutofRange(u32),
}

fn map_reg_id(id: u32) -> Result<RegId, RegParsingError> {
    if id > 63 {
        Err(RegParsingError::IdOutofRange(id))
    } else {
        Ok(RegId::new(id as u8).unwrap())
    }
}

fn map_reg_type(c: char) -> RegType {
    match c {
        'f' => RegType::Float,
        'i' => RegType::Int,
        _ => unreachable!(
            "winnow doesn't accept chars other than f or i, so the map won't encouter it."
        ),
    }
}
