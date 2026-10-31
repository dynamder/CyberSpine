//! The **platform layer**: everything that needs an operating system.
//!
//! `cyberspine-core` is `#![no_std]` and cannot open a file, spawn anything, or
//! name `std` at all. This crate is the other side of that line: today it reads
//! `.csp` images from disk; the backend adapters that need an OS (dynamic
//! libraries, sockets) belong here too.
//!
//! Keeping the split at "can this compile on a microcontroller" rather than at
//! "is this conceptually the VM" is what lets the same execution core run on a
//! device and on a desktop with no `#if` between them.

pub mod io;
