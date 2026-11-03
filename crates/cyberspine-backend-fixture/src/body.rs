//! A body with no world, used to prove the transports carry a tick.
//!
//! It reports how many steps it has taken and holds its last command, which is
//! the smallest body that exercises the `sample -> commit -> step` order. The
//! same type is linked into a test and exported from a `cdylib`: the traces have
//! to match.

use cyberspine_rig::{BoundFingerprint, Name, Rig};
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

pub const RIG_JSON: &str = include_str!("../fixture.rig.json");

pub fn rig() -> Rig {
    Rig::new("fixture", 1)
        .muscle(name("m0"))
        .muscle(name("m1"))
        .sensor_f(name("s0"))
        .sensor_i(name("steps"))
}

fn name(text: &str) -> Name {
    Name::new(text).expect("the fixture's names are identifiers")
}

pub struct Fixture {
    fingerprint: BoundFingerprint,
    commands: [f32; 2],
    muscles: [f32; 2],
    tick: i32,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            fingerprint: BoundFingerprint::of(&rig()),
            commands: [0.0; 2],
            muscles: [0.0; 2],
            tick: 0,
        }
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Fixture {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    fn shape(&self) -> Shape {
        Shape {
            muscles: 2,
            sensors_f: 1,
            sensors_i: 1,
        }
    }

    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        out.f[0] = self.tick as f32;
        out.i[0] = self.tick;
        out.muscles.copy_from_slice(&self.muscles);
        Ok(())
    }

    fn commit(&mut self, commands: &[Option<f32>], _dt: f32) -> Result<(), BackendError> {
        for (held, command) in self.commands.iter_mut().zip(commands) {
            if let Some(command) = command {
                *held = *command;
            }
        }
        Ok(())
    }

    fn step(&mut self, _dt: f32) -> Result<(), BackendError> {
        self.muscles = self.commands;
        self.tick += 1;
        Ok(())
    }
}
