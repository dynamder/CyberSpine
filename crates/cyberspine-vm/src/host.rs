//! The tick loop (`docs/isa.md` §8.4): sample, resume, merge, commit, step.
//!
//! This is the only place that owns `now`, load order and the command merge.
//! The VM executes one instance at a time and never sees any of it.

use cyberspine_core::binfmt::image::{Image, Sensor, SensorSource};
use cyberspine_core::exec::{Instance, Program, Sensors, Status};

use crate::backend::{Backend, RigLayout, SensorValues};

/// A stable handle on a loaded instance. Ids are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceId(u64);

/// One tick's merged command vector, kept when tracing is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub now: u64,
    pub commands: Vec<Option<f32>>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("image declares {declared} float sensors, the rig provides {rig}")]
    SensorFCount { declared: usize, rig: usize },
    #[error("image declares {declared} integer sensors, the rig provides {rig}")]
    SensorICount { declared: usize, rig: usize },
    #[error("image declares {declared} muscles, the rig provides {rig}")]
    MuscleCount { declared: usize, rig: usize },
    #[error("image declares {declared} Hz, the host is already ticking at {host} Hz")]
    Hz { declared: u32, host: u32 },
}

#[derive(Debug, Clone, Copy)]
enum SourceF {
    Backend,
    Muscle(usize),
    Dt,
    Tick,
}

#[derive(Debug, Clone, Copy)]
enum SourceI {
    Backend,
    Tick,
}

struct Slot {
    id: InstanceId,
    layer: u16,
    image: Image,
    instance: Instance,
    sensors_f: Vec<f32>,
    sensors_i: Vec<i32>,
    plan_f: Vec<SourceF>,
    plan_i: Vec<SourceI>,
}

impl Slot {
    fn fill<B: Backend>(
        &mut self,
        backend: &B,
        rig_f: &[f32],
        rig_i: &[i32],
        now: u64,
        dt: f32,
    ) {
        for (index, source) in self.plan_f.iter().enumerate() {
            self.sensors_f[index] = match *source {
                SourceF::Backend => rig_f[index],
                SourceF::Muscle(muscle) => backend.muscle(muscle),
                SourceF::Dt => dt,
                SourceF::Tick => now as i32 as f32,
            };
        }
        for (index, source) in self.plan_i.iter().enumerate() {
            self.sensors_i[index] = match *source {
                SourceI::Backend => rig_i[index],
                SourceI::Tick => now as i32,
            };
        }
    }
}

/// Several instances over one rig, advanced together.
pub struct Host<B: Backend> {
    backend: B,
    layout: RigLayout,
    slots: Vec<Slot>,
    merge_order: Vec<usize>,
    rig_f: Vec<f32>,
    rig_i: Vec<i32>,
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
        let layout = backend.layout();
        Self {
            rig_f: vec![0.0; layout.sensors_f],
            rig_i: vec![0; layout.sensors_i],
            backend,
            layout,
            slots: Vec::new(),
            merge_order: Vec::new(),
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

    /// Load one action. The image must be assembled for this rig: same sensor
    /// counts, same muscle count, same clock. Names become indices at assembly
    /// time; nothing here compares strings.
    pub fn load(&mut self, image: Image) -> Result<InstanceId, LoadError> {
        let layout = self.layout;
        if image.sensors_f.len() != layout.sensors_f {
            return Err(LoadError::SensorFCount {
                declared: image.sensors_f.len(),
                rig: layout.sensors_f,
            });
        }
        if image.sensors_i.len() != layout.sensors_i {
            return Err(LoadError::SensorICount {
                declared: image.sensors_i.len(),
                rig: layout.sensors_i,
            });
        }
        if image.muscles.len() != layout.muscles {
            return Err(LoadError::MuscleCount {
                declared: image.muscles.len(),
                rig: layout.muscles,
            });
        }

        let hz = image.header.hz;
        match self.hz {
            None => {
                self.hz = Some(hz);
                self.dt = 1.0 / hz as f32;
            }
            Some(host) if host != hz => {
                return Err(LoadError::Hz {
                    declared: hz,
                    host,
                });
            }
            Some(_) => {}
        }

        let plan_f = image
            .sensors_f
            .iter()
            .enumerate()
            .map(|(index, sensor)| float_source(index, sensor))
            .collect();
        let plan_i = image
            .sensors_i
            .iter()
            .map(|sensor| match sensor.source {
                SensorSource::BuiltinTick => SourceI::Tick,
                _ => SourceI::Backend,
            })
            .collect();

        let instance = {
            let program = Program::new(&image);
            Instance::new(&program)
        };
        let id = InstanceId(self.next_id);
        self.next_id += 1;

        self.slots.push(Slot {
            id,
            layer: image.header.layer,
            image,
            instance,
            sensors_f: vec![0.0; layout.sensors_f],
            sensors_i: vec![0; layout.sensors_i],
            plan_f,
            plan_i,
        });
        self.rebuild_merge_order();
        Ok(id)
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

    pub fn set_max_ticks_per_frame(&mut self, ticks: u32) {
        self.max_ticks_per_frame = ticks.max(1);
    }

    fn sample(&mut self) {
        let mut values = SensorValues {
            f: &mut self.rig_f,
            i: &mut self.rig_i,
        };
        self.backend.sample(&mut values);
    }

    fn resume_due(&mut self) {
        let now = self.now;
        let dt = self.dt;
        let backend = &self.backend;
        let rig_f = &self.rig_f;
        let rig_i = &self.rig_i;

        for slot in &mut self.slots {
            if slot.instance.wake_at() > now {
                continue;
            }
            slot.fill(backend, rig_f, rig_i, now, dt);

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
        let mut merged = vec![None; self.layout.muscles];
        for &index in &self.merge_order {
            let instance = &self.slots[index].instance;
            for (muscle, slot) in merged.iter_mut().enumerate() {
                if let Some(value) = instance.published(muscle) {
                    *slot = Some(value);
                }
            }
        }
        merged
    }

    fn remove_terminated(&mut self) {
        let before = self.slots.len();
        self.slots
            .retain(|slot| slot.instance.status() == Status::Suspended);
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

fn float_source(index: usize, sensor: &Sensor) -> SourceF {
    match sensor.source {
        SensorSource::Muscle => SourceF::Muscle(index),
        SensorSource::Backend => SourceF::Backend,
        SensorSource::BuiltinDt => SourceF::Dt,
        SensorSource::BuiltinTick => SourceF::Tick,
    }
}
