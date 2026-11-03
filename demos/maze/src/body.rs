//! The two things a session can drive: a body compiled into this binary, and a
//! body reached through a transport.
//!
//! Both are `Backend`, so the host never learns which one it has and every tick
//! is the same three calls. The demo's viewer is the one that needs more: it
//! asks for an instant to draw. A linked body answers from its own fields; a
//! wired body answers with one extra `Sample`, which the protocol defines as a
//! read and therefore allows outside the tick.

use cyberspine_backend::BackendSpec;
use cyberspine_core::rig::BoundFingerprint;
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

use crate::backend::{MazeBackend, SensorF, SensorI};
use crate::grid::{DEFAULT_LAYOUT, Grid, Heading};

/// How far the runner has travelled, as the body counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Odometer {
    pub moves: u32,
    pub turns: u32,
    pub bumps: u32,
}

/// One instant of the world, as the rig publishes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub x: i32,
    pub y: i32,
    pub heading: Heading,
    pub marker_red: bool,
    pub marker_blue: bool,
    pub at_goal: bool,
    pub wall_ahead: bool,
    pub odometer: Odometer,
}

/// A body compiled into this binary, or one reached over a wire.
pub enum Body {
    Linked(MazeBackend),
    Wired(Wired),
}

/// A body behind a transport, plus the buffers an observation outside the tick
/// loop needs.
pub struct Wired {
    inner: Box<dyn Backend>,
    grid: Grid,
    f: Vec<f32>,
    i: Vec<i32>,
    muscles: Vec<f32>,
}

impl Body {
    pub fn linked() -> Self {
        Self::Linked(MazeBackend::new())
    }

    /// Open the body `spec` names. The handshake happens here, so a body that
    /// cannot introduce itself fails before the image is assembled.
    pub fn external(spec: &BackendSpec) -> Result<Self, BackendError> {
        let inner = spec.open()?;
        let shape = inner.shape();
        Ok(Self::Wired(Wired {
            inner,
            grid: Grid::parse(&DEFAULT_LAYOUT).expect("the embedded maze layout is valid"),
            f: vec![0.0; shape.sensors_f],
            i: vec![0; shape.sensors_i],
            muscles: vec![0.0; shape.muscles],
        }))
    }

    /// The instant the viewer draws. For a linked body this is a field read; for
    /// a wired one it is a round trip, so a frame costs one more exchange than a
    /// tick does — never the other way round.
    pub fn observe(&mut self) -> Result<Observation, BackendError> {
        match self {
            Self::Linked(body) => Ok(Observation {
                x: body.position().0,
                y: body.position().1,
                heading: body.heading(),
                marker_red: body.marker_red(),
                marker_blue: body.marker_blue(),
                at_goal: body.at_goal(),
                wall_ahead: body.wall_ahead(),
                odometer: Odometer {
                    moves: body.moves(),
                    turns: body.turns(),
                    bumps: body.bumps(),
                },
            }),
            Self::Wired(wired) => wired.observe(),
        }
    }

    /// The map the viewer draws. A linked body owns it; a wired body is drawn on
    /// the demo's own map, because terrain is not part of the rig.
    pub fn grid(&self) -> &Grid {
        match self {
            Self::Linked(body) => body.grid(),
            Self::Wired(wired) => &wired.grid,
        }
    }
}

impl Wired {
    fn observe(&mut self) -> Result<Observation, BackendError> {
        let mut snapshot = Snapshot {
            f: &mut self.f,
            i: &mut self.i,
            muscles: &mut self.muscles,
        };
        self.inner.sample(&mut snapshot)?;

        let heading = Heading::from_index(self.i[SensorI::Heading.index()]).ok_or_else(|| {
            BackendError::message(format!(
                "the body reported heading {}, expected 0..=3",
                self.i[SensorI::Heading.index()]
            ))
        })?;

        Ok(Observation {
            x: self.i[SensorI::X.index()],
            y: self.i[SensorI::Y.index()],
            heading,
            marker_red: raised(self.f[SensorF::MarkerRed.index()]),
            marker_blue: raised(self.f[SensorF::MarkerBlue.index()]),
            at_goal: raised(self.f[SensorF::AtGoal.index()]),
            wall_ahead: raised(self.f[SensorF::WallAhead.index()]),
            odometer: Odometer {
                moves: counted(self.i[SensorI::Moves.index()]),
                turns: counted(self.i[SensorI::Turns.index()]),
                bumps: counted(self.i[SensorI::Bumps.index()]),
            },
        })
    }
}

impl Backend for Body {
    fn fingerprint(&self) -> BoundFingerprint {
        match self {
            Self::Linked(body) => body.fingerprint(),
            Self::Wired(wired) => wired.inner.fingerprint(),
        }
    }

    fn shape(&self) -> Shape {
        match self {
            Self::Linked(body) => body.shape(),
            Self::Wired(wired) => wired.inner.shape(),
        }
    }

    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        match self {
            Self::Linked(body) => body.sample(out),
            Self::Wired(wired) => wired.inner.sample(out),
        }
    }

    fn commit(&mut self, commands: &[Option<f32>], dt: f32) -> Result<(), BackendError> {
        match self {
            Self::Linked(body) => body.commit(commands, dt),
            Self::Wired(wired) => wired.inner.commit(commands, dt),
        }
    }

    fn step(&mut self, dt: f32) -> Result<(), BackendError> {
        match self {
            Self::Linked(body) => body.step(dt),
            Self::Wired(wired) => wired.inner.step(dt),
        }
    }
}

fn raised(value: f32) -> bool {
    value > 0.5
}

fn counted(value: i32) -> u32 {
    u32::try_from(value).unwrap_or_default()
}
