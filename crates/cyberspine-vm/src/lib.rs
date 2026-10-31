//! cyber-spine: a bytecode VM that drives real or virtual robots through one
//! uniform interface.
//!
//! The two normative documents live in `docs/`:
//!
//! * [`docs/isa.md`](../../../docs/isa.md) — instruction encoding and semantics,
//!   registers, the clock, the instance model.
//! * [`docs/binfmt.md`](../../../docs/binfmt.md) — the `.csp` executable-image
//!   byte layout and its load-time validation checklist.
//!
//! [`docs/asm.md`](../../../docs/asm.md) specifies the `.csa` assembly source;
//! the assembler lives in the `cyberspine-asm` crate, which also owns writing an
//! image to disk (this crate only reads and validates one).

pub mod binfmt;
pub mod register;
pub mod vm;
