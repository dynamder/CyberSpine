//! A recording backend for tests and for the host's own test suite.
//!
//! It is not a simulator: `step` copies committed commands into the muscle
//! values and does nothing else. That is enough to check ordering, timing and
//! the command path without a physics model.

use cyberspine_core::rig::{BoundFingerprint, Rig};

use crate::backend::{Backend, Shape, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Sample,
    Commit,
    Step,
}

/// What a backend did, in order, so a test can assert the tick's shape.
pub struct DummyBackend {
    fingerprint: BoundFingerprint,
    muscles: Vec<f32>,
    sensors_f: Vec<f32>,
    sensors_i: Vec<i32>,
    committed: Vec<Option<f32>>,
    calls: Vec<Call>,
}

impl DummyBackend {
    pub fn new(rig: Rig) -> Self {
        Self {
            fingerprint: BoundFingerprint::of(&rig),
            muscles: vec![0.0; rig.muscles.len()],
            sensors_f: vec![0.0; rig.sensors_f.len()],
            sensors_i: vec![0; rig.sensors_i.len()],
            committed: Vec::new(),
            calls: Vec::new(),
        }
    }

    pub fn set_sensor_f(&mut self, index: usize, value: f32) {
        self.sensors_f[index] = value;
    }

    pub fn set_sensor_i(&mut self, index: usize, value: i32) {
        self.sensors_i[index] = value;
    }

    pub fn set_muscle(&mut self, index: usize, value: f32) {
        self.muscles[index] = value;
    }

    pub fn muscles(&self) -> &[f32] {
        &self.muscles
    }

    pub fn committed(&self) -> &[Option<f32>] {
        &self.committed
    }

    pub fn calls(&self) -> &[Call] {
        &self.calls
    }
}

impl Backend for DummyBackend {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    fn shape(&self) -> Shape {
        Shape {
            muscles: self.muscles.len(),
            sensors_f: self.sensors_f.len(),
            sensors_i: self.sensors_i.len(),
        }
    }

    fn sample(&mut self, out: &mut Snapshot<'_>) {
        out.f.copy_from_slice(&self.sensors_f);
        out.i.copy_from_slice(&self.sensors_i);
        out.muscles.copy_from_slice(&self.muscles);
        self.calls.push(Call::Sample);
    }

    fn commit(&mut self, commands: &[Option<f32>], _dt: f32) {
        self.committed.clear();
        self.committed.extend_from_slice(commands);
        self.calls.push(Call::Commit);
    }

    fn step(&mut self, _dt: f32) {
        for (muscle, command) in self.muscles.iter_mut().zip(&self.committed) {
            if let Some(value) = command {
                *muscle = *value;
            }
        }
        self.calls.push(Call::Step);
    }
}
