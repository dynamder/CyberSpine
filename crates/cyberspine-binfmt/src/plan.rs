//! The sensor tables compiled into "what each slot reads from" (`docs/isa.md` §9.2).
//!
//! A [`Plan`] is a product of verification, not of the file: the bytes say *which
//! source kind* a sensor has, and the plan says *which index* that kind resolves
//! to. The two `k` counters are per kind, because a backend's snapshot arrays are
//! packed (only the `Backend` entries) while the image's tables interleave
//! read-backs, observables and built-ins. `Backend(2)` therefore means "the third
//! `Backend` entry", not "sensor #2".
//!
//! Only a [`VerifiedImage`](crate::load::VerifiedImage) has one: the address form
//! is meaningless until M3–M6 have been checked.

use alloc::vec::Vec;

/// Where a float sensor reads from. The address form of
/// [`SensorSource`](crate::image::SensorSource), decided once while the table is
/// validated.
///
/// There is no `Tick` variant: M5 refuses `BUILTIN_TICK` in `sensors_f`, so a
/// float tick is not representable here — and a `tick` that has to round through
/// `f32` would lose its integer resolution after 2²⁴ ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorFSource {
    /// The k-th `MUSCLE` entry in `sensors_f`: that muscle's read-back.
    ///
    /// M4 forces the first `muscles.len()` entries to be exactly `Muscle(k)` at
    /// index `k`, so this is also the muscle index.
    Muscle(usize),
    /// The k-th `BACKEND` entry in `sensors_f`.
    Backend(usize),
    /// The built-in `dt`, in seconds.
    Dt,
}

/// Where an integer sensor reads from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorISource {
    /// The k-th `BACKEND` entry in `sensors_i`.
    Backend(usize),
    /// The built-in `tick`: the host's tick counter, integer by construction.
    Tick,
}

/// One address per sensor, in table order.
///
/// `f` and `i` are as long as the image's two sensor tables, and their `Backend`
/// counters are independent: `Snapshot::f` and `Snapshot::i` are separate arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub f: Vec<SensorFSource>,
    pub i: Vec<SensorISource>,
}

impl Plan {
    /// How many backend observables each snapshot array holds — the length a
    /// backend must fill, and the length of the host's `rig_f`/`rig_i` buffers.
    pub fn backend_counts(&self) -> (usize, usize) {
        let float = self
            .f
            .iter()
            .filter(|source| matches!(source, SensorFSource::Backend(_)))
            .count();
        let int = self
            .i
            .iter()
            .filter(|source| matches!(source, SensorISource::Backend(_)))
            .count();
        (float, int)
    }
}
