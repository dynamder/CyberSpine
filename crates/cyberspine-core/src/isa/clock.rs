//! The virtual clock rate (`docs/isa.md` §6.1).
//!
//! `dt = 1 / hz`, and `hz` is the denominator of `WAIT`. Every consumer divides
//! by it, so zero is not a value the program can be given: a non-zero rate is
//! carried by the type rather than re-checked at each division.

use core::fmt;
use core::num::NonZeroU32;

/// Ticks per virtual second.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hz(NonZeroU32);

/// The rate `docs/isa.md` §6.1 makes the default.
pub const DEFAULT_HZ: u32 = 60;

impl Hz {
    pub const DEFAULT: Self = match NonZeroU32::new(DEFAULT_HZ) {
        Some(rate) => Self(rate),
        None => panic!("the default clock rate must not be zero"),
    };

    pub const fn new(rate: u32) -> Option<Self> {
        match NonZeroU32::new(rate) {
            Some(rate) => Some(Self(rate)),
            None => None,
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// Seconds per tick, the value `dt` carries for the whole action.
    pub fn dt(self) -> f32 {
        1.0 / self.get() as f32
    }
}

impl Default for Hz {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Debug for Hz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hz({})", self.get())
    }
}

impl fmt::Display for Hz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}
