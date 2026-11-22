//! The shared vocabulary for everything the toolchain reports.
//!
//! A diagnostic is a *fact with an owner and a place*, not a sentence. The crates
//! that detect problems build one of these and push it into a [`Diagnostics`]
//! sink; the crate that knows how to print builds its own renderer on top. This
//! crate holds the shape and nothing else, which is what keeps the assembler, the
//! loader and the runtime from each growing their own output format.
//!
//! Two rules that the types enforce:
//!
//! - A [`Diagnostic`] always names the artifact it belongs to. There is no
//!   "assume the only file" state, because a fault's instruction lives in the
//!   image while its source line lives in another file entirely.
//! - [`Coded`] has no default `code()`. Adding a variant to an error enum
//!   without giving it a stable identifier is a compile error, not a silent gap.
//!
//! It is `#![no_std]` with only `alloc`: the loader runs on a microcontroller,
//! where rendering is somebody else's business.

#![no_std]

extern crate alloc;

mod cause;
mod code;
mod coded;
mod diagnostic;
mod level;
mod location;
mod sink;
mod site;

pub use cause::Cause;
pub use code::Code;
pub use coded::{Coded, table_is_sane};
pub use diagnostic::Diagnostic;
pub use level::Level;
pub use location::Location;
pub use sink::Diagnostics;
pub use site::{Anchor, Label, Site};

/// `diagnostics.md` numbers every code; this is the number the docs use.
pub const SCHEMA: u32 = 1;
