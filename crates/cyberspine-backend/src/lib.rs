//! Bodies that are not a Rust value in this process.
//!
//! Three integration modes share one [`Backend`](cyberspine_vm::Backend) trait:
//!
//! * **linked** — depend on the body's crate and pass it to `Host::new`. Nothing
//!   in this crate is involved.
//! * **dynamic library** — `dylib` feature: load a `cdylib` at run time.
//! * **remote** — `rpc` feature: connect to a body that serves itself over TCP.
//!
//! The last two cross a real boundary, so they speak one protocol: [`wire`] is a
//! 12-byte frame header around a MessagePack payload, and [`abi`] is the C
//! function table a dynamic library exports. A body implements
//! [`Backend`](cyberspine_vm::Backend) once and is packaged by `export!` or by
//! [`server::Server`], so one protocol means one conformance suite.
//!
//! The host side stays synchronous. `rpc` keeps its runtime inside the transport,
//! so [`Backend`](cyberspine_vm::Backend) and `Session::tick` never name a future.

pub mod abi;
pub mod remote;
pub mod server;
pub mod spec;
pub mod wire;

#[cfg(feature = "dylib")]
pub mod dylib;

#[cfg(feature = "export")]
pub mod export;

#[cfg(feature = "rpc")]
pub mod rpc;

pub use remote::{RemoteBody, Transport};
pub use server::Server;
pub use spec::{BackendSpec, SpecError};
