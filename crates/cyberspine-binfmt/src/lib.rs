//! The executable image: its layout (`docs/binfmt.md`) and the loader that
//! validates one.
//!
//! `#![no_std]` with only `alloc`: the same decoder runs where the action runs,
//! including on a microcontroller. It knows `isa` and `rig` and nothing about
//! execution or a platform.

#![no_std]

extern crate alloc;

pub mod diag;
pub mod image;
pub mod load;
pub mod plan;
pub mod reason;

pub use diag::Warning;
pub use image::{Image, MAX_IMAGE_LEN, NameRef, Names};
pub use load::{LoadError, VerifiedImage};
pub use plan::{Plan, SensorFSource, SensorISource};
pub use reason::{MAX_REASON_LEN, ReasonView, Segment, Template, TemplateError, Value, VarRef};

// The diagnostic vocabulary an image can be reported in. Re-exported because a
// caller who has an image also has its diagnostics, and asking them to add a
// second dependency to name `Diagnostics` would just be a second version of it.
pub use cyberspine_diag::{Diagnostic, DiagnosticKind, Diagnostics, Level, Location};
