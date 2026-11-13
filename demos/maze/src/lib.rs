//! The host half of the maze demo: assemble the action, drive a body, draw it.
//!
//! The demo is split by responsibility:
//!
//! * `cyberspine-demo-maze-world` — the body and its world.
//! * `cyberspine-demo-maze-dylib` — the same body as a `cdylib`.
//! * `cyberspine-demo-maze-rpc` — the same body behind a TCP server.
//! * this crate — the host. It opens one of the three and draws the walk.
//!
//! Only [`Body`] knows which one is behind it; the host and the renderer read
//! the rig and never the struct.

pub mod actions;
pub mod arbiter;
pub mod body;
pub mod render;
pub mod session;

pub use body::{Body, Observation, Odometer};
pub use cyberspine_demo_maze_world::MANIFEST;
pub use session::{Error, MAX_TICKS, Outcome, REVERSE_SOURCE, SOURCE, Scene, Session, assemble};
