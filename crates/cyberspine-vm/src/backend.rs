//! The boundary between the host and a controlled body (`docs/isa.md` §9).
//!
//! `cyberspine-core` never names this trait: the VM sees only a sensor snapshot
//! and returns a command vector. Everything here is the host's business.

use cyberspine_core::rig::Fingerprint;

/// The sensor buffers the host lends to [`Backend::sample`] for one tick.
///
/// Their lengths come from the image, not from the backend: `f` is as long as
/// the image's `SensorSource::Backend` entries in `sensors_f`, and `i` as long as
/// those in `sensors_i` (`docs/isa.md` §9.2).
pub struct SensorValues<'s> {
    pub f: &'s mut [f32],
    pub i: &'s mut [i32],
}

/// A controlled body: the thing whose state the program observes and drives.
///
/// The host calls [`Backend::sample`] once per tick, then [`Backend::commit`]
/// and [`Backend::step`]. The order is the host's contract, not the backend's
/// concern — a backend may assume the three are called once each per tick, in
/// that order.
///
/// A backend does **not** declare how big it is. The fingerprint already hashes
/// the rig's shape, so a second declaration would be a second truth
/// (`docs/isa.md` §9.2, §13.3).
pub trait Backend {
    /// The rig this backend implements, computed once at construction.
    ///
    /// The host compares it with the image's before anything runs. It guards
    /// against a stale or wrong manifest, not against a hostile backend: code
    /// that lies here is already running on the device (§13.3).
    fn fingerprint(&self) -> Fingerprint;

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
