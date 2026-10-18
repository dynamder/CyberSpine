use core::str::FromStr;

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
pub enum OperandParam {
    Reg(RegName, RegType),
    Imm(ImmType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("Invalid Operation Code {0}")]
    InvalidOpCode(u8),
}

/// A mnemonic that names no instruction in the ISA table.
///
/// Deliberately **not** a [`DecodeError`] variant: that type is about wire bytes
/// (`word >> 56`), this one is about source spelling. Merging them would produce
/// a variant whose name and payload disagree with how it is used.
///
/// Owns the rejected text instead of borrowing it, so `Err` stays `'static` and
/// the assembler can aggregate errors across lines (`docs/asm.md` §6).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown instruction mnemonic {0:?}")]
pub struct MnemonicError(pub String);

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
                OperandParam::Reg(RegName::Rd, _) => w |= (self.rd.get() as u64) << 50,
                OperandParam::Reg(RegName::Ra, _) => w |= (self.ra.get() as u64) << 44,
                OperandParam::Reg(RegName::Rb, _) => w |= (self.rb.get() as u64) << 38,
                OperandParam::Reg(RegName::Rc, _) => w |= (self.rc.get() as u64) << 32,
                OperandParam::Imm(ImmType::None) => {}
                OperandParam::Imm(_) => w |= self.imm as u64,
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
        OperandParam::Reg(RegName::Rd, RegType::Float)
    };
    (rd I) => {
        OperandParam::Reg(RegName::Rd, RegType::Int)
    };
    (ra F) => {
        OperandParam::Reg(RegName::Ra, RegType::Float)
    };
    (ra I) => {
        OperandParam::Reg(RegName::Ra, RegType::Int)
    };
    (rb F) => {
        OperandParam::Reg(RegName::Rb, RegType::Float)
    };
    (rb I) => {
        OperandParam::Reg(RegName::Rb, RegType::Int)
    };
    (rc F) => {
        OperandParam::Reg(RegName::Rc, RegType::Float)
    };
    (rc I) => {
        OperandParam::Reg(RegName::Rc, RegType::Int)
    };
    (imm F32) => {
        OperandParam::Imm(ImmType::F32)
    };
    (imm I32) => {
        OperandParam::Imm(ImmType::I32)
    };
    (imm Index) => {
        OperandParam::Imm(ImmType::Index)
    };
    (imm JumpOffset) => {
        OperandParam::Imm(ImmType::JumpOffset)
    };
}

macro_rules! isa_table {
    (
        $(
            $opcode:literal => $variant:ident, $mnemonic:literal
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

            pub fn signature(self) -> &'static [OperandParam] {
                match self {
                    $( OpCode::$variant => &[ $( operand!($reg $reg_type) ),* ], )*
                }
            }

            pub fn imm_type(self) -> ImmType {
                self.signature()
                    .iter()
                    .find_map(|o| match o {
                        OperandParam::Imm(t) => Some(*t),
                        _ => None,
                    })
                    .unwrap_or(ImmType::None)
            }

            pub fn uses(self, reg_name: RegName) -> bool {
                self.signature().iter().any(|o| matches!(o, OperandParam::Reg(n, _) if *n == reg_name))
            }

            pub fn reg_type(self, reg_name: RegName) -> Option<RegType> {
                self.signature().iter().find_map(|o| match o {
                    OperandParam::Reg(n, c) if *n == reg_name => Some(*c),
                    _ => None,
                })
            }
        }

        /// `docs/asm.md` §1 spells the rule out: mnemonics are
        /// **case-insensitive**. Case carries no information for a mnemonic, so
        /// the lookup folds the input rather than making every caller do it.
        ///
        /// Expanding to a `match` instead of scanning `ALL` with `mnemonic()`
        /// buys two things: a duplicated mnemonic in the table trips
        /// `unreachable_patterns` **at compile time** (a scan would only make
        /// the first match silently win), and the lookup no longer depends on
        /// `mnemonic()` staying correct.
        ///
        /// It does depend on the table's canonical spellings being uppercase,
        /// since the folded input is compared against them verbatim: a lowercase
        /// row would silently never match. `tests/opcode.rs` pins that invariant
        /// in `canonical_mnemonics_are_non_empty_uppercase_identifiers`.
        impl FromStr for OpCode {
            type Err = MnemonicError;

            fn from_str(name: &str) -> Result<Self, Self::Err> {
                let folded = name.to_ascii_uppercase();
                match folded.as_str() {
                    $( $mnemonic => Ok(OpCode::$variant), )*
                    _ => Err(MnemonicError(name.to_owned())),
                }
            }
        }
    };


}

// ---------------------------------------------------------------------------
// The ISA table (`docs/isa.md` §4, 49 instructions)
// ---------------------------------------------------------------------------
//
// One row per opcode: `opcode => variant, "MNEMONIC" [, (operand ...)]`.
//
// There is deliberately **no cost column**. The cycle-accounting table is
// retired (`docs/isa.md` §6.5): fuel charges every executed instruction exactly
// 1, so a per-opcode weight is a number with no consumer.

isa_table! {
    // 传送与常量 0x00
    0x00 => Fmovi, "FMOVI", (rd F), (imm F32);
    0x01 => Imovi, "IMOVI", (rd I), (imm I32);
    0x02 => Fmov,  "FMOV",  (rd F), (ra F);
    0x03 => Imov,  "IMOV",  (rd I), (ra I);

    // 浮点二元 0x10
    0x10 => Fadd, "FADD", (rd F), (ra F), (rb F);
    0x11 => Fsub, "FSUB", (rd F), (ra F), (rb F);
    0x12 => Fmul, "FMUL", (rd F), (ra F), (rb F);
    0x13 => Fdiv, "FDIV", (rd F), (ra F), (rb F);
    0x14 => Fmin, "FMIN", (rd F), (ra F), (rb F);
    0x15 => Fmax, "FMAX", (rd F), (ra F), (rb F);

    // 浮点一元 0x20
    0x20 => Fneg,  "FNEG",  (rd F), (ra F);
    0x21 => Fabs,  "FABS",  (rd F), (ra F);
    0x22 => Fsqrt, "FSQRT", (rd F), (ra F);
    0x23 => Fsin,  "FSIN",  (rd F), (ra F);
    0x24 => Fcos,  "FCOS",  (rd F), (ra F);
    0x25 => Ffrac, "FFRAC", (rd F), (ra F);

    // 浮点三元 0x30
    0x30 => Fmadd,  "FMADD",  (rd F), (ra F), (rb F), (rc F);
    0x31 => Flerp,  "FLERP",  (rd F), (ra F), (rb F), (rc F);
    0x32 => Fclamp, "FCLAMP", (rd F), (ra F), (rb F), (rc F);

    // 整数与转换 0x40
    0x40 => Iadd, "IADD", (rd I), (ra I), (rb I);
    0x41 => Isub, "ISUB", (rd I), (ra I), (rb I);
    0x42 => Imul, "IMUL", (rd I), (ra I), (rb I);
    0x43 => Itof, "ITOF", (rd F), (ra I);
    0x44 => Ftoi, "FTOI", (rd I), (ra F);

    // 控制流 0x50
    0x50 => Jmp,  "JMP",  (imm JumpOffset);
    0x51 => Ijz,  "IJZ",  (ra I), (imm JumpOffset);
    0x52 => Ijnz, "IJNZ", (ra I), (imm JumpOffset);
    0x53 => Flt,  "FLT",  (ra F), (rb F), (imm JumpOffset);
    0x54 => Fle,  "FLE",  (ra F), (rb F), (imm JumpOffset);
    0x55 => Fgt,  "FGT",  (ra F), (rb F), (imm JumpOffset);
    0x56 => Fge,  "FGE",  (ra F), (rb F), (imm JumpOffset);
    0x57 => Ilt,  "ILT",  (ra I), (rb I), (imm JumpOffset);
    0x58 => Ile,  "ILE",  (ra I), (rb I), (imm JumpOffset);
    0x59 => Igt,  "IGT",  (ra I), (rb I), (imm JumpOffset);
    0x5A => Ige,  "IGE",  (ra I), (rb I), (imm JumpOffset);
    0x5B => Ieq,  "IEQ",  (ra I), (rb I), (imm JumpOffset);
    0x5C => Ine,  "INE",  (ra I), (rb I), (imm JumpOffset);

    // 输入 0x60
    0x60 => Fsense, "FSENSE", (rd F), (imm Index);
    0x61 => Isense, "ISENSE", (rd I), (imm Index);

    // 挂起 0x70
    0x70 => Yield, "YIELD";
    0x71 => Wait,  "WAIT", (imm F32);

    // 激发 0x80
    0x80 => Fexcite, "FEXCITE", (imm Index), (ra F);

    // 任务变量 0x90
    0x90 => Fvget, "FVGET", (rd F), (imm Index);
    0x91 => Fvset, "FVSET", (imm Index), (ra F);
    0x92 => Ivget, "IVGET", (rd I), (imm Index);
    0x93 => Ivset, "IVSET", (imm Index), (ra I);

    // 杂项 0xA0
    0xA0 => Halt, "HALT";
    0xA1 => Nop,  "NOP";
    // `published ← pending`, then clear pending. Not a suspend: it does not
    // advance time and does not touch the backend (`docs/isa.md` §4.11, §8.2).
    0xA2 => Flush, "FLUSH";
}
