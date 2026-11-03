//! A guided walk through a 2D maze: the action is a `.csa` program, the body is
//! a plain Rust `Backend`. Nothing here is VM-specific — the shim is the four
//! methods in [`cyberspine_vm::Backend`].

pub mod backend;
pub mod grid;
pub mod render;

use cyberspine_asm::assembler::{self, AsmParseErrors, ImageGenError};
use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Rig;
use cyberspine_vm::{BackendError, FaultRecord, Host, LoadError, Session as EngineSession};

pub use backend::{MazeBackend, Muscle, RIG_NAME, RIG_VERSION, SensorF, SensorI, rig};
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
}

/// Compile a `.csa` source against a rig, ready for `Host::load`.
pub fn assemble(source: &str, rig: &Rig) -> Result<Image, Error> {
    let asm = assembler::parse(source)?;
    Ok(assembler::generate_image(&asm, rig)?)
}

/// One assembled action, its body, and the host ticking them together.
///
/// Pacing is the host's (`Host::advance`): a frame hands over the real time it
/// spent and the host decides how many ticks that buys. Nothing outside this
/// type can step the machine one tick at a time, so an example cannot invent a
/// second advance policy.
pub struct Session {
    host: EngineSession<MazeBackend>,
}

impl Session {
    pub fn embedded() -> Result<Self, Error> {
        Self::open(SOURCE, MANIFEST)
    }

    /// Assemble `source` against `manifest`, then load the image into a fresh
    /// body. The fingerprints must agree or loading refuses the image.
    pub fn open(source: &str, manifest: &str) -> Result<Self, Error> {
        let rig = cyberspine_manifest::from_json(manifest)?;
        let image = assemble(source, &rig)?;

        let (mut host, _) = Host::new(MazeBackend::new())
            .load(VerifiedImage::verify(image)?)
            .map_err(|(_, error)| error)?;
        host.set_tracing(true);
        Ok(Self { host })
    }

    /// Hand the host the real seconds a frame took; it runs the ticks they
    /// contain and keeps the remainder. The maze body cannot fail, so the only
    /// error this can return would come from a backend that replaced it.
    pub fn advance(&mut self, seconds: f64) -> Result<(), BackendError> {
        self.host.advance(seconds)
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

    pub fn backend(&self) -> &MazeBackend {
        self.host.backend()
    }

    pub fn host(&self) -> &EngineSession<MazeBackend> {
        &self.host
    }

    pub fn outcome(&self) -> Outcome {
        let backend = self.host.backend();
        Outcome {
            ticks: self.ticks(),
            moves: backend.moves(),
            turns: backend.turns(),
            bumps: backend.bumps(),
            at_goal: backend.at_goal(),
            running: self.running(),
            faults: self.host.faults().to_vec(),
        }
    }
}

/// What one run did, in the numbers the demo prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub ticks: u64,
    pub moves: u32,
    pub turns: u32,
    pub bumps: u32,
    pub at_goal: bool,
    pub running: bool,
    pub faults: Vec<FaultRecord>,
}

impl Outcome {
    pub fn solved(&self) -> bool {
        self.at_goal && !self.running && self.faults.is_empty()
    }
}
