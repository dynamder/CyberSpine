//! The dynamic-library packaging of [`body::Panicky`].
//!
//! `export!` wraps every entry in `catch_unwind`, so a panic inside a body is an
//! ABI failure the host can report instead of an unwind across the boundary
//! (`docs/backend.md` §5). This is the only body in the tree that panics, and it
//! exists so that guard has a test.

pub mod body;

pub use body::{PANIC_REASON, Panicky, RIG_JSON, rig};

cyberspine_backend::export! {
    body: Panicky,
    rig: RIG_JSON,
    create: |_config| Panicky::new(),
}
