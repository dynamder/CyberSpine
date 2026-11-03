//! The body for `maze.csa`: a grid world, its heading and its odometer.
//!
//! This crate is the world and nothing else: no host, no transport, no terminal.
//! It is one `impl Backend`, so the same type can be linked into the app,
//! exported from the `dylib` crate, and served by the `rpc` crate without a line
//! changing.

pub mod backend;
pub mod grid;

pub use backend::{MazeBackend, Muscle, RIG_NAME, RIG_VERSION, SensorF, SensorI, rig};
pub use grid::{Cell, DEFAULT_LAYOUT, Grid, GridError, Heading};

/// The manifest the assembler reads. [`rig`] is the body's side of the same
/// declaration; the test in `tests/world.rs` compares the two.
pub const MANIFEST: &str = include_str!("../../maze.rig.json");
