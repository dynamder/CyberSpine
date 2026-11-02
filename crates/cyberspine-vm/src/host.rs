//! The tick loop (`docs/isa.md` §8.4): sample, resume, merge, commit, step.
//!
//! This is the only place that owns `now`, load order and the command merge.
//! The VM executes one instance at a time and never sees any of it.
//!
//! Before any of that, [`Host::load`] is where an image meets the body it was
//! assembled for: the fingerprints are compared, and a mismatch is refused
//! (`docs/isa.md` §13.3). Nothing here compares names — they became indices at
//! assembly time.

use cyberspine_core::binfmt::image::{Sensor, SensorSource};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::exec::{Fault, Instance, Program, Sensors, Status};
use cyberspine_core::rig::Fingerprint;

use crate::backend::{Backend, Snapshot};

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

/// One tick's merged command vector, kept when tracing is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub now: u64,
    pub commands: Vec<Option<f32>>,
}

/// The host own refusals, all of them about compatibility rather than validity:
/// a `VerifiedImage` is a legal action, but it may still be an action for another
/// body, another clock, or another shape.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("the image was assembled for rig {declared}, this backend implements {backend}")]
    RigMismatch {
        declared: Fingerprint,
        backend: Fingerprint,
    },
    #[error("the image carries no rig fingerprint; call `Host::allow_unbound` to load it anyway")]
    Unbound,
    #[error(
        "the image's {table} table has {declared} entries, the host is already sized for {host}"
    )]
    ShapeMismatch {
        table: &'static str,
        declared: usize,
        host: usize,
    },
    #[error("image declares {declared} Hz, the host is already ticking at {host} Hz")]
    Hz { declared: u32, host: u32 },
}

/// How big the loaded image is, in the numbers the host needs: the merge
/// vector, the two observable buffers, and the muscle table a backend fills.
///
/// A backend does not declare these (`docs/isa.md` §9.2) — the image does, and
/// every image loaded here shares one fingerprint, so it declares the same
/// shape every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scale {
    muscles: usize,
    sensors_f: usize,
    sensors_i: usize,
}

impl Scale {
    /// The image's own counts. M3/M4/M6 already guarantee that the read-backs
    /// match the muscles, so "how many back-end observables" is all that is left
    /// to count.
    fn of(image: &VerifiedImage) -> Self {
        let backend = |sensor: &&Sensor| sensor.source == SensorSource::Backend;
        Self {
            muscles: image.muscles.len(),
            sensors_f: image.sensors_f.iter().filter(backend).count(),
            sensors_i: image.sensors_i.iter().filter(backend).count(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SourceF {
    /// The k-th `SensorSource::Backend` entry in `sensors_f`.
    Backend(usize),
    /// The k-th `SensorSource::Muscle` entry: that muscle's read-back.
    Muscle(usize),
    Dt,
    Tick,
}

#[derive(Debug, Clone, Copy)]
enum SourceI {
    /// The k-th `SensorSource::Backend` entry in `sensors_i`.
    Backend(usize),
    Tick,
}

struct Slot {
    id: InstanceId,
    layer: u16,
    image: VerifiedImage,
    instance: Instance,
    sensors_f: Vec<f32>,
    sensors_i: Vec<i32>,
    plan_f: Vec<SourceF>,
    plan_i: Vec<SourceI>,
}

impl Slot {
    fn fill(&mut self, rig_f: &[f32], rig_i: &[i32], rig_muscles: &[f32], now: u64, dt: f32) {
        for (index, source) in self.plan_f.iter().enumerate() {
            self.sensors_f[index] = match *source {
                SourceF::Backend(observable) => rig_f[observable],
                SourceF::Muscle(muscle) => rig_muscles[muscle],
                SourceF::Dt => dt,
                SourceF::Tick => now as i32 as f32,
            };
        }
        for (index, source) in self.plan_i.iter().enumerate() {
            self.sensors_i[index] = match *source {
                SourceI::Backend(observable) => rig_i[observable],
                SourceI::Tick => now as i32,
            };
        }
    }
}

/// Several instances over one rig, advanced together.
pub struct Host<B: Backend> {
    backend: B,
    scale: Option<Scale>,
    slots: Vec<Slot>,
    merge_order: Vec<usize>,
    faults: Vec<FaultRecord>,
    rig_f: Vec<f32>,
    rig_i: Vec<i32>,
    rig_muscles: Vec<f32>,
    allow_unbound: bool,
    now: u64,
    hz: Option<u32>,
    dt: f32,
    accumulator: f64,
    max_ticks_per_frame: u32,
    overruns: u64,
    next_id: u64,
    tracing: bool,
    trace: Vec<Tick>,
}

impl<B: Backend> Host<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            scale: None,
            slots: Vec::new(),
            merge_order: Vec::new(),
            faults: Vec::new(),
            rig_f: Vec::new(),
            rig_i: Vec::new(),
            rig_muscles: Vec::new(),
            allow_unbound: false,
            now: 0,
            hz: None,
            dt: 0.0,
            accumulator: 0.0,
            max_ticks_per_frame: 8,
            overruns: 0,
            next_id: 0,
            tracing: false,
            trace: Vec::new(),
        }
    }

    /// Load one action.
    ///
    /// The image must have been assembled for this backend's rig: equal
    /// fingerprints, hence equal shape and clock. Names became indices at
    /// assembly time; nothing here compares strings.
    ///
    /// An image assembled without a rig carries [`Fingerprint::UNBOUND`] and is
    /// refused, because nothing ties it to this body. Tests and firmware that
    /// never see a host may opt out with [`Host::allow_unbound`].
    pub fn load(&mut self, image: VerifiedImage) -> Result<InstanceId, LoadError> {
        self.check_rig(&image)?;
        let scale = Scale::of(&image);
        self.fit_scale(scale)?;

        let hz = image.header.hz;
        match self.hz {
            None => {
                self.hz = Some(hz);
                self.dt = 1.0 / hz as f32;
            }
            Some(host) if host != hz => {
                return Err(LoadError::Hz { declared: hz, host });
            }
            Some(_) => {}
        }

        let plan_f = float_plan(&image.sensors_f);
        let plan_i = int_plan(&image.sensors_i);

        let instance = {
            let program = Program::new(&image);
            Instance::new(&program)
        };
        let id = InstanceId(self.next_id);
        self.next_id += 1;

        self.slots.push(Slot {
            id,
            layer: image.header.layer,
            sensors_f: vec![0.0; image.sensors_f.len()],
            sensors_i: vec![0; image.sensors_i.len()],
            image,
            instance,
            plan_f,
            plan_i,
        });
        self.rebuild_merge_order();
        Ok(id)
    }

    /// Accept an image that was assembled without a rig.
    ///
    /// Only for paths that legitimately have no rig: tests, and firmware that
    /// runs hand-built images. A real host must not call this — it is what makes
    /// "this action was built for this body" a machine check.
    pub fn allow_unbound(&mut self) {
        self.allow_unbound = true;
    }

    /// One tick: sample, resume everything due, merge, hand over, step.
    pub fn tick(&mut self) {
        if self.hz.is_none() {
            return;
        }

        self.sample();
        self.resume_due();

        let merged = self.merge();
        self.backend.commit(&merged, self.dt);
        self.backend.step(self.dt);

        if self.tracing {
            self.trace.push(Tick {
                now: self.now,
                commands: merged,
            });
        }

        self.now += 1;
        self.remove_terminated();
    }

    /// Consume wall-clock seconds and run the whole ticks they contain.
    ///
    /// The remainder stays here: it is host timing, and it never enters a tick.
    /// A frame that asks for more than [`Host::set_max_ticks_per_frame`] ticks
    /// drops its remainder and counts one overrun, so a slow frame can never
    /// spiral.
    pub fn advance(&mut self, seconds: f64) {
        if self.hz.is_none() {
            return;
        }
        self.accumulator += seconds.max(0.0);
        let dt = f64::from(self.dt);
        let mut ticks = 0;
        while self.accumulator >= dt {
            if ticks >= self.max_ticks_per_frame {
                self.overruns += 1;
                self.accumulator = 0.0;
                break;
            }
            self.tick();
            self.accumulator -= dt;
            ticks += 1;
        }
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn dt(&self) -> f32 {
        self.dt
    }

    pub fn hz(&self) -> Option<u32> {
        self.hz
    }

    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    pub fn instances(&self) -> usize {
        self.slots.len()
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

    pub fn set_tracing(&mut self, tracing: bool) {
        self.tracing = tracing;
        if !tracing {
            self.trace.clear();
        }
    }

    pub fn trace(&self) -> &[Tick] {
        &self.trace
    }

    /// Every fault since the last [`Host::take_faults`], oldest first.
    pub fn faults(&self) -> &[FaultRecord] {
        &self.faults
    }

    /// Drain the fault log: the records the caller has not been told about yet.
    ///
    /// Drained rather than capped — a fault is rare, and dropping the oldest
    /// record would be exactly the silence this log exists to rule out.
    pub fn take_faults(&mut self) -> Vec<FaultRecord> {
        core::mem::take(&mut self.faults)
    }

    pub fn set_max_ticks_per_frame(&mut self, ticks: u32) {
        self.max_ticks_per_frame = ticks.max(1);
    }

    /// The image's fingerprint against the backend's: equal, or refused.
    ///
    /// An all-zero fingerprint means the image has no rig at all, which is a
    /// different failure from a mismatch — it says "unbound", not "another
    /// body", and only an explicit opt-out lets it through.
    fn check_rig(&self, image: &VerifiedImage) -> Result<(), LoadError> {
        let declared = image.header.rig;
        let backend = self.backend.fingerprint();
        if declared == backend {
            return Ok(());
        }
        if declared.is_unbound() {
            return if self.allow_unbound {
                Ok(())
            } else {
                Err(LoadError::Unbound)
            };
        }
        Err(LoadError::RigMismatch { declared, backend })
    }

    /// The first image fixes the shape of everything the host allocates.
    ///
    /// Later images only have to agree with it. With equal fingerprints they
    /// always do; this check is what an unbound image (which proves nothing
    /// about its shape) is measured against.
    fn fit_scale(&mut self, scale: Scale) -> Result<(), LoadError> {
        match self.scale {
            None => {
                self.rig_f = vec![0.0; scale.sensors_f];
                self.rig_i = vec![0; scale.sensors_i];
                self.rig_muscles = vec![0.0; scale.muscles];
                self.scale = Some(scale);
                Ok(())
            }
            Some(host) if scale.sensors_f != host.sensors_f => Err(LoadError::ShapeMismatch {
                table: "sensors_f",
                declared: scale.sensors_f,
                host: host.sensors_f,
            }),
            Some(host) if scale.sensors_i != host.sensors_i => Err(LoadError::ShapeMismatch {
                table: "sensors_i",
                declared: scale.sensors_i,
                host: host.sensors_i,
            }),
            Some(host) if scale.muscles != host.muscles => Err(LoadError::ShapeMismatch {
                table: "muscles",
                declared: scale.muscles,
                host: host.muscles,
            }),
            Some(_) => Ok(()),
        }
    }

    fn sample(&mut self) {
        let mut snapshot = Snapshot {
            f: &mut self.rig_f,
            i: &mut self.rig_i,
            muscles: &mut self.rig_muscles,
        };
        self.backend.sample(&mut snapshot);
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

    fn merge(&self) -> Vec<Option<f32>> {
        let muscles = self.scale.map_or(0, |scale| scale.muscles);
        let mut merged = vec![None; muscles];
        for &index in &self.merge_order {
            let instance = &self.slots[index].instance;
            // G12: a fault takes its contributions with it, in the same tick.
            if instance.status() == Status::Faulted {
                continue;
            }
            for (muscle, slot) in merged.iter_mut().enumerate() {
                if let Some(value) = instance.published(muscle) {
                    *slot = Some(value);
                }
            }
        }
        merged
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

/// The k-th `Backend` entry of `sensors_f` is the backend's k-th observable
/// (`docs/isa.md` §9.2), whatever the read-back and built-in entries around it.
fn float_plan(sensors: &[Sensor]) -> Vec<SourceF> {
    let mut backends = 0usize;
    let mut muscles = 0usize;
    sensors
        .iter()
        .map(|sensor| match sensor.source {
            SensorSource::Muscle => {
                let muscle = muscles;
                muscles += 1;
                SourceF::Muscle(muscle)
            }
            SensorSource::Backend => {
                let observable = backends;
                backends += 1;
                SourceF::Backend(observable)
            }
            SensorSource::BuiltinDt => SourceF::Dt,
            SensorSource::BuiltinTick => SourceF::Tick,
        })
        .collect()
}

fn int_plan(sensors: &[Sensor]) -> Vec<SourceI> {
    let mut backends = 0usize;
    sensors
        .iter()
        .map(|sensor| match sensor.source {
            SensorSource::BuiltinTick => SourceI::Tick,
            _ => {
                let observable = backends;
                backends += 1;
                SourceI::Backend(observable)
            }
        })
        .collect()
}
