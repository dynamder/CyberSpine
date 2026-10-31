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
    combinator::{eof, opt, preceded, repeat, terminated},
    error::{ContextError, ErrMode},
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

/// `docs/asm.md` §1 / §6.8: `fN`/`iN` are the register shape and `dt`/`tick` are
/// built-in sensor names, so none of them may be used as a label.
fn is_reserved_name(name: &str) -> bool {
    if matches!(name, "dt" | "tick") {
        return true;
    }
    let mut chars = name.chars();
    if !matches!(chars.next(), Some('f' | 'i')) {
        return false;
    }
    let rest = chars.as_str();
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn label(input: &mut Line<'_>) -> ModalResult<TextItem> {
    let name = terminated(ident, ':').parse_next(input)?;
    if is_reserved_name(name) {
        // Hard error on purpose: if this backtracked, `text_stmt` would retry the
        // same text as a mnemonic and the mnemonic lookup would panic.
        return Err(ErrMode::Cut(ContextError::new()));
    }
    Ok(TextItem::Label(SmolStr::new(name)))
}

fn text_stmt(input: &mut Line<'_>) -> ModalResult<Vec<TextItem>> {
    // A failed `label` backtracks, so `L_ok: FMOVI …` leaves the instruction for
    // the `insn` branch below instead of treating the mnemonic as a label.
    let labels: Vec<TextItem> = repeat(0.., preceded(space0, label)).parse_next(input)?;

    space0.void().parse_next(input)?;

    let insn = opt(insn.map(TextItem::Insn)).parse_next(input)?;

    Ok(labels.into_iter().chain(insn).collect())
}

pub fn text(input: &mut &str) -> ModalResult<Vec<TextItem>> {
    let src: &str = *input;
    let mut text_items = Vec::new();
    for line in src.lines() {
        let mut line = line;
        let line_items = text_stmt.parse_next(&mut line)?;
        (space0, opt(comments), eof).void().parse_next(&mut line)?;
        text_items.extend(line_items);
    }
    // The instruction section is consumed in full; leaving the cursor on the last
    // line would let a caller parse it twice.
    *input = &src[src.len()..];

    Ok(text_items)
}
