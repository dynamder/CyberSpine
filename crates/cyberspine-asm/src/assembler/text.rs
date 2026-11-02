pub mod imm;
pub mod insn;
pub mod reg;

use cyberspine_core::isa::{
    OpCode, RegName,
    register::{RegId, RegType},
};
use smol_str::SmolStr;
use winnow::{
    Parser,
    ascii::space0,
    combinator::{opt, preceded, repeat, terminated},
};

use crate::assembler::{
    Line, comments,
    diagnostic::{
        AsmParseErrors, PResult, ParseErrorKind, cut_at, into_syntax, snippet, token_snippet,
    },
    ident, is_removed_directive, is_reserved_name,
    text::insn::insn,
};

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

fn label(input: &mut Line<'_>) -> PResult<TextItem> {
    let start = *input;
    let name = terminated(ident, ':').parse_next(input)?;
    if is_reserved_name(name) {
        // Hard error on purpose: if this backtracked, `text_stmt` would retry the
        // same text as a mnemonic and report "unknown mnemonic" instead.
        return cut_at(start, ParseErrorKind::ReservedName(SmolStr::new(name)));
    }
    Ok(TextItem::Label(SmolStr::new(name)))
}

fn text_stmt(input: &mut Line<'_>) -> PResult<Vec<TextItem>> {
    // A failed `label` backtracks, so `L_ok: FMOVI …` leaves the instruction for
    // the `insn` branch below instead of treating the mnemonic as a label.
    let labels: Vec<TextItem> = repeat(0.., preceded(space0, label)).parse_next(input)?;

    space0.parse_next(input)?;

    let parsed_insn = opt(insn).parse_next(input)?;
    match parsed_insn {
        Some(insn) => {
            finish_text_line(input, true)?;
            let mut items = labels;
            items.push(TextItem::Insn(insn));
            Ok(items)
        }
        None => {
            finish_text_line(input, false)?;
            Ok(labels)
        }
    }
}

/// After the labels and the optional instruction, only whitespace and a
/// trailing comment may remain. Everything else gets a rule-specific error:
/// a declaration in the instruction section, trailing garbage after a real
/// instruction, or a line that is neither a label nor an instruction.
fn finish_text_line(input: &mut Line<'_>, had_insn: bool) -> PResult<()> {
    space0.parse_next(input)?;

    if input.is_empty() {
        return Ok(());
    }
    if input.starts_with('#') {
        return comments.parse_next(input);
    }

    let start = *input;
    if let Some(rest) = start.strip_prefix('.') {
        let name = directive_name(rest);
        let kind = if is_removed_directive(&name) {
            ParseErrorKind::RemovedDirective(name)
        } else {
            ParseErrorKind::LateDirective(name)
        };
        return cut_at(start, kind);
    }

    let kind = if had_insn {
        ParseErrorKind::TrailingTokens(snippet(start))
    } else {
        ParseErrorKind::ExpectedInstruction(token_snippet(start))
    };
    cut_at(start, kind)
}

fn directive_name(rest: &str) -> SmolStr {
    let mut cursor = rest;
    match ident(&mut cursor) {
        Ok(name) => SmolStr::new(name),
        Err(_) => SmolStr::new(""),
    }
}

pub fn text(input: &mut &str) -> Result<Vec<TextItem>, AsmParseErrors> {
    let src = *input;
    let mut text_items = Vec::new();
    let mut errors = Vec::new();

    for (index, line) in src.lines().enumerate() {
        let mut cursor = line;
        match text_stmt(&mut cursor) {
            Ok(line_items) => text_items.extend(line_items),
            Err(err) => errors.push(into_syntax(err).located(line, index + 1)),
        }
    }

    // The instruction section is consumed in full; leaving the cursor on the last
    // line would let a caller parse it twice.
    *input = &src[src.len()..];

    if errors.is_empty() {
        Ok(text_items)
    } else {
        errors.sort_by_key(|error| (error.location.line, error.location.column));
        Err(AsmParseErrors::new(errors))
    }
}
