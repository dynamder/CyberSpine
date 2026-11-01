#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegType {
    Float,
    Int,
}

/// A register number: `f0..f63` or `i0..i63` (`docs/isa.md` §2.1).
///
/// [`RegId::COUNT`] is the array length and the encoding's field width at the
/// same time, so a `RegId` indexes a register file without a bounds check.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegId(u8);

impl RegId {
    pub const ZERO: RegId = RegId(0);

    /// One slot per representable register number.
    pub const COUNT: usize = 64;

    pub fn new(id: u8) -> Option<Self> {
        (id < Self::COUNT as u8).then_some(RegId(id))
    }

    /// For a caller that has just masked a 6-bit field, so the bound is already
    /// known to hold. Crate-private on purpose: it is what keeps
    /// [`Registers`](crate::exec::registers::Registers)' unchecked indexing
    /// sound.
    pub(crate) const fn from_6bit(id: u8) -> Self {
        debug_assert!(id < Self::COUNT as u8);
        RegId(id)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}
