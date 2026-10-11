use thiserror::Error;

use crate::register::{RegId, RegType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegName {
    Rd,
    Ra,
    Rb,
    Rc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmType {
    None,
    F32,
    I32,
    Index,
    JumpOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    Reg(RegName, RegType),
    Imm(ImmType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("Invalid Operation Code {0}")]
    InvalidOpCode(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub op: OpCode,
    pub rd: RegId,
    pub ra: RegId,
    pub rb: RegId,
    pub rc: RegId,
    pub imm: u32, // Raw bits
}

impl Insn {
    /// An instruction with `op` set and every operand field zero. `encode` writes
    /// only the fields the opcode declares, so this is the all-zero-operand form.
    pub const fn new(op: OpCode) -> Self {
        Self {
            op,
            rd: RegId::ZERO,
            ra: RegId::ZERO,
            rb: RegId::ZERO,
            rc: RegId::ZERO,
            imm: 0,
        }
    }

    pub fn decode(word: u64) -> Result<Insn, DecodeError> {
        let op = OpCode::from_byte((word >> 56) as u8)
            .ok_or(DecodeError::InvalidOpCode((word >> 56) as u8))?;
        let mut insn = Insn {
            op,
            rd: RegId::from_6bit(((word >> 50) & 0x3F) as u8),
            ra: RegId::from_6bit(((word >> 44) & 0x3F) as u8),
            rb: RegId::from_6bit(((word >> 38) & 0x3F) as u8),
            rc: RegId::from_6bit(((word >> 32) & 0x3F) as u8),
            imm: word as u32,
        };

        if !op.uses(RegName::Rd) {
            insn.rd = RegId::ZERO;
        }
        if !op.uses(RegName::Ra) {
            insn.ra = RegId::ZERO;
        }
        if !op.uses(RegName::Rb) {
            insn.rb = RegId::ZERO;
        }
        if !op.uses(RegName::Rc) {
            insn.rc = RegId::ZERO;
        }
        if op.imm_type() == ImmType::None {
            insn.imm = 0;
        }
        Ok(insn)
    }

    pub fn encode(self) -> u64 {
        let mut w = (self.op.byte() as u64) << 56;
        for operand in self.op.signature() {
            match *operand {
                Operand::Reg(RegName::Rd, _) => w |= (self.rd.get() as u64) << 50,
                Operand::Reg(RegName::Ra, _) => w |= (self.ra.get() as u64) << 44,
                Operand::Reg(RegName::Rb, _) => w |= (self.rb.get() as u64) << 38,
                Operand::Reg(RegName::Rc, _) => w |= (self.rc.get() as u64) << 32,
                Operand::Imm(ImmType::None) => {}
                Operand::Imm(_) => w |= self.imm as u64,
            }
        }
        w
    }

    pub fn imm_f32(self) -> f32 {
        f32::from_bits(self.imm)
    }
    pub fn imm_i32(self) -> i32 {
        self.imm as i32
    }
    pub fn imm_index(self) -> usize {
        self.imm as usize
    }
}

macro_rules! operand {
    (rd F) => {
        Operand::Reg(RegName::Rd, RegType::Float)
    };
    (rd I) => {
        Operand::Reg(RegName::Rd, RegType::Int)
    };
    (ra F) => {
        Operand::Reg(RegName::Ra, RegType::Float)
    };
    (ra I) => {
        Operand::Reg(RegName::Ra, RegType::Int)
    };
    (rb F) => {
        Operand::Reg(RegName::Rb, RegType::Float)
    };
    (rb I) => {
        Operand::Reg(RegName::Rb, RegType::Int)
    };
    (rc F) => {
        Operand::Reg(RegName::Rc, RegType::Float)
    };
    (rc I) => {
        Operand::Reg(RegName::Rc, RegType::Int)
    };
    (imm F32) => {
        Operand::Imm(ImmType::F32)
    };
    (imm I32) => {
        Operand::Imm(ImmType::I32)
    };
    (imm Index) => {
        Operand::Imm(ImmType::Index)
    };
    (imm JumpOffset) => {
        Operand::Imm(ImmType::JumpOffset)
    };
}

macro_rules! isa_table {
    (
        $(
            $opcode:literal => $variant:ident, $mnemonic:literal, $cycles:literal
            $(, ( $reg:ident $reg_type: ident) )*;
        )*
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum OpCode {
            $( $variant, )*
        }

        impl OpCode {
            pub const ALL: &'static [OpCode] = &[ $( OpCode::$variant, )* ];

            pub fn from_byte(b: u8) -> Option<Self> {
                match b {
                    $( $opcode => Some(OpCode::$variant), )*
                    _ => None
                }
            }

            pub fn byte(self) -> u8 {
                match self {
                    $( OpCode::$variant => $opcode, )*
                }
            }

            pub fn mnemonic(self) -> &'static str {
                match self {
                    $( OpCode::$variant => $mnemonic, )*
                }
            }

            pub fn cycles(self) -> u32 {
                match self {
                    $( OpCode::$variant => $cycles, )*
                }
            }

            pub fn signature(self) -> &'static [Operand] {
                match self {
                    $( OpCode::$variant => &[ $( operand!($reg $reg_type) ),* ], )*
                }
            }

            pub fn imm_type(self) -> ImmType {
                self.signature()
                    .iter()
                    .find_map(|o| match o {
                        Operand::Imm(t) => Some(*t),
                        _ => None,
                    })
                    .unwrap_or(ImmType::None)
            }

            pub fn uses(self, reg_name: RegName) -> bool {
                self.signature().iter().any(|o| matches!(o, Operand::Reg(n, _) if *n == reg_name))
            }

            pub fn reg_type(self, reg_name: RegName) -> Option<RegType> {
                self.signature().iter().find_map(|o| match o {
                    Operand::Reg(n, c) if *n == reg_name => Some(*c),
                    _ => None,
                })
            }
        }
    };


}

isa_table! {
    // 传送与常量 0x00
    0x00 => Fmovi, "FMOVI", 1, (rd F), (imm F32);
    0x01 => Imovi, "IMOVI", 1, (rd I), (imm I32);
    0x02 => Fmov,  "FMOV",  1, (rd F), (ra F);
    0x03 => Imov,  "IMOV",  1, (rd I), (ra I);

    // 浮点二元 0x10
    0x10 => Fadd, "FADD", 3, (rd F), (ra F), (rb F);
    0x11 => Fsub, "FSUB", 3, (rd F), (ra F), (rb F);
    0x12 => Fmul, "FMUL", 4, (rd F), (ra F), (rb F);
    0x13 => Fdiv, "FDIV", 14, (rd F), (ra F), (rb F);
    0x14 => Fmin, "FMIN", 2, (rd F), (ra F), (rb F);
    0x15 => Fmax, "FMAX", 2, (rd F), (ra F), (rb F);

    // 浮点一元 0x20
    0x20 => Fneg,  "FNEG",  1, (rd F), (ra F);
    0x21 => Fabs,  "FABS",  1, (rd F), (ra F);
    0x22 => Fsqrt, "FSQRT", 12, (rd F), (ra F);
    0x23 => Fsin,  "FSIN",  40, (rd F), (ra F);
    0x24 => Fcos,  "FCOS",  40, (rd F), (ra F);
    0x25 => Ffrac, "FFRAC", 6, (rd F), (ra F);

    // 浮点三元 0x30
    0x30 => Fmadd,  "FMADD",  5, (rd F), (ra F), (rb F), (rc F);
    0x31 => Flerp,  "FLERP",  8, (rd F), (ra F), (rb F), (rc F);
    0x32 => Fclamp, "FCLAMP", 3, (rd F), (ra F), (rb F), (rc F);

    // 整数与转换 0x40
    0x40 => Iadd, "IADD", 1, (rd I), (ra I), (rb I);
    0x41 => Isub, "ISUB", 1, (rd I), (ra I), (rb I);
    0x42 => Imul, "IMUL", 3, (rd I), (ra I), (rb I);
    0x43 => Itof, "ITOF", 2, (rd F), (ra I);
    0x44 => Ftoi, "FTOI", 2, (rd I), (ra F);

    // 控制流 0x50
    0x50 => Jmp,  "JMP",  1, (imm JumpOffset);
    0x51 => Ijz,  "IJZ",  1, (ra I), (imm JumpOffset);
    0x52 => Ijnz, "IJNZ", 1, (ra I), (imm JumpOffset);
    0x53 => Flt,  "FLT",  1, (ra F), (rb F), (imm JumpOffset);
    0x54 => Fle,  "FLE",  1, (ra F), (rb F), (imm JumpOffset);
    0x55 => Fgt,  "FGT",  1, (ra F), (rb F), (imm JumpOffset);
    0x56 => Fge,  "FGE",  1, (ra F), (rb F), (imm JumpOffset);
    0x57 => Ilt,  "ILT",  1, (ra I), (rb I), (imm JumpOffset);
    0x58 => Ile,  "ILE",  1, (ra I), (rb I), (imm JumpOffset);
    0x59 => Igt,  "IGT",  1, (ra I), (rb I), (imm JumpOffset);
    0x5A => Ige,  "IGE",  1, (ra I), (rb I), (imm JumpOffset);
    0x5B => Ieq,  "IEQ",  1, (ra I), (rb I), (imm JumpOffset);
    0x5C => Ine,  "INE",  1, (ra I), (rb I), (imm JumpOffset);

    // 输入 0x60
    0x60 => Fsense, "FSENSE", 1, (rd F), (imm Index);
    0x61 => Isense, "ISENSE", 1, (rd I), (imm Index);

    // 挂起 0x70
    0x70 => Yield, "YIELD", 2;
    0x71 => Wait,  "WAIT",  2, (imm F32);

    // 激发 0x80
    0x80 => Fexcite, "FEXCITE", 1, (imm Index), (ra F);

    // 任务变量 0x90
    0x90 => Fvget, "FVGET", 1, (rd F), (imm Index);
    0x91 => Fvset, "FVSET", 1, (imm Index), (ra F);
    0x92 => Ivget, "IVGET", 1, (rd I), (imm Index);
    0x93 => Ivset, "IVSET", 1, (imm Index), (ra I);

    // 杂项 0xA0
    0xA0 => Halt, "HALT", 1;
    0xA1 => Nop,  "NOP",  1;
}
