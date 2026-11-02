//! The boundary between the host and a controlled body (`docs/isa.md` §9).
//!
//! `cyberspine-core` never names this trait: the VM sees only a sensor snapshot
//! and returns a command vector. Everything here is the host's business.

/// The sensor buffers the host lends to [`Backend::sample`] for one tick.
pub struct SensorValues<'s> {
    pub f: &'s mut [f32],
    pub i: &'s mut [i32],
}

/// How big this rig is. Fixed at construction: a rig does not grow muscles at
/// runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RigLayout {
    pub sensors_f: usize,
    pub sensors_i: usize,
    pub muscles: usize,
}

/// A controlled body: the thing whose state the program observes and drives.
///
/// The host calls [`Backend::sample`] once per tick, then [`Backend::commit`]
/// and [`Backend::step`]. The order is the host's contract, not the backend's
/// concern — a backend may assume the three are called once each per tick, in
/// that order.
pub trait Backend {
    fn layout(&self) -> RigLayout;

    /// Fill this tick's sensor snapshot. Called *before* any instance runs, so
    /// a program never observes an excitation it made this tick.
    fn sample(&mut self, out: &mut SensorValues<'_>);

    /// The current value of one muscle, for a `SensorSource::Muscle` sensor.
    /// This is the body's state, not the last command.
    fn muscle(&self, index: usize) -> f32;

    /// Take this tick's merged command vector. `None` means no instance
    /// published that muscle. Must not advance the body.
    fn commit(&mut self, commands: &[Option<f32>], dt: f32);

    /// Advance the body by one fixed tick. The only place physical state moves.
    fn step(&mut self, dt: f32);
}
