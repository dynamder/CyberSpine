//! The **platform layer**: everything that needs an operating system.
//!
//! `cyberspine-core` is `#![no_std]` and cannot open a file, spawn anything, or
//! name `std` at all. This crate is the other side of that line: it reads `.csp`
//! images from disk and gates the read on the file's size; the backend adapters
//! that need an OS (dynamic libraries, sockets) belong here too.
//!
//! It is also where the *host* lives: the tick loop, the command merge and the
//! [`Backend`] trait. None of that is the VM — the VM executes one instance at a
//! time and knows nothing about `now`, load order or bodies (`docs/isa.md` §8).
//!
//! The assembler lives in `cyberspine-asm`, which also owns writing an image to
//! disk — this crate only reads and validates one.
//!
//! Keeping the split at "can this compile on a microcontroller" rather than at
//! "is this conceptually the VM" is what lets the same execution core run on a
//! device and on a desktop with no `#if` between them.

pub mod backend;
pub mod dummy;
pub mod host;
pub mod io;

pub use backend::{Backend, BackendError, Shape, Snapshot};
pub use dummy::{Call, DummyBackend};
pub use host::{
    DEFAULT_TRACE_CAPACITY, DecideError, FaultRecord, Host, InstanceId, Interrupt, LoadError,
    Permissive, RigPolicy, Session, Strict, TickRef, TraceIter, Verdict,
};
