//! cyber-spine: a bytecode VM that drives real or virtual robots through one
//! uniform interface.
//!
//! This crate is the **portable half**: it is `#![no_std]` unconditionally and
//! needs only `alloc`. Everything that requires an operating system — files,
//! dynamic libraries, sockets — lives in the `cyberspine-vm` platform layer.
//!
//! The normative documents live in `docs/`:
//!
//! * [`docs/isa.md`](../../../docs/isa.md) — instruction encoding and semantics,
//!   registers, the clock, the instance model.
//! * [`docs/binfmt.md`](../../../docs/binfmt.md) — the `.csp` executable-image
//!   byte layout and its load-time validation checklist.
//! * [`docs/exec.md`](../../../docs/exec.md) — the constraints the execution core
//!   must satisfy: state layout, the `resume` loop, fuel, `WAIT`.
//!
//! [`docs/asm.md`](../../../docs/asm.md) specifies the `.csa` assembly source.
//!
//! # Why there is no `std` feature
//!
//! A conditional `std` would make "this builds on a microcontroller" a per-build
//! configuration that CI can silently stop checking. Instead the crate cannot
//! name `std` at all — the platform layer depends on it, so the no_std claim is a
//! compile-time fact rather than a configuration choice.

#![no_std]

extern crate alloc;

pub mod binfmt;
pub mod exec;
pub mod isa;
pub mod rig;
