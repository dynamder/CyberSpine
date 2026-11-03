//! A guided walk through a 2D maze: the action is a `.csa` program, the body is
//! a plain Rust [`Backend`](cyberspine_vm::Backend). Nothing here is VM-specific
//! — the shim is the trait in `cyberspine_vm`.
//!
//! The same body ships three ways. [`Body::linked`] is the body compiled into
//! this binary; [`Body::external`] is one reached through a transport, opened
//! from the same `dylib:` / `rpc:` spelling the CLI uses. A session does not know
//! which it has, and neither does the viewer: it reads the rig, not the struct.

pub mod backend;
pub mod body;
pub mod grid;
pub mod render;

use cyberspine_asm::assembler::{self, AsmParseErrors, ImageGenError};
use cyberspine_backend::BackendSpec;
use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Rig;
use cyberspine_vm::{
    BackendError, FaultRecord, Host, LoadError, Session as EngineSession, TraceIter,
};

pub use backend::{MazeBackend, Muscle, RIG_NAME, RIG_VERSION, SensorF, SensorI, rig};
pub use body::{Body, Observation, Odometer};
pub use grid::{Cell, DEFAULT_LAYOUT, Grid, GridError, Heading};

pub const SOURCE: &str = include_str!("../maze.csa");
pub const MANIFEST: &str = include_str!("../maze.rig.json");
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
}

/// Compile a `.csa` source against a rig, ready for `Host::load`.
pub fn assemble(source: &str, rig: &Rig) -> Result<Image, Error> {
    let asm = assembler::parse(source)?;
    Ok(assembler::generate_image(&asm, rig)?)
}

/// One instant of the world plus the walk the viewer has recorded so far.
///
/// The trail is the viewer's memory, not the body's: a body only reports where
/// it is now, and a viewer draws the cells it has seen it occupy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub tick: u64,
    pub observation: Observation,
    pub trail: Vec<(i32, i32)>,
}

/// One assembled action, its body, and the host ticking them together.
///
/// Pacing is the host's (`Host::advance`): a frame hands over the real time it
/// spent and the host decides how many ticks that buys. Nothing outside this
/// type can step the machine one tick at a time, so an example cannot invent a
/// second advance policy.
pub struct Session {
    host: EngineSession<Body>,
    trail: Vec<(i32, i32)>,
    trail_tick: Option<u64>,
}

impl Session {
    /// The body compiled into this binary.
    pub fn embedded() -> Result<Self, Error> {
        Self::open(SOURCE, MANIFEST, Body::linked())
    }

    /// A body the spec names, reached in its own process or from a shared
    /// library. The handshake runs before the image is assembled, so a body that
    /// cannot introduce itself fails at startup instead of mid-tick.
    pub fn external(spec: &BackendSpec) -> Result<Self, Error> {
        Self::open(SOURCE, MANIFEST, Body::external(spec)?)
    }

    /// Assemble `source` against `manifest`, then load the image into `body`.
    /// The fingerprints must agree or loading refuses the image.
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
        })
    }

    /// Hand the host the real seconds a frame took; it runs the ticks they
    /// contain and keeps the remainder. A linked body cannot fail; a wired one
    /// can, and that error is terminal.
    pub fn advance(&mut self, seconds: f64) -> Result<(), BackendError> {
        self.host.advance(seconds)
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

    /// The instant to draw, plus the walk recorded so far. For a wired body this
    /// costs one extra exchange; calling it twice for one tick records the trail
    /// only once.
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
        })
    }

    /// What one run did, in the numbers the demo prints. Reading it asks the
    /// body for one final instant.
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

// The body as a shared library. The linked path does not need this: the same
// `MazeBackend` is the thing that ships.
cyberspine_backend::export! {
    body: backend::MazeBackend,
    rig: MANIFEST,
    create: |_config| backend::MazeBackend::new(),
}
