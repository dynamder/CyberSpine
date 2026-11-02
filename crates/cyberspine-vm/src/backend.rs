//! The boundary between the host and a controlled body (`docs/isa.md` §9).
//!
//! `cyberspine-core` never names this trait: the VM sees only a sensor snapshot
//! and returns a command vector. Everything here is the host's business.
//!
//! The shape is deliberately "one snapshot in, one command vector out" per tick:
//! every observation travels in a single [`Snapshot`], so the host never
//! interrogates the body piecewise.

use cyberspine_core::rig::Fingerprint;

/// Everything the body reports about itself at one instant.
///
/// The three arrays are the body's whole observable surface: its declared
/// observables of each type, and the current value of every muscle. Their
/// lengths come from the image, not from the backend (`docs/isa.md` §9.2): `f`
/// and `i` are as long as the image's `SensorSource::Backend` entries, and
/// `muscles` as long as its muscle table.
///
/// A muscle read-back is a sensor like any other (`SensorSource::Muscle`), so it
/// travels with the rest of the snapshot. That is what keeps one tick to one
/// instant: if the body were asked for each read-back separately, a driver that
/// polls its device per call would report two sensors taken at different moments
/// in the same tick.
pub struct Snapshot<'s> {
    pub f: &'s mut [f32],
    pub i: &'s mut [i32],
    pub muscles: &'s mut [f32],
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

    /// Report the body as it is right now, filling every slot of `out`.
    ///
    /// Called *before* any instance runs, so a program never observes an
    /// excitation it made this tick. It is a read: it must not advance the body
    /// or the clock. It still takes `&mut self` because producing the snapshot
    /// may need exclusive access — polling a device, or refreshing a cache.
    fn sample(&mut self, out: &mut Snapshot<'_>);

    /// Take this tick's merged command vector. `None` means no instance
    /// published that muscle. Must not advance the body.
    fn commit(&mut self, commands: &[Option<f32>], dt: f32);

    /// Advance the body by one fixed tick. The only place physical state moves.
    fn step(&mut self, dt: f32);
}
