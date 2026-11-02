#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegType {
    Float,
    Int,
}

/// A register number: `f0..f63` or `i0..i63` (`docs/isa.md` §2.1).
///
/// [`RegId::COUNT`] is the array length and [`RegId::FIELD_BITS`] is the
/// encoding's field width, so a `RegId` indexes a register file without a bounds
/// check. The two are tied by a compile-time assertion: widening the register
/// file without widening the field, or the other way round, would make
/// `from_6bit` produce a number the register arrays cannot hold.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegId(u8);

impl RegId {
    pub const ZERO: RegId = RegId(0);

    /// One slot per representable register number.
    pub const COUNT: usize = 64;

    /// The width of the register field in an instruction word (§1.1).
    pub const FIELD_BITS: u32 = 6;

    /// The mask that field is read with.
    pub const FIELD_MASK: u64 = (1 << Self::FIELD_BITS) - 1;

    pub fn new(id: u8) -> Option<Self> {
        (id < Self::COUNT as u8).then_some(RegId(id))
    }

    /// For a caller that has just masked a field of [`RegId::FIELD_BITS`], so the
    /// bound is already known to hold. Crate-private on purpose: it is what keeps
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

const _: () = assert!(RegId::COUNT == 1usize << RegId::FIELD_BITS);
