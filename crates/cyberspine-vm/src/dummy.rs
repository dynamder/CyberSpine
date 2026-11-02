//! A recording backend for tests and for the host's own test suite.
//!
//! It is not a simulator: `step` copies committed commands into the muscle
//! values and does nothing else. That is enough to check ordering, timing and
//! the command path without a physics model.

use crate::backend::{Backend, RigLayout, SensorValues};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Sample,
    Commit,
    Step,
}

/// What a backend did, in order, so a test can assert the tick's shape.
pub struct DummyBackend {
    layout: RigLayout,
    muscles: Vec<f32>,
    sensors_f: Vec<f32>,
    sensors_i: Vec<i32>,
    committed: Vec<Option<f32>>,
    calls: Vec<Call>,
}

impl DummyBackend {
    pub fn new(layout: RigLayout) -> Self {
        Self {
            layout,
            muscles: vec![0.0; layout.muscles],
            sensors_f: vec![0.0; layout.sensors_f],
            sensors_i: vec![0; layout.sensors_i],
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
    fn layout(&self) -> RigLayout {
        self.layout
    }

    fn sample(&mut self, out: &mut SensorValues<'_>) {
        out.f.copy_from_slice(&self.sensors_f);
        out.i.copy_from_slice(&self.sensors_i);
        self.calls.push(Call::Sample);
    }

    fn muscle(&self, index: usize) -> f32 {
        self.muscles[index]
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
