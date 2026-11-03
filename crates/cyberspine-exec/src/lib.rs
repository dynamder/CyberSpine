//! The execution core: instance state, the `resume` loop, fuel and `WAIT`
//! (`docs/exec.md`).
//!
//! `#![no_std]` with only `alloc`: this is the half that runs on the device. It
//! depends on `isa` and `binfmt` and never on a platform; the assembler depends
//! on neither of those plus this, and the host is the only layer above it.

#![no_std]

extern crate alloc;

pub mod instance;
pub mod program;
pub mod registers;

pub use instance::{Fault, Instance, InterruptError, PendingInterrupt, Sensors, Status, Step};
pub use program::Program;
