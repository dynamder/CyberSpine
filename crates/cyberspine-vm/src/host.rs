//! The tick loop (`docs/isa.md` §8.4): sample, resume, merge, commit, step.
//!
//! This is the only place that owns `now`, load order and the command merge.
//! The VM executes one instance at a time and never sees any of it.
//!
//! Before any of that, loading is where an image meets the body it was assembled
//! for: the fingerprints are compared, and a mismatch is refused
//! (`docs/isa.md` §13.3). Nothing here compares names — they became indices at
//! assembly time.
//!
//! # Two types, because a clock needs an image
//!
//! A [`Host`] holds a backend and nothing else: it has no clock, so it cannot
//! tick, and the "tick before any image" state is not representable. [`Host::load`]
//! consumes it and yields a [`Session`], which is the ticking type. A session never
//! has zero images, and its `hz`/`dt`/`shape` are plain values rather than options.
//!
//! # The tick loop is the body's lifeline
//!
//! Ticking is what drives the body. An action that has halted stops *publishing*,
//! but its effect on the body does not vanish: the merged vector becomes all
//! `None`, and `None` means "no instance published this muscle" — the backend
//! keeps the last commanded value (`docs/isa.md` §9.2). That is what makes "raise
//! a hand and hold it" a real state rather than a program that has to keep running.
//! So `tick` never short-circuits: a host with no instances still samples, commits
//! and steps, and only the per-instance work disappears.

use core::marker::PhantomData;

use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::binfmt::plan::{SensorFSource, SensorISource};
use cyberspine_core::exec::{Fault, Instance, Program, Sensors, Status};
use cyberspine_core::isa::clock::Hz;
use cyberspine_core::rig::{BoundFingerprint, Fingerprint};

use crate::backend::{Backend, BackendError, Shape, Snapshot};

/// How many one-tick command vectors a fresh trace holds.
pub const DEFAULT_TRACE_CAPACITY: usize = 4096;

/// A stable handle on a loaded instance. Ids are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceId(u64);

impl core::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Why an instance is gone (`docs/isa.md` §G12).
///
/// Removal is silent by nature — the slot disappears — so the reason has to
/// outlive it. A fault is a bug signal, never a normal ending: it has to reach
/// the caller instead of dying with the bookkeeping that drops the instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultRecord {
    pub instance: InstanceId,
    pub fault: Fault,
}

/// One tick's merged command vector, borrowed out of the trace ring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickRef<'t> {
    pub now: u64,
    pub commands: &'t [Option<f32>],
}

/// The host's own refusals, all of them about compatibility rather than validity:
/// a `VerifiedImage` is a legal action, but it may still be an action for another
/// body, another clock, or another shape.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("the image was assembled for rig {declared}, this backend implements {backend}")]
    RigMismatch {
        declared: Fingerprint,
        backend: BoundFingerprint,
    },
    #[error(
        "the image carries no rig fingerprint; load it with a permissive host (`Host::permissive`) to run it anyway"
    )]
    Unbound,
    #[error("the image's {table} table has {declared} entries, the backend reports {host}")]
    ShapeMismatch {
        table: &'static str,
        declared: usize,
        host: usize,
    },
    #[error("image declares {declared} Hz, the session is already ticking at {host} Hz")]
    Hz { declared: u32, host: u32 },
}

/// What a host demands of an image's identity.
///
/// The fingerprint proves the image was assembled for *this* body; it does not
/// and cannot prove anything about the body's honesty (`docs/isa.md` §13.3). The
/// policy decides what to do when the only thing on offer is "no identity at all".
pub trait RigPolicy {
    fn check(declared: Fingerprint, backend: BoundFingerprint) -> Result<(), LoadError>;
}

/// The default: the image must name this body, and a body-less image is refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Strict;

/// Accepts an image that names no body.
///
/// For paths that legitimately have no rig: tests, and firmware that runs
/// hand-built images. A real host must not use this — under [`Strict`] the
/// question "was this action built for this body" is a machine check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Permissive;

impl RigPolicy for Strict {
    fn check(declared: Fingerprint, backend: BoundFingerprint) -> Result<(), LoadError> {
        if declared == backend.fingerprint() {
            return Ok(());
        }
        if declared.is_unbound() {
            return Err(LoadError::Unbound);
        }
        Err(LoadError::RigMismatch { declared, backend })
    }
}

impl RigPolicy for Permissive {
    fn check(_declared: Fingerprint, _backend: BoundFingerprint) -> Result<(), LoadError> {
        Ok(())
    }
}

/// A backend and the policy it will load images under: no images, no clock, no
/// `tick`. The only thing it can do is [`Host::load`].
pub struct Host<B: Backend, P: RigPolicy = Strict> {
    backend: B,
    _policy: PhantomData<P>,
}

/// Hand-written so a caller can `expect` on a load result without the backend
/// having to be printable: a host is nothing but a backend and a policy.
impl<B: Backend, P: RigPolicy> core::fmt::Debug for Host<B, P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

impl<B: Backend> Host<B, Strict> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            _policy: PhantomData,
        }
    }

    /// Accept images assembled without a rig, and keep everything else strict.
    ///
    /// One-way on purpose: once a host has loaded a body-less image, "the host
    /// only ever ran images for this body" is no longer a fact, and un-saying it
    /// would not make it true.
    pub fn permissive(self) -> Host<B, Permissive> {
        Host {
            backend: self.backend,
            _policy: PhantomData,
        }
    }
}

impl<B: Backend, P: RigPolicy> Host<B, P> {
    /// Load the first action, turning the host into a ticking [`Session`].
    ///
    /// The instance id comes back alongside the session — it is the handle the
    /// caller has on the action it just started, and there is no other way to
    /// name it.
    ///
    /// On failure the host comes back untouched: the image was wrong, the host
    /// was not, and a caller that wants to try another image should not have to
    /// rebuild the backend to do it.
    pub fn load(
        self,
        image: VerifiedImage,
    ) -> Result<(Session<B, P>, InstanceId), (Self, LoadError)> {
        let shape = self.backend.shape();
        if let Err(error) = admit::<B, P>(&self.backend, shape, &image) {
            return Err((self, error));
        }

        let hz = image.header().hz;
        let mut session = Session {
            backend: self.backend,
            shape,
            hz,
            dt: hz.dt(),
            slots: Vec::new(),
            merge_order: Vec::new(),
            faults: Vec::new(),
            rig_f: vec![0.0; shape.sensors_f],
            rig_i: vec![0; shape.sensors_i],
            rig_muscles: vec![0.0; shape.muscles],
            merged: vec![None; shape.muscles],
            now: 0,
            accumulator: 0.0,
            max_ticks_per_frame: 8,
            overruns: 0,
            next_id: 0,
            tracing: false,
            trace: Trace::new(0, shape.muscles),
            _policy: PhantomData,
        };
        // `admit` already passed for this exact image, so the slot push cannot
        // fail; it goes through the same private path a later `load` uses.
        let id = session.push(image);
        Ok((session, id))
    }
}

/// The identity and shape checks, in one place so a first load and a later load
/// cannot disagree about what "fits" means.
fn admit<B: Backend, P: RigPolicy>(
    backend: &B,
    host: Shape,
    image: &VerifiedImage,
) -> Result<(), LoadError> {
    P::check(image.header().rig, backend.fingerprint())?;
    let (sensors_f, sensors_i) = image.plan().backend_counts();
    let image = Shape {
        muscles: image.muscles().len(),
        sensors_f,
        sensors_i,
    };
    check_shape(host, image)
}

fn check_shape(host: Shape, image: Shape) -> Result<(), LoadError> {
    for (table, declared, host) in [
        ("muscles", image.muscles, host.muscles),
        ("sensors_f", image.sensors_f, host.sensors_f),
        ("sensors_i", image.sensors_i, host.sensors_i),
    ] {
        if declared != host {
            return Err(LoadError::ShapeMismatch {
                table,
                declared,
                host,
            });
        }
    }
    Ok(())
}

struct Slot {
    id: InstanceId,
    layer: u16,
    image: VerifiedImage,
    instance: Instance,
    sensors_f: Vec<f32>,
    sensors_i: Vec<i32>,
}

impl Slot {
    fn fill(&mut self, rig_f: &[f32], rig_i: &[i32], rig_muscles: &[f32], now: u64, dt: f32) {
        let plan = self.image.plan();
        // `zip` rather than indexing: if the plan and the buffers ever disagreed
        // the fill would truncate instead of panicking, and the disagreement is
        // already impossible because both come from the same table lengths.
        for (slot, source) in self.sensors_f.iter_mut().zip(&plan.f) {
            *slot = match *source {
                SensorFSource::Backend(observable) => rig_f[observable],
                SensorFSource::Muscle(muscle) => rig_muscles[muscle],
                SensorFSource::Dt => dt,
            };
        }
        for (slot, source) in self.sensors_i.iter_mut().zip(&plan.i) {
            *slot = match *source {
                SensorISource::Backend(observable) => rig_i[observable],
                SensorISource::Tick => now as i32,
            };
        }
    }
}

/// Several instances over one rig, advanced together.
pub struct Session<B: Backend, P: RigPolicy = Strict> {
    backend: B,
    shape: Shape,
    hz: Hz,
    dt: f32,
    slots: Vec<Slot>,
    merge_order: Vec<usize>,
    faults: Vec<FaultRecord>,
    rig_f: Vec<f32>,
    rig_i: Vec<i32>,
    rig_muscles: Vec<f32>,
    /// The tick's command vector, reused so the loop allocates nothing.
    merged: Vec<Option<f32>>,
    now: u64,
    accumulator: f64,
    max_ticks_per_frame: u32,
    overruns: u64,
    next_id: u64,
    tracing: bool,
    trace: Trace,
    _policy: PhantomData<P>,
}

impl<B: Backend, P: RigPolicy> core::fmt::Debug for Session<B, P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Session")
            .field("hz", &self.hz)
            .field("shape", &self.shape)
            .field("now", &self.now)
            .field("instances", &self.slots.len())
            .finish_non_exhaustive()
    }
}

impl<B: Backend, P: RigPolicy> Session<B, P> {
    /// Load another action into this session.
    ///
    /// The first image fixed the clock; every later image has to agree with it,
    /// with the backend's shape and with the identity policy. Names became
    /// indices at assembly time, so nothing here compares strings.
    pub fn load(&mut self, image: VerifiedImage) -> Result<InstanceId, LoadError> {
        admit::<B, P>(&self.backend, self.shape, &image)?;
        let hz = image.header().hz;
        if hz != self.hz {
            return Err(LoadError::Hz {
                declared: hz.get(),
                host: self.hz.get(),
            });
        }
        Ok(self.push(image))
    }

    /// Admit an image whose checks have already passed.
    fn push(&mut self, image: VerifiedImage) -> InstanceId {
        let instance = {
            let program = Program::new(&image);
            Instance::new(&program)
        };
        let id = InstanceId(self.next_id);
        self.next_id += 1;

        self.slots.push(Slot {
            id,
            layer: image.header().layer,
            sensors_f: vec![0.0; image.sensors_f().len()],
            sensors_i: vec![0; image.sensors_i().len()],
            image,
            instance,
        });
        self.rebuild_merge_order();
        id
    }

    /// One tick: sample, resume everything due, merge, hand over, step.
    ///
    /// Never short-circuits, not even with no instances: the last three calls are
    /// how the body keeps holding what it was last told.
    ///
    /// The body is the only fallible part. An `Err` ends the tick on the spot:
    /// instances already resumed keep their state, but nothing is merged, nothing
    /// reaches the body and the clock does not advance. The body is gone at that
    /// point, so a caller must stop ticking this session.
    pub fn tick(&mut self) -> Result<(), BackendError> {
        self.sample()?;
        self.resume_due();

        self.merge();
        self.backend.commit(&self.merged, self.dt)?;
        self.backend.step(self.dt)?;

        if self.tracing {
            let now = self.now;
            self.trace.push(now, &self.merged);
        }

        self.now += 1;
        self.remove_terminated();
        Ok(())
    }

    /// Consume wall-clock seconds and run the whole ticks they contain.
    ///
    /// The remainder stays here: it is host timing, and it never enters a tick.
    /// A frame that asks for more than [`Session::set_max_ticks_per_frame`] ticks
    /// drops its remainder and counts one overrun, so a slow frame can never
    /// spiral. A failed tick ends the frame immediately, so the caller learns
    /// about the body instead of losing it behind a loop.
    pub fn advance(&mut self, seconds: f64) -> Result<(), BackendError> {
        self.accumulator += seconds.max(0.0);
        let dt = f64::from(self.dt);
        let mut ticks = 0;
        while self.accumulator >= dt {
            if ticks >= self.max_ticks_per_frame {
                self.overruns += 1;
                self.accumulator = 0.0;
                break;
            }
            self.tick()?;
            self.accumulator -= dt;
            ticks += 1;
        }
        Ok(())
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn dt(&self) -> f32 {
        self.dt
    }

    pub fn hz(&self) -> Hz {
        self.hz
    }

    /// The shape every image in this session had to agree with.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    pub fn instances(&self) -> usize {
        self.slots.len()
    }

    /// No instances at all — everything halted or faulted.
    ///
    /// Observation only: an idle session still ticks, because the body still
    /// needs driving. A caller uses this to decide whether to keep a frame loop
    /// running, not to skip [`Session::tick`].
    pub fn is_idle(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
        self.slots
            .iter()
            .find(|slot| slot.id == id)
            .map(|slot| &slot.instance)
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Start or stop recording the per-tick command vector.
    ///
    /// Turning tracing on allocates the ring ([`DEFAULT_TRACE_CAPACITY`] ticks,
    /// or whatever [`Session::set_trace_capacity`] last asked for); turning it off
    /// clears it and keeps the allocation.
    pub fn set_tracing(&mut self, tracing: bool) {
        self.tracing = tracing;
        if tracing {
            if self.trace.capacity() == 0 {
                self.trace = Trace::new(DEFAULT_TRACE_CAPACITY, self.shape.muscles);
            }
        } else {
            self.trace.clear();
        }
    }

    pub fn tracing(&self) -> bool {
        self.tracing
    }

    /// Resize the ring. Zero turns recording into a no-op that still counts
    /// [`Session::trace_dropped`].
    pub fn set_trace_capacity(&mut self, ticks: usize) {
        self.trace = Trace::new(ticks, self.shape.muscles);
    }

    /// The recorded ticks, oldest first.
    pub fn trace(&self) -> TraceIter<'_> {
        self.trace.iter()
    }

    /// How many tick records were overwritten by later ones.
    ///
    /// The ring is a sample, not a log: a consumer that does not drain it will
    /// see this number grow, and that is the honest signal that its window is
    /// shorter than the run.
    pub fn trace_dropped(&self) -> u64 {
        self.trace.dropped()
    }

    /// Every fault since the last [`Session::take_faults`], oldest first.
    pub fn faults(&self) -> &[FaultRecord] {
        &self.faults
    }

    /// Drain the fault log: the records the caller has not been told about yet.
    ///
    /// Drained rather than capped — a fault is rare, and dropping the oldest
    /// record would be exactly the silence this log exists to rule out. A caller
    /// that never drains it therefore grows without bound.
    pub fn take_faults(&mut self) -> Vec<FaultRecord> {
        core::mem::take(&mut self.faults)
    }

    pub fn set_max_ticks_per_frame(&mut self, ticks: u32) {
        self.max_ticks_per_frame = ticks.max(1);
    }

    fn sample(&mut self) -> Result<(), BackendError> {
        let mut snapshot = Snapshot {
            f: &mut self.rig_f,
            i: &mut self.rig_i,
            muscles: &mut self.rig_muscles,
        };
        self.backend.sample(&mut snapshot)
    }

    fn resume_due(&mut self) {
        let now = self.now;
        let dt = self.dt;
        let rig_f = &self.rig_f;
        let rig_i = &self.rig_i;
        let rig_muscles = &self.rig_muscles;

        for slot in &mut self.slots {
            if slot.instance.wake_at() > now {
                continue;
            }
            slot.fill(rig_f, rig_i, rig_muscles, now, dt);

            let Slot {
                image,
                instance,
                sensors_f,
                sensors_i,
                ..
            } = slot;
            let program = Program::new(image);
            instance.resume(&program, &Sensors::new(sensors_f, sensors_i), now);
        }
    }

    /// Rebuild this tick's command vector in place: `None` for every muscle no
    /// instance published, and nothing allocated.
    fn merge(&mut self) {
        self.merged.fill(None);
        for &index in &self.merge_order {
            let instance = &self.slots[index].instance;
            // G12: a fault takes its contributions with it, in the same tick.
            if instance.status() == Status::Faulted {
                continue;
            }
            for (muscle, slot) in self.merged.iter_mut().enumerate() {
                if let Some(value) = instance.published(muscle) {
                    *slot = Some(value);
                }
            }
        }
    }

    /// Drop the instances that ended this tick, keeping their reasons.
    ///
    /// `Halted` is a normal ending. `Faulted` is a bug signal, and this is the
    /// last moment its reason exists — the slot is about to disappear
    /// (`docs/isa.md` §G12).
    fn remove_terminated(&mut self) {
        let before = self.slots.len();
        let mut faults = Vec::new();
        self.slots.retain(|slot| {
            if slot.instance.status() == Status::Suspended {
                return true;
            }
            if let Some(fault) = slot.instance.fault() {
                faults.push(FaultRecord {
                    instance: slot.id,
                    fault,
                });
            }
            false
        });
        self.faults.append(&mut faults);
        if self.slots.len() != before {
            self.rebuild_merge_order();
        }
    }

    fn rebuild_merge_order(&mut self) {
        self.merge_order.clear();
        self.merge_order.extend(0..self.slots.len());
        self.merge_order
            .sort_by_key(|&index| self.slots[index].layer);
    }
}

/// A fixed-capacity ring of tick records.
///
/// Commands live in one flat arena rather than a `Vec` per tick, so recording a
/// tick copies bytes and never allocates.
struct Trace {
    now: Vec<u64>,
    commands: Vec<Option<f32>>,
    muscles: usize,
    len: usize,
    head: usize,
    dropped: u64,
}

impl Trace {
    fn new(capacity: usize, muscles: usize) -> Self {
        Self {
            now: vec![0; capacity],
            commands: vec![None; capacity * muscles],
            muscles,
            len: 0,
            head: 0,
            dropped: 0,
        }
    }

    fn capacity(&self) -> usize {
        self.now.len()
    }

    fn dropped(&self) -> u64 {
        self.dropped
    }

    fn clear(&mut self) {
        self.len = 0;
        self.head = 0;
        self.dropped = 0;
    }

    fn push(&mut self, now: u64, commands: &[Option<f32>]) {
        let capacity = self.capacity();
        if capacity == 0 {
            self.dropped += 1;
            return;
        }
        let start = self.head * self.muscles;
        self.now[self.head] = now;
        self.commands[start..start + self.muscles].copy_from_slice(commands);
        if self.len == capacity {
            self.dropped += 1;
        } else {
            self.len += 1;
        }
        self.head = (self.head + 1) % capacity;
    }

    fn iter(&self) -> TraceIter<'_> {
        TraceIter {
            trace: self,
            index: 0,
        }
    }

    fn get(&self, index: usize) -> TickRef<'_> {
        let capacity = self.capacity();
        let oldest = (self.head + capacity - self.len) % capacity;
        let physical = (oldest + index) % capacity;
        let start = physical * self.muscles;
        TickRef {
            now: self.now[physical],
            commands: &self.commands[start..start + self.muscles],
        }
    }
}

/// The recorded ticks, oldest first.
pub struct TraceIter<'t> {
    trace: &'t Trace,
    index: usize,
}

impl<'t> Iterator for TraceIter<'t> {
    type Item = TickRef<'t>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.trace.len {
            return None;
        }
        let tick = self.trace.get(self.index);
        self.index += 1;
        Some(tick)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.trace.len - self.index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for TraceIter<'_> {}
