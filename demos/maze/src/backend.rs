//! The body for `maze.csa`: a grid world with a heading.
//!
//! The host asks this type for three things per tick: a snapshot of what the
//! runner senses, the commands the program published, and one step of the
//! world. The grid, the walls and the position never leave this side; the
//! program only sees the names in `maze.rig.json`.

use cyberspine_core::rig::{BoundFingerprint, Name, Rig};
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

use crate::grid::{Cell, DEFAULT_LAYOUT, Grid, Heading};

pub const RIG_NAME: &str = "maze-runner";
pub const RIG_VERSION: u32 = 1;

/// The body's actuators, in physical-index order. A command at or above `0.5`
/// means "do it this tick"; the value is held until the program lowers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Muscle {
    Forward,
    Backward,
    TurnLeft,
    TurnRight,
}

impl Muscle {
    pub const ALL: [Self; 4] = [
        Self::Forward,
        Self::Backward,
        Self::TurnLeft,
        Self::TurnRight,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::TurnLeft => "turn_left",
            Self::TurnRight => "turn_right",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The body's `f32` observables, in physical-index order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorF {
    MarkerRed,
    MarkerBlue,
    AtGoal,
    WallAhead,
}

impl SensorF {
    pub const ALL: [Self; 4] = [
        Self::MarkerRed,
        Self::MarkerBlue,
        Self::AtGoal,
        Self::WallAhead,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::MarkerRed => "marker_red",
            Self::MarkerBlue => "marker_blue",
            Self::AtGoal => "at_goal",
            Self::WallAhead => "wall_ahead",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The body's `i32` observables, in physical-index order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorI {
    X,
    Y,
    Heading,
}

impl SensorI {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Heading];

    pub const fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Heading => "heading",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The backend's declaration of the body. `maze.rig.json` is the assembler's
/// copy; the two are hashed independently and the host refuses a mismatch.
pub fn rig() -> Rig {
    let mut rig = Rig::new(RIG_NAME, RIG_VERSION);
    for muscle in Muscle::ALL {
        rig = rig.muscle(name(muscle.name()));
    }
    for sensor in SensorF::ALL {
        rig = rig.sensor_f(name(sensor.name()));
    }
    for sensor in SensorI::ALL {
        rig = rig.sensor_i(name(sensor.name()));
    }
    rig
}

/// The body's own names are literals; a typo here is caught by the test that
/// compares this rig against the shipped manifest.
fn name(text: &'static str) -> Name {
    Name::new(text).expect("the body's names are literals")
}

/// The body's state: where the runner is, which way it faces, and the value
/// each muscle was last commanded to hold.
pub struct MazeBackend {
    grid: Grid,
    fingerprint: BoundFingerprint,
    x: i32,
    y: i32,
    heading: Heading,
    commands: [f32; Muscle::ALL.len()],
    moves: u32,
    turns: u32,
    bumps: u32,
    trail: Vec<(i32, i32)>,
}

impl MazeBackend {
    /// The runner starts on `S` facing east; the heading is part of the body.
    pub fn new() -> Self {
        Self::with_grid(Grid::parse(&DEFAULT_LAYOUT).expect("the embedded maze layout is valid"))
    }

    pub fn with_grid(grid: Grid) -> Self {
        let (x, y) = grid.start();
        Self {
            fingerprint: BoundFingerprint::of(&rig()),
            grid,
            x,
            y,
            heading: Heading::East,
            commands: [0.0; Muscle::ALL.len()],
            moves: 0,
            turns: 0,
            bumps: 0,
            trail: vec![(x, y)],
        }
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn heading(&self) -> Heading {
        self.heading
    }

    pub fn moves(&self) -> u32 {
        self.moves
    }

    pub fn turns(&self) -> u32 {
        self.turns
    }

    pub fn bumps(&self) -> u32 {
        self.bumps
    }

    pub fn trail(&self) -> &[(i32, i32)] {
        &self.trail
    }

    pub fn marker_red(&self) -> bool {
        self.grid.cell(self.x, self.y) == Some(Cell::Red)
    }

    pub fn marker_blue(&self) -> bool {
        self.grid.cell(self.x, self.y) == Some(Cell::Blue)
    }

    pub fn at_goal(&self) -> bool {
        (self.x, self.y) == self.grid.goal()
    }

    pub fn wall_ahead(&self) -> bool {
        let (dx, dy) = self.heading.delta();
        !self.grid.is_walkable(self.x + dx, self.y + dy)
    }

    fn excited(&self, muscle: Muscle) -> bool {
        self.commands[muscle.index()] >= 0.5
    }
}

impl Default for MazeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for MazeBackend {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    /// The three counts the host sizes a snapshot with. They are the same rig the
    /// fingerprint hashes, counted once.
    fn shape(&self) -> Shape {
        Shape {
            muscles: Muscle::ALL.len(),
            sensors_f: SensorF::ALL.len(),
            sensors_i: SensorI::ALL.len(),
        }
    }

    /// Report one instant: the cell underfoot, the wall in front, the goal and
    /// the position. The host freezes this snapshot for the whole tick.
    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        out.f[SensorF::MarkerRed.index()] = flag(self.marker_red());
        out.f[SensorF::MarkerBlue.index()] = flag(self.marker_blue());
        out.f[SensorF::AtGoal.index()] = flag(self.at_goal());
        out.f[SensorF::WallAhead.index()] = flag(self.wall_ahead());

        out.i[SensorI::X.index()] = self.x;
        out.i[SensorI::Y.index()] = self.y;
        out.i[SensorI::Heading.index()] = self.heading.index();

        out.muscles.copy_from_slice(&self.commands);
        Ok(())
    }

    /// Latch this tick's published commands. A muscle keeps its last value
    /// until a program changes it; `None` means no instance published it.
    fn commit(&mut self, commands: &[Option<f32>], _dt: f32) -> Result<(), BackendError> {
        for (held, command) in self.commands.iter_mut().zip(commands) {
            if let Some(command) = command {
                *held = *command;
            }
        }
        Ok(())
    }

    /// Apply one tick: turn first, then move. The order only matters if a
    /// program presses both, which this one never does. A move into a wall is
    /// a bump, not a fault; the world simply refuses it.
    fn step(&mut self, _dt: f32) -> Result<(), BackendError> {
        let turn = match (
            self.excited(Muscle::TurnLeft),
            self.excited(Muscle::TurnRight),
        ) {
            (true, false) => Some(self.heading.left()),
            (false, true) => Some(self.heading.right()),
            _ => None,
        };
        if let Some(heading) = turn {
            self.heading = heading;
            self.turns += 1;
        }

        let displacement = match (
            self.excited(Muscle::Forward),
            self.excited(Muscle::Backward),
        ) {
            (true, false) => 1,
            (false, true) => -1,
            _ => return Ok(()),
        };
        let (dx, dy) = self.heading.delta();
        let (x, y) = (self.x + dx * displacement, self.y + dy * displacement);
        if self.grid.is_walkable(x, y) {
            (self.x, self.y) = (x, y);
            self.moves += 1;
            self.trail.push((x, y));
        } else {
            self.bumps += 1;
        }
        Ok(())
    }
}

fn flag(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}
