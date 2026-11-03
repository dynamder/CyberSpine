//! The dynamic-library packaging of [`body::Fixture`].
//!
//! The body itself is an ordinary [`Backend`](cyberspine_vm::Backend); the macro
//! is the whole difference between this crate and a linked one.

pub mod body;

pub use body::{Fixture, RIG_JSON, rig};

cyberspine_backend::export! {
    body: Fixture,
    rig: RIG_JSON,
    create: |_config| Fixture::new(),
}
