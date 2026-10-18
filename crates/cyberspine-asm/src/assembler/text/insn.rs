use std::str::FromStr;

use cyberspine_vm::vm::op::{ImmType, OpCode, OperandParam};
use winnow::{ModalResult, Parser, ascii::space0};

use crate::assembler::{
    Line, ident,
    text::{
        AsmInsn, AsmOperand,
        imm::{imm_float, imm_ident, imm_int},
        reg::reg,
    },
};

pub fn operand_delimiter(input: &mut Line<'_>) -> ModalResult<()> {
    (space0, ',', space0).void().parse_next(input)
}

fn opcode(input: &mut Line<'_>) -> ModalResult<OpCode> {
    ident
        //TODO: enhance the error handling
        .map(|op| OpCode::from_str(op).unwrap())
        .parse_next(input)
}

pub fn operand(input: &mut Line<'_>, sig: OperandParam) -> ModalResult<AsmOperand> {
    let operand = match sig {
        OperandParam::Reg(name, sig_reg_type) => {
            let parsed_reg = reg
                .verify_map(|(id, reg_type)| {
                    if sig_reg_type != reg_type {
                        return None;
                    }
                    Some(AsmOperand::new_reg(id, name, reg_type))
                })
                .parse_next(input)?;

            operand_delimiter.parse_next(input)?;
            parsed_reg
        }
        OperandParam::Imm(imm_type) => {
            let imm = match imm_type {
                ImmType::F32 => imm_float.map(AsmOperand::new_imm_f32).parse_next(input)?,
                ImmType::I32 => imm_int.map(AsmOperand::new_imm_i32).parse_next(input)?,
                ImmType::Index => imm_ident.map(AsmOperand::new_index).parse_next(input)?,
                ImmType::JumpOffset => imm_ident.map(AsmOperand::new_target).parse_next(input)?,
                ImmType::None => AsmOperand::ImmNone,
            };
            operand_delimiter.parse_next(input)?;
            imm
        }
    };
    Ok(operand)
}

pub fn insn(input: &mut Line<'_>) -> ModalResult<AsmInsn> {
    let opcode = opcode.parse_next(input)?;
    let signature = opcode.signature();

    space0.parse_next(input)?;

    let operands = signature
        .iter()
        .map(|sig| operand(input, *sig))
        //TODO: enhance the error handling
        .collect::<ModalResult<Vec<_>>>()?;

    Ok(AsmInsn {
        op: opcode,
        operands,
    })
}
