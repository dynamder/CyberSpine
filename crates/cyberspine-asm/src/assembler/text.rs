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
    combinator::{opt, terminated},
    error::ErrMode,
};

use crate::assembler::{
    Line, comments,
    diagnostic::{
        AsmParseErrors, Location, PResult, ParseErrorKind, cut_at, into_syntax, snippet,
        token_snippet,
    },
    ident, is_reserved_name,
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
    /// A quoted reason template, interned into the image's reason table.
    Text(SmolStr),
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

    pub fn new_text(text: SmolStr) -> Self {
        Self::Text(text)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsmInsn {
    pub op: OpCode,
    pub operands: Vec<AsmOperand>,
}

/// One item of the instruction section, pinned to where it came from.
///
/// The line is filled in by [`text`] (the parser sees one line at a time); the
/// column is already exact. This is what lets the generation stage report
/// `path:line:col` for problems that are not parse errors: an out-of-range
/// jump, an unknown name, a `FEXCITE` that was never flushed.
#[derive(Debug, Clone, PartialEq)]
pub struct Located<T> {
    pub item: T,
    pub location: Location,
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

/// The 1-based column of `input` within `line`, given that `input` is a suffix
/// of it.
fn column(line: &str, input: &str) -> usize {
    let consumed = line.len() - input.len();
    line[..consumed].chars().count() + 1
}

fn text_stmt(line: &str, input: &mut Line<'_>) -> PResult<Vec<Located<TextItem>>> {
    let mut items = Vec::new();

    // A failed `label` backtracks, so `L_ok: FMOVI …` leaves the instruction for
    // the `insn` branch below instead of treating the mnemonic as a label. The
    // cursor is restored before each try, so the column is the item's own.
    loop {
        let before = *input;
        space0.parse_next(input)?;
        let at_label = *input;
        match label.parse_next(input) {
            Ok(label) => items.push(Located {
                item: label,
                location: Location::new(0, column(line, at_label)),
            }),
            // A reserved name is a hard error (`docs/asm.md` §1), not "this line
            // has no label": backtracking here would report the same text as an
            // unknown mnemonic instead.
            Err(ErrMode::Cut(error)) => return Err(ErrMode::Cut(error)),
            Err(_) => {
                *input = before;
                break;
            }
        }
    }

    space0.parse_next(input)?;

    let before = *input;
    let parsed_insn = opt(insn).parse_next(input)?;
    match parsed_insn {
        Some(insn) => {
            finish_text_line(input, true)?;
            items.push(Located {
                item: TextItem::Insn(insn),
                location: Location::new(0, column(line, before)),
            });
            Ok(items)
        }
        None => {
            finish_text_line(input, false)?;
            Ok(items)
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
        return cut_at(start, ParseErrorKind::LateDirective(directive_name(rest)));
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

/// A double-quoted string. Only `\\` and `\"` are escapes; a backslash before
/// anything else is reported, so a Windows path has to be written `\\`.
///
/// Shared by `.rig` and the `INTR` reason operand (`docs/asm.md` §1): one
/// string lexer, so a reason and a manifest path cannot disagree about escapes.
pub(crate) fn quoted_string<'i>(input: &mut Line<'i>) -> PResult<SmolStr> {
    let text = *input;
    let Some(rest) = text.strip_prefix('"') else {
        return cut_at(text, ParseErrorKind::Unexpected);
    };

    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((offset, character)) = chars.next() {
        match character {
            '"' => {
                *input = &rest[offset + 1..];
                return Ok(SmolStr::new(out));
            }
            '\\' => match chars.next() {
                Some((_, escaped @ ('\\' | '"'))) => out.push(escaped),
                _ => return cut_at(&rest[offset..], ParseErrorKind::Unexpected),
            },
            other => out.push(other),
        }
    }
    cut_at(text, ParseErrorKind::UnterminatedString)
}

pub fn text(input: &mut &str) -> Result<Vec<Located<TextItem>>, AsmParseErrors> {
    let src = *input;
    let mut text_items = Vec::new();
    let mut errors = Vec::new();

    for (index, line) in src.lines().enumerate() {
        let mut cursor = line;
        match text_stmt(line, &mut cursor) {
            Ok(line_items) => {
                for mut located in line_items {
                    located.location.line = index + 1;
                    text_items.push(located);
                }
            }
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
