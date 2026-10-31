use std::str::FromStr;

use cyberspine_core::{
    register::RegType,
    vm::op::{ImmType, OpCode, OperandParam},
};
use smol_str::SmolStr;
use winnow::{Parser, ascii::space0, error::ErrMode};

use crate::assembler::{
    Line,
    diagnostic::{PResult, ParseErrorKind, SyntaxError, cut_at, token_snippet},
    ident,
    text::{
        AsmInsn, AsmOperand,
        imm::{imm_float, imm_ident, imm_int},
        reg::reg,
    },
};

pub(crate) fn operand_delimiter(input: &mut Line<'_>) -> PResult<()> {
    (space0, ',', space0).void().parse_next(input)
}

fn opcode(input: &mut Line<'_>) -> PResult<OpCode> {
    let start = *input;
    let mnemonic = ident.parse_next(input)?;
    match OpCode::from_str(mnemonic) {
        Ok(op) => Ok(op),
        Err(_) => cut_at(
            start,
            ParseErrorKind::UnknownMnemonic(SmolStr::new(mnemonic)),
        ),
    }
}

pub(crate) fn operand(input: &mut Line<'_>, sig: OperandParam) -> PResult<AsmOperand> {
    match sig {
        OperandParam::Reg(name, expected) => {
            let start = *input;
            let (id, reg_type) = reg.parse_next(input)?;
            if reg_type != expected {
                return cut_at(
                    start,
                    ParseErrorKind::RegisterFileMismatch {
                        expected: reg_file_label(expected),
                        found: reg_file_label(reg_type),
                    },
                );
            }
            Ok(AsmOperand::new_reg(id, name, reg_type))
        }
        OperandParam::Imm(imm_type) => match imm_type {
            ImmType::F32 => imm_float.map(AsmOperand::new_imm_f32).parse_next(input),
            ImmType::I32 => imm_int.map(AsmOperand::new_imm_i32).parse_next(input),
            ImmType::Index => imm_ident.map(AsmOperand::new_index).parse_next(input),
            ImmType::JumpOffset => imm_ident.map(AsmOperand::new_target).parse_next(input),
            ImmType::None => Ok(AsmOperand::ImmNone),
        },
    }
}

pub(crate) fn insn(input: &mut Line<'_>) -> PResult<AsmInsn> {
    let opcode = opcode.parse_next(input)?;
    let signature = opcode.signature();

    space0.parse_next(input)?;

    let mut operands = Vec::with_capacity(signature.len());
    for (index, sig) in signature.iter().enumerate() {
        // Commas separate operands; there is no comma *after* the last one.
        if index != 0 {
            let before = *input;
            if at_line_end(before) {
                // The line ended (possibly after trailing spaces or a comment):
                // the comma is not the problem, the operand is simply absent.
                return cut_at(
                    "",
                    ParseErrorKind::MissingOperand {
                        mnemonic: opcode.mnemonic(),
                        index: index + 1,
                        arity: signature.len(),
                    },
                );
            }
            if operand_delimiter.parse_next(input).is_err() {
                return cut_at(
                    before,
                    ParseErrorKind::MissingComma {
                        mnemonic: opcode.mnemonic(),
                        index: index + 1,
                    },
                );
            }
        }

        let before = *input;
        match operand(input, *sig) {
            Ok(operand) => operands.push(operand),
            Err(err) => {
                return Err(ErrMode::Cut(operand_failure(
                    err, before, opcode, index, *sig,
                )));
            }
        }
    }

    Ok(AsmInsn {
        op: opcode,
        operands,
    })
}

/// Turn a bare operand failure into a rule-specific error. Register errors keep
/// their own variant because "expected a float register, found an integer
/// register" is more useful than the generic "operand must be a float register".
fn operand_failure(
    err: ErrMode<SyntaxError>,
    before: &str,
    op: OpCode,
    index: usize,
    sig: OperandParam,
) -> SyntaxError {
    let inner = match err {
        ErrMode::Backtrack(error) | ErrMode::Cut(error) => Some(error),
        ErrMode::Incomplete(_) => None,
    };

    if let Some(error) = inner
        && matches!(
            &error.kind,
            ParseErrorKind::RegisterOutOfRange { .. } | ParseErrorKind::RegisterFileMismatch { .. }
        )
    {
        return error;
    }

    let missing = at_line_end(before);
    let kind = if missing {
        ParseErrorKind::MissingOperand {
            mnemonic: op.mnemonic(),
            index: index + 1,
            arity: op.signature().len(),
        }
    } else {
        ParseErrorKind::InvalidOperand {
            mnemonic: op.mnemonic(),
            index: index + 1,
            expected: expected_operand(sig),
            found: token_snippet(before),
        }
    };
    // A missing operand points at the end of the line, not at the whitespace
    // that happened to be there.
    let remaining = if missing { "" } else { before };
    SyntaxError::at(remaining, kind)
}

/// True when only whitespace or a trailing comment is left. A comment ends the
/// line (`docs/asm.md` §1), so it cannot stand in for a missing operand.
fn at_line_end(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn expected_operand(sig: OperandParam) -> &'static str {
    match sig {
        OperandParam::Reg(_, RegType::Float) => "a float register (`f0`..`f63`)",
        OperandParam::Reg(_, RegType::Int) => "an integer register (`i0`..`i63`)",
        OperandParam::Imm(ImmType::F32) => "an f32 literal",
        OperandParam::Imm(ImmType::I32) => "an i32 literal",
        OperandParam::Imm(ImmType::Index) => "a declared name",
        OperandParam::Imm(ImmType::JumpOffset) => "a label",
        OperandParam::Imm(ImmType::None) => "no operand",
    }
}

fn reg_file_label(reg_type: RegType) -> &'static str {
    match reg_type {
        RegType::Float => "float",
        RegType::Int => "integer",
    }
}
