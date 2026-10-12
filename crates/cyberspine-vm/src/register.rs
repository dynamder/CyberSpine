pub const REGS_SIZE: usize = 64;

pub struct Registers {
    f: [f32; REGS_SIZE],
    i: [i32; REGS_SIZE],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegType {
    Float,
    Int,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegId(u8);

impl RegId {

    pub const ZERO: RegId = RegId(0);
    
    pub fn new(id: u8) -> Option<Self> {
        if id < 64 { Some(RegId(id)) } else { None }
    }

    pub const fn from_6bit(id: u8) -> Self {
        debug_assert!(id < 64);
        RegId(id)
    }

    pub const fn get(&self) -> u8 {
        self.0
    }
}

impl Registers {
    pub fn new() -> Self {
        Registers {
            f: [0 as f32; REGS_SIZE],
            i: [0; REGS_SIZE],
        }
    }

    pub fn fetch_i(&self, reg_id: usize) -> i32 {
        *unsafe { self.i.get_unchecked(reg_id) } //SAFETY: the bytecode interpret regs for 6 bits, which is impossible to out of bound.
    }

    pub fn fetch_f(&self, reg_id: usize) -> f32 {
        *unsafe { self.f.get_unchecked(reg_id) } //SAFETY: same as fetch_i
    }

    pub fn load_i(&mut self, reg_id: usize, imm: i32) {
        unsafe {
            let reg = self.i.get_unchecked_mut(reg_id); //SAFETY: same as fetch_i
            *reg = imm;
        }
    }

    pub fn load_f(&mut self, reg_id: usize, imm: f32) {
        unsafe {
            let reg = self.f.get_unchecked_mut(reg_id); //SAFETY: same as fetch_i
            *reg = imm;
        }
    }
}
