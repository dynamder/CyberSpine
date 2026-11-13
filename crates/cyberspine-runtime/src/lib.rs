//! The `cyinevm` runtime: open a body, load an image and tick them together.
//!
//! A linked body is a type-level choice (`linked::LinkedBody`); an external body
//! is a run-time choice (`BackendSpec`). The two paths meet at `Session`, which
//! is why the ticking code is generic over `Backend` and `RigPolicy` rather than
//! taking a boxed body.

pub mod actions;
pub mod cli;
pub mod linked;
pub mod run;
mod trace;

pub use actions::{ActionDirectory, ActionError, ActionResolver};
pub use cli::Args;
pub use cyberspine_core::binfmt::load::VerifiedImage;
pub use linked::{LinkedBody, Notice, Official};
pub use run::{Error, Feature, OnInterrupt, Options, RunMode, Stop, run_external, run_linked};
