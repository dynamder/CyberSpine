//! The rig: the names a body provides, and the fingerprint an image carries
//! (`docs/isa.md` §13).
//!
//! The assembler and the backend build a [`Rig`] from their own inputs and hash
//! it with the same function. The image stores the assembler's answer; the host
//! compares it with the backend's before anything runs, so an action can never
//! be executed against a body it was not assembled for.

use alloc::boxed::Box;
use alloc::vec::Vec;

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

/// A body's vocabulary: identity, muscles, and the observables it reports.
///
/// Order is meaning: the index of a name here is its physical index in the
/// backend. `dt` and `tick` are not listed — they are built into the language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rig {
    pub name: Box<str>,
    pub version: u32,
    pub muscles: Vec<Box<str>>,
    pub sensors_f: Vec<Box<str>>,
    pub sensors_i: Vec<Box<str>>,
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

    pub fn muscle(mut self, name: impl Into<Box<str>>) -> Self {
        self.muscles.push(name.into());
        self
    }

    pub fn sensor_f(mut self, name: impl Into<Box<str>>) -> Self {
        self.sensors_f.push(name.into());
        self
    }

    pub fn sensor_i(mut self, name: impl Into<Box<str>>) -> Self {
        self.sensors_i.push(name.into());
        self
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
                push_name(&mut bytes, name);
            }
        }
        Fingerprint(*blake3::hash(&bytes).as_bytes())
    }
}

fn push_name(bytes: &mut Vec<u8>, name: &str) {
    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name.as_bytes());
}
