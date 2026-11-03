//! The boundary between the host and a controlled body (`docs/isa.md` §9).
//!
//! `cyberspine-core` never names this trait: the VM sees only a sensor snapshot
//! and returns a command vector. Everything here is the host's business.
//!
//! The shape is deliberately "one snapshot in, one command vector out" per tick:
//! every observation travels in a single [`Snapshot`], so the host never
//! interrogates the body piecewise.

use cyberspine_core::rig::BoundFingerprint;

/// Why a backend could not finish this tick.
///
/// A body is not always a Rust value in this process: a serial port closes, a
/// device is unplugged, a dynamic library returns garbage, a socket times out.
/// Those failures had nowhere to go, so a backend had to panic or silently lie.
/// One concrete error type keeps `Box<dyn Backend>` possible while still
/// carrying a concrete adapter's error inside it.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct BackendError {
    message: Box<str>,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl BackendError {
    /// Wrap a concrete failure, keeping its text and its source chain.
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            message: error.to_string().into_boxed_str(),
            source: Some(Box::new(error)),
        }
    }

    /// A failure with no error type behind it.
    pub fn message(message: impl Into<Box<str>>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }
}

/// How much a backend has to report: the three snapshot lengths.
///
/// This is a *view* of the backend's rig, not a second truth: it is derived once
/// from the same [`Rig`](cyberspine_core::rig::Rig) the fingerprint hashes. It
/// exists so a mismatched image is refused at load time instead of making
/// [`Backend::sample`] write past — or short of — the snapshot it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub muscles: usize,
    pub sensors_f: usize,
    pub sensors_i: usize,
}

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
/// that order. That holds even when the host holds no instances: an idle tick
/// still drives the body, so a held pose stays held.
///
/// A backend does **not** declare its *names*: the fingerprint already hashes the
/// rig, and a second name table would be a second truth (`docs/isa.md` §9.2,
/// §13.3). It does report its counts, which is the one part of the rig the host
/// needs before it can size a snapshot.
///
/// # Failure
///
/// Every tick call is fallible. Returning [`BackendError`] says the body cannot
/// be driven any more: the host aborts the tick there, and the session must not
/// be ticked again. A failed call is the last call of its tick, and a body may
/// be left in any state by it — this is not a retryable signal. An adapter that
/// owns a link latches the failure and returns it on every later call.
///
/// In-process bodies normally return `Ok(())`, but the signature is where a real
/// device's "the serial port is gone" belongs instead of a panic.
pub trait Backend {
    /// The rig this backend implements, computed once at construction.
    ///
    /// The host compares it with the image's before anything runs. It guards
    /// against a stale or wrong manifest, not against a hostile backend: code
    /// that lies here is already running on the device (§13.3).
    ///
    /// The return type is [`BoundFingerprint`], not [`Fingerprint`]: a body
    /// always has an identity, so "no fingerprint at all" is not a state a
    /// backend can report.
    fn fingerprint(&self) -> BoundFingerprint;

    /// The snapshot lengths this body fills — the same rig, counted.
    fn shape(&self) -> Shape;

    /// Report the body as it is right now, filling every slot of `out`.
    ///
    /// Called *before* any instance runs, so a program never observes an
    /// excitation it made this tick. It is a read: it must not advance the body
    /// or the clock. It still takes `&mut self` because producing the snapshot
    /// may need exclusive access — polling a device, or refreshing a cache that
    /// [`Backend::step`] then reads.
    ///
    /// On error the tick stops here: no instance runs, and `out` may be
    /// half-filled, which is why the host only reads it back after `Ok`.
    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError>;

    /// Take this tick's merged command vector.
    ///
    /// `None` means no instance published that muscle this tick — it does **not**
    /// mean "release it". A backend must leave that muscle's target alone, which
    /// is what lets an action halt and leave the body holding its last commanded
    /// state (`docs/isa.md` §9.2). Releasing is a command like any other:
    /// `Some(0.0)`.
    ///
    /// Must not advance the body.
    fn commit(&mut self, commands: &[Option<f32>], dt: f32) -> Result<(), BackendError>;

    /// Advance the body by one fixed tick. The only place physical state moves.
    fn step(&mut self, dt: f32) -> Result<(), BackendError>;
}

/// A boxed body is a body: this is what lets a host hold
/// `Box<dyn Backend>` and stay non-generic above the platform layer.
impl<T: Backend + ?Sized> Backend for Box<T> {
    fn fingerprint(&self) -> BoundFingerprint {
        (**self).fingerprint()
    }

    fn shape(&self) -> Shape {
        (**self).shape()
    }

    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        (**self).sample(out)
    }

    fn commit(&mut self, commands: &[Option<f32>], dt: f32) -> Result<(), BackendError> {
        (**self).commit(commands, dt)
    }

    fn step(&mut self, dt: f32) -> Result<(), BackendError> {
        (**self).step(dt)
    }
}
