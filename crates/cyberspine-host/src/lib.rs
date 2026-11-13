//! Host-side arbitration: the in-flight table, atomic actions, and the ruling
//! stream (`docs/interrupt.md` §7).
//!
//! A host owns the tickets and the instances; this crate turns one
//! [`InterruptEvent`](cyberspine_arbiter::InterruptEvent) into applied
//! [`Ruling`](cyberspine_arbiter::Ruling)s and reports what happened. It knows
//! nothing about a CLI, a terminal, or real time, so `cyinevm` and the maze
//! example drive the same loop.

pub mod actions;
pub mod decisions;
pub mod interrupts;
pub mod registry;
pub mod report;

pub use actions::{ActionDirectory, ActionError, ActionResolver};
pub use decisions::{Log, LogError, Record, Replay};
pub use interrupts::Interrupts;
pub use registry::{Loaded, Registry};
pub use report::{Report, RoundTrace};
