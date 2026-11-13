//! The action, the host and the viewer's record of the walk.
//!
//! This is the app half of the demo: it assembles the actions, loads them into
//! whatever body it was handed, pumps the arbiter, and turns each tick into a
//! [`Scene`] for the renderer.

use cyberspine_arbiter::Verdict as ArbiterVerdict;
use cyberspine_asm::assembler::{self, AsmParseErrors, ImageGenError};
use cyberspine_backend::BackendSpec;
use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Rig;
use cyberspine_demo_maze_world::{Grid, MANIFEST};
use cyberspine_host::{Interrupts, LogError, Registry, Report};
use cyberspine_vm::{
    BackendError, FaultRecord, Host, LoadError, Session as EngineSession, TraceIter,
};

use crate::actions::ActionSet;
use crate::arbiter::Junction;
use crate::body::{Body, Observation};

pub const SOURCE: &str = include_str!("../maze.csa");
pub const REVERSE_SOURCE: &str = include_str!("../reverse.csa");
pub const MAX_TICKS: u64 = 600;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the rig manifest is invalid: {0}")]
    Manifest(#[from] cyberspine_manifest::ManifestError),
    #[error("the program does not parse:\n{0}")]
    Parse(#[from] AsmParseErrors),
    #[error("the program does not assemble: {0}")]
    Assemble(#[from] ImageGenError),
    #[error("the image is not valid: {0}")]
    Invalid(#[from] cyberspine_core::binfmt::load::LoadError),
    #[error("the image does not load: {0}")]
    Load(#[from] LoadError),
    #[error("the backend spec is invalid: {0}")]
    Spec(#[from] cyberspine_backend::SpecError),
    #[error("the backend is not available: {0}")]
    Backend(#[from] BackendError),
    #[error("the decision log could not be written: {0}")]
    Log(#[from] LogError),
}

/// Compile a `.csa` source against a rig, ready for `Host::load`.
pub fn assemble(source: &str, rig: &Rig) -> Result<Image, Error> {
    let asm = assembler::parse(source)?;
    Ok(assembler::generate_image(&asm, rig)?)
}

/// One instant of the world plus the walk and the rulings recorded so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub tick: u64,
    pub observation: Observation,
    pub trail: Vec<(i32, i32)>,
    pub reports: Vec<Report>,
}

struct Arbitration {
    registry: Registry,
    interrupts: Interrupts<Junction>,
}

/// One assembled action, its body, and the host ticking them together.
pub struct Session {
    host: EngineSession<Body>,
    trail: Vec<(i32, i32)>,
    trail_tick: Option<u64>,
    arbitration: Option<Arbitration>,
    reports: Vec<Report>,
}

impl Session {
    /// The default demo: the unsigned-junction handover.
    pub fn embedded() -> Result<Self, Error> {
        Self::handover()
    }

    /// The handover over a body the spec names.
    pub fn external(spec: &BackendSpec) -> Result<Self, Error> {
        Self::handover_external(spec)
    }

    /// Load `maze.csa` and the three action images the arbiter may switch to.
    pub fn handover() -> Result<Self, Error> {
        Self::handover_with(Body::linked())
    }

    pub fn handover_external(spec: &BackendSpec) -> Result<Self, Error> {
        Self::handover_with(Body::external(spec)?)
    }

    /// Assemble `source` against `manifest`, then load the image into `body`.
    pub fn open(source: &str, manifest: &str, body: Body) -> Result<Self, Error> {
        let rig = cyberspine_manifest::from_json(manifest)?;
        let image = assemble(source, &rig)?;

        let (mut host, _) = Host::new(body)
            .load(VerifiedImage::verify(image)?)
            .map_err(|(_, error)| error)?;
        host.set_tracing(true);
        Ok(Self {
            host,
            trail: Vec::new(),
            trail_tick: None,
            arbitration: None,
            reports: Vec::new(),
        })
    }

    fn handover_with(body: Body) -> Result<Self, Error> {
        let rig = cyberspine_manifest::from_json(MANIFEST)?;
        let maze = VerifiedImage::verify(assemble(SOURCE, &rig)?)?;
        let left = VerifiedImage::verify(assemble(&SOURCE.replace("turn = 0", "turn = 1"), &rig)?)?;
        let right =
            VerifiedImage::verify(assemble(&SOURCE.replace("turn = 0", "turn = 2"), &rig)?)?;
        let reverse = VerifiedImage::verify(assemble(REVERSE_SOURCE, &rig)?)?;

        let (mut host, id) = Host::new(body)
            .load(maze.clone())
            .map_err(|(_, error)| error)?;
        host.set_tracing(true);

        let mut registry = Registry::new(&maze);
        registry.push(
            id,
            maze.header().layer,
            maze.content_hash(),
            "maze.csa".into(),
        );

        let interrupts = Interrupts::new(
            Junction::new(),
            Some(Box::new(ActionSet::new(reverse, left, right))),
            None,
            ArbiterVerdict::Stop,
        );

        Ok(Self {
            host,
            trail: Vec::new(),
            trail_tick: None,
            arbitration: Some(Arbitration {
                registry,
                interrupts,
            }),
            reports: Vec::new(),
        })
    }

    /// Advance the clock, then let the arbiter answer whatever the tick raised.
    pub fn advance(&mut self, seconds: f64) -> Result<(), Error> {
        self.host.advance(seconds)?;
        if let Some(arbitration) = self.arbitration.as_mut() {
            let reports = arbitration
                .interrupts
                .pump(&mut self.host, &mut arbitration.registry)?;
            self.reports.extend(reports);
        }
        Ok(())
    }

    /// Draw at most one tick per frame. A viewer that ran several would only see
    /// the last position and record a trail with gaps in it.
    pub fn set_max_ticks_per_frame(&mut self, ticks: u32) {
        self.host.set_max_ticks_per_frame(ticks);
    }

    pub fn ticks(&self) -> u64 {
        self.host.now()
    }

    /// One virtual tick in seconds, i.e. `1 / hz` from the image header.
    pub fn dt(&self) -> f32 {
        self.host.dt()
    }

    pub fn running(&self) -> bool {
        self.host.instances() > 0
    }

    /// The map the viewer draws: the linked body's own, or the demo's for a
    /// wired body, whose terrain the rig does not carry.
    pub fn grid(&self) -> &Grid {
        self.host.backend().grid()
    }

    /// Everything the arbiter did so far, oldest first.
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }

    /// The instant to draw, plus the walk and the rulings recorded so far.
    pub fn scene(&mut self) -> Result<Scene, BackendError> {
        let observation = self.host.backend_mut().observe()?;
        let tick = self.host.now();

        if self.trail_tick != Some(tick) {
            let here = (observation.x, observation.y);
            if self.trail.last() != Some(&here) {
                self.trail.push(here);
            }
            self.trail_tick = Some(tick);
        }

        Ok(Scene {
            tick,
            observation,
            trail: self.trail.clone(),
            reports: self.reports.clone(),
        })
    }

    /// What one run did, in the numbers the demo prints.
    pub fn outcome(&mut self) -> Result<Outcome, BackendError> {
        let observation = self.host.backend_mut().observe()?;
        Ok(Outcome {
            ticks: self.ticks(),
            moves: observation.odometer.moves,
            turns: observation.odometer.turns,
            bumps: observation.odometer.bumps,
            position: (observation.x, observation.y),
            at_goal: observation.at_goal,
            running: self.running(),
            faults: self.host.faults().to_vec(),
        })
    }

    /// The per-tick command vectors, oldest first.
    pub fn trace(&self) -> TraceIter<'_> {
        self.host.trace()
    }
}

/// What one run did, in the numbers the demo prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub ticks: u64,
    pub moves: u32,
    pub turns: u32,
    pub bumps: u32,
    pub position: (i32, i32),
    pub at_goal: bool,
    pub running: bool,
    pub faults: Vec<FaultRecord>,
}

impl Outcome {
    pub fn solved(&self) -> bool {
        self.at_goal && !self.running && self.faults.is_empty()
    }
}
