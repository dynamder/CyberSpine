//! The boundary every CyberSpine plugin shares.
//!
//! A body and an arbiter answer different messages, but they cross a process or
//! library boundary the same way. One C table loads either of them, one retry
//! loop keeps a buffer lookup from becoming a second call, one dedup cursor
//! makes a retried frame a reply instead of a second delivery, and one TCP loop
//! serves either side.
//!
//! What stays in the two crates is what actually differs: the message types, the
//! [`Server`](crate::server::Serve) implementation that dispatches them, and the
//! host-side `Backend` / `Arbiter` adapters. Everything below that line lives
//! here once, so a protocol fix cannot land on one side only.
//!
//! The frame head itself is [`cyberspine_frame`]'s. This crate adds the layer a
//! plugin protocol shares *above* the head: the ABI table, the loaders and the
//! transport loops.

#![warn(missing_docs)]

pub mod abi;
pub mod server;
pub mod spec;

#[cfg(feature = "dylib")]
pub mod dylib;

#[cfg(feature = "rpc")]
pub mod rpc;
