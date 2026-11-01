//! The execution core: instance state, the `resume` loop, fuel and `WAIT`
//! (`docs/exec.md`).
//!
//! Only the host needs this layer. The assembler and the loader depend on
//! [`crate::isa`] and [`crate::binfmt`], never on anything here.

pub mod instance;
pub mod program;
pub mod registers;

pub use instance::{Fault, Instance, Sensors, Status, Step};
pub use program::Program;
