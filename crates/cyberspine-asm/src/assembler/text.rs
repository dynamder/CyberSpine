pub mod imm;
pub mod insn;
pub mod reg;

use cyberspine_vm::{
    register::{RegId, RegType},
    vm::op::{OpCode, RegName},
};
use smol_str::SmolStr;
use winnow::{
    ModalResult, Parser,
    ascii::space0,
    combinator::{eof, opt, repeat, terminated},
};

use crate::assembler::{Line, comments, ident, text::insn::insn};

#[derive(Debug, Clone, PartialEq)]
pub enum AsmOperand {
    Reg {
        id: RegId,
        name: RegName,
        reg_type: RegType,
    },
    ImmF32(f32),
    ImmI32(i32),
    ImmNone,
    Index(SmolStr),
    Target(SmolStr),
}

impl AsmOperand {
    pub fn new_reg(id: RegId, name: RegName, reg_type: RegType) -> Self {
        Self::Reg { id, name, reg_type }
    }

    pub fn new_imm_f32(v: f32) -> Self {
        Self::ImmF32(v)
    }

    pub fn new_imm_i32(v: i32) -> Self {
        Self::ImmI32(v)
    }

    pub fn new_index(ident: SmolStr) -> Self {
        Self::Index(ident)
    }

    pub fn new_target(label: SmolStr) -> Self {
        Self::Target(label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsmInsn {
    pub op: OpCode,
    pub operands: Vec<AsmOperand>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextItem {
    Label(SmolStr),
    Insn(AsmInsn),
}

fn label(input: &mut Line<'_>) -> ModalResult<TextItem> {
    terminated(ident, ':')
        .map(|label| TextItem::Label(SmolStr::new(label)))
        .parse_next(input)
}

fn text_stmt(input: &mut Line<'_>) -> ModalResult<Vec<TextItem>> {
    let label: Vec<TextItem> = repeat(0.., label).parse_next(input)?;

    space0.void().parse_next(input)?;

    let insn = opt(insn.map(TextItem::Insn)).parse_next(input)?;

    Ok(label.into_iter().chain(insn.into_iter()).collect())
}

pub fn text(input: &mut &str) -> ModalResult<Vec<TextItem>> {
    let mut text_items = Vec::new();
    for mut line in input.lines() {
        let line_text_item = text_stmt.parse_next(&mut line)?;
        (space0, opt(comments), eof).void().parse_next(&mut line)?;
        text_items.extend(line_text_item);
    }

    Ok(text_items)
}
