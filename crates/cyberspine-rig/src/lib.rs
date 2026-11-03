//! The rig: the names a body provides, and the fingerprint an image carries
//! (`docs/isa.md` §13).
//!
//! This is the identity layer: a `Rig`, its validated [`Name`]s, and the
//! [`Fingerprint`] both the assembler and the host compute from it. It is
//! `#![no_std]` with only `alloc`, because the same hash has to be computable on
//! a device and read back from an image.
//!
//! The assembler and the backend build a [`Rig`] from their own inputs and hash
//! it with the same function. The image stores the assembler's answer; the host
//! compares it with the backend's before anything runs, so an action can never
//! be executed against a body it was not assembled for.

#![no_std]

extern crate alloc;

pub mod name;

use alloc::boxed::Box;
use alloc::vec::Vec;

use thiserror::Error;

pub use name::{Name, NameError, is_reserved};

pub const FINGERPRINT_LEN: usize = 32;

/// What an image was assembled against.
///
/// [`Fingerprint::UNBOUND`] means "assembled without a rig" — a hand-built test
/// image. A host must refuse it unless it was explicitly told otherwise.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Fingerprint(pub [u8; FINGERPRINT_LEN]);

impl Fingerprint {
    pub const UNBOUND: Self = Self([0; FINGERPRINT_LEN]);

    pub fn is_unbound(self) -> bool {
        self == Self::UNBOUND
    }
}

impl core::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl core::fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

/// A fingerprint that is known to name a body.
///
/// [`Fingerprint`] can be [`Fingerprint::UNBOUND`], because an image may be
/// assembled without a rig. A backend cannot: it *is* a body, so it always has
/// an identity to report. Splitting the two types removes the case where an
/// unbound image and an unbound backend "match" and slip past the host's check
/// (`docs/rig.md` §4).
///
/// The invariant is "never [`Fingerprint::UNBOUND`]", and it is upheld by three
/// things together: the field is private, [`BoundFingerprint::of`] is the only
/// constructor, and there is deliberately no `From<Fingerprint>`, no `Default`
/// and no `UNBOUND` constant. A `[u8; 32]` read back from a device therefore
/// cannot be promoted to a body identity by painting it with this type.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BoundFingerprint(Fingerprint);

impl BoundFingerprint {
    /// The identity of a validated rig.
    ///
    /// Validation is the caller's, not this function's: a rig that failed
    /// [`Rig::validate`] is not a body, and hashing it would mint an identity for
    /// something that does not exist.
    pub fn of(rig: &Rig) -> Self {
        let digest = rig.fingerprint();
        // blake3 over a non-empty, fixed-prefix encoding: the digest is all zeros
        // with probability 2^-256, and such a digest would be indistinguishable
        // from UNBOUND. Nothing else can produce a zero here, so this is the whole
        // of the invariant's runtime half.
        debug_assert!(
            !digest.is_unbound(),
            "a body's fingerprint must never read as UNBOUND"
        );
        Self(digest)
    }

    pub fn fingerprint(self) -> Fingerprint {
        self.0
    }
}

impl core::fmt::Display for BoundFingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl core::fmt::Debug for BoundFingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "BoundFingerprint({})", self.0)
    }
}

/// Which table a [`RigError`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RigTable {
    Muscles,
    SensorsF,
    SensorsI,
}

impl core::fmt::Display for RigTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Muscles => "muscles",
            Self::SensorsF => "sensors_f",
            Self::SensorsI => "sensors_i",
        })
    }
}

/// Why a body's declaration is not a legal rig (`docs/rig.md` §2).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RigError {
    #[error("the rig name must not be empty")]
    EmptyName,
    #[error("{table}[{index}] {name:?} is not an identifier")]
    InvalidName {
        table: RigTable,
        index: usize,
        name: Box<str>,
    },
    #[error("{table}[{index}] {name:?} is reserved by the language")]
    ReservedName {
        table: RigTable,
        index: usize,
        name: Name,
    },
    #[error("{table}[{index}] {name:?} is declared more than once")]
    DuplicateName {
        table: RigTable,
        index: usize,
        name: Name,
    },
}

impl RigError {
    /// The `InvalidName` form of a name that [`Name::new`] rejected.
    pub fn invalid_name(table: RigTable, index: usize, name: &str) -> Self {
        Self::InvalidName {
            table,
            index,
            name: name.into(),
        }
    }
}

/// A body's vocabulary: identity, muscles, and the observables it reports.
///
/// Order is meaning: the index of a name here is its physical index in the
/// backend. `dt` and `tick` are not listed — they are built into the language.
///
/// Construction does not validate: a builder whose every step returned `Result`
/// would make each `rig.muscle(..)` chain fallible for no benefit. The gate is
/// [`Rig::validate`], which the manifest parser and the assembler both call —
/// one implementation, two entry points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rig {
    pub name: Box<str>,
    pub version: u32,
    pub muscles: Vec<Name>,
    pub sensors_f: Vec<Name>,
    pub sensors_i: Vec<Name>,
}

impl Rig {
    pub fn new(name: impl Into<Box<str>>, version: u32) -> Self {
        Self {
            name: name.into(),
            version,
            muscles: Vec::new(),
            sensors_f: Vec::new(),
            sensors_i: Vec::new(),
        }
    }

    pub fn muscle(mut self, name: Name) -> Self {
        self.muscles.push(name);
        self
    }

    pub fn sensor_f(mut self, name: Name) -> Self {
        self.sensors_f.push(name);
        self
    }

    pub fn sensor_i(mut self, name: Name) -> Self {
        self.sensors_i.push(name);
        self
    }

    /// The rig-level half of `docs/asm.md` §6.8: a non-empty identity, no
    /// reserved spellings, and one namespace across the three tables.
    ///
    /// The grammar of a single name is [`Name`]'s job; this decides only what a
    /// declaration may not do.
    pub fn validate(&self) -> Result<(), RigError> {
        if self.name.is_empty() {
            return Err(RigError::EmptyName);
        }

        let mut seen: Vec<&Name> = Vec::new();
        for (table, names) in [
            (RigTable::Muscles, &self.muscles),
            (RigTable::SensorsF, &self.sensors_f),
            (RigTable::SensorsI, &self.sensors_i),
        ] {
            for (index, name) in names.iter().enumerate() {
                if is_reserved(name.as_str()) {
                    return Err(RigError::ReservedName {
                        table,
                        index,
                        name: name.clone(),
                    });
                }
                if seen.contains(&name) {
                    return Err(RigError::DuplicateName {
                        table,
                        index,
                        name: name.clone(),
                    });
                }
                seen.push(name);
            }
        }

        Ok(())
    }

    /// The canonical hash of this rig.
    ///
    /// Every field is length-prefixed so no two different rigs can produce the
    /// same byte string, and the identity version is part of the input: renaming
    /// nothing but changing what a muscle means still has to bump it.
    pub fn fingerprint(&self) -> Fingerprint {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"cyberspine-rig\0");
        push_name(&mut bytes, &self.name);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        for list in [&self.muscles, &self.sensors_f, &self.sensors_i] {
            bytes.extend_from_slice(&(list.len() as u32).to_le_bytes());
            for name in list {
                push_name(&mut bytes, name.as_str());
            }
        }
        Fingerprint(*blake3::hash(&bytes).as_bytes())
    }
}

fn push_name(bytes: &mut Vec<u8>, name: &str) {
    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name.as_bytes());
}
