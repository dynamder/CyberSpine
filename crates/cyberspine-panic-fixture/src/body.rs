//! A body that is legal up to the first tick and panics there.
//!
//! The rig, the shape and the handshake are ordinary: the host must reach the
//! first `sample` before anything goes wrong, so the failure is the body's
//! panic and nothing earlier.

use cyberspine_rig::{BoundFingerprint, Name, Rig};
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

pub const RIG_JSON: &str = include_str!("../panic.rig.json");

/// The text the panic carries; a test asserting on it proves the message
/// crossed the boundary through `last_error`, not just the status.
pub const PANIC_REASON: &str = "the fixture body panics on purpose";

pub fn rig() -> Rig {
    Rig::new("panic", 1).muscle(Name::new("m0").expect("the fixture's name is an identifier"))
}

pub struct Panicky {
    fingerprint: BoundFingerprint,
}

impl Panicky {
    pub fn new() -> Self {
        Self {
            fingerprint: BoundFingerprint::of(&rig()),
        }
    }
}

impl Default for Panicky {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Panicky {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    fn shape(&self) -> Shape {
        Shape {
            muscles: 1,
            sensors_f: 0,
            sensors_i: 0,
        }
    }

    fn sample(&mut self, _out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        panic!("{PANIC_REASON}")
    }

    fn commit(&mut self, _commands: &[Option<f32>], _dt: f32) -> Result<(), BackendError> {
        Ok(())
    }

    fn step(&mut self, _dt: f32) -> Result<(), BackendError> {
        Ok(())
    }
}
