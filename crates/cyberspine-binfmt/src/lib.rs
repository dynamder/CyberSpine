//! The executable image: its layout (`docs/binfmt.md`) and the loader that
//! validates one.
//!
//! `#![no_std]` with only `alloc`: the same decoder runs where the action runs,
//! including on a microcontroller. It knows `isa` and `rig` and nothing about
//! execution or a platform.

#![no_std]

extern crate alloc;

pub mod image;
pub mod load;

pub use image::Image;
pub use load::{LoadError, VerifiedImage};
