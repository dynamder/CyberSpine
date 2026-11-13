//! The dynamic-library packaging of [`scripted::Scripted`].
//!
//! The arbiter itself is an ordinary [`Arbiter`](cyberspine_arbiter::Arbiter);
//! the macro is the whole difference between this crate and a linked one.

pub mod scripted;

pub use scripted::Scripted;

cyberspine_arbiter::export_arbiter! {
    arbiter: Scripted,
    create: |config| Scripted::new(config),
}
