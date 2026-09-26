use crate::vm::op::Op;

const REG_MASK: u64 = 0x3F;
const OP_SHIFT: u32 = 56;
const RD_SHIFT: u32 = 50;
const RA_SHIFT: u32 = 44;
const RB_SHIFT: u32 = 38;
const RC_SHIFT: u32 = 32;

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct RawImm(u32);

impl RawImm {
    pub fn interpret_int(self) -> i32 {
        unsafe { i32::from_le_bytes(self.0.into()) }
    }

    pub fn interpret_float(self) -> f32 {
        self.0 as f32
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct ByteCode(u64);

impl ByteCode {
    pub fn parse(&self) -> Op {
        let op_code = self.0 & OP_MASK;
    }

    pub fn op_code(&self) -> u8 {
        (self.0 >> OP_SHIFT) as u8
    }

    pub fn rd(&self) -> u8 {
        ((self.0 >> RD_SHIFT) & REG_MASK) as u8
    }

    pub fn ra(&self) -> u8 {
        ((self.0 >> RA_SHIFT) & REG_MASK) as u8
    }

    pub fn rb(&self) -> u8 {
        ((self.0 >> RB_SHIFT) & REG_MASK) as u8
    }

    pub fn rc(&self) -> u8 {
        ((self.0 >> RC_SHIFT) & REG_MASK) as u8
    }

    pub fn imm(&self) -> RawImm {
        RawImm((self.0 & IMM_MASK) as u32)
    }
}
