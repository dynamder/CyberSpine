use crate::register;

pub const REGS_SIZE: usize = 64;

pub struct Registers {
    f: [f32; REGS_SIZE],
    i: [i32; REGS_SIZE],
}

pub enum RegType {
    Float,
    Int,
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
