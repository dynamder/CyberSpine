//! The executor's read-only view of a loaded image (`docs/exec.md` §2).

use crate::binfmt::image::{VarF, VarI};
use crate::binfmt::load::VerifiedImage;
use crate::isa::Insn;
use crate::isa::clock::Hz;

/// A loaded action with the names stripped: exactly what execution reads.
///
/// Borrowed rather than owned because [`Instance`](super::Instance) holds a
/// reference and `thumbv6m` has no atomic reference counting (`docs/exec.md`
/// §2). The layout is derived once here, so the instruction loop never walks the
/// image's tables again.
pub struct Program<'p> {
    code: &'p [Insn],
    vars_f: &'p [VarF],
    vars_i: &'p [VarI],
    sensor_f_count: usize,
    sensor_i_count: usize,
    muscle_count: usize,
    hz: Hz,
    fuel: u32,
}

impl<'p> Program<'p> {
    pub fn new(image: &'p VerifiedImage) -> Self {
        Self {
            code: image.code(),
            vars_f: image.vars_f(),
            vars_i: image.vars_i(),
            sensor_f_count: image.sensors_f().len(),
            sensor_i_count: image.sensors_i().len(),
            muscle_count: image.muscles().len(),
            hz: image.header.hz,
            fuel: image.header.fuel,
        }
    }

    pub fn code(&self) -> &'p [Insn] {
        self.code
    }

    pub fn vars_f(&self) -> &'p [VarF] {
        self.vars_f
    }

    pub fn vars_i(&self) -> &'p [VarI] {
        self.vars_i
    }

    pub fn sensor_f_count(&self) -> usize {
        self.sensor_f_count
    }

    pub fn sensor_i_count(&self) -> usize {
        self.sensor_i_count
    }

    pub fn muscle_count(&self) -> usize {
        self.muscle_count
    }

    /// The per-resume instruction budget (`docs/isa.md` §7).
    pub fn fuel(&self) -> u32 {
        self.fuel
    }

    /// Ticks per virtual second, and the denominator of `WAIT` (`docs/isa.md` §6.1).
    pub fn hz(&self) -> Hz {
        self.hz
    }
}
