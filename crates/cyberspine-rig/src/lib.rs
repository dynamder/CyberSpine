//! The rig manifest, as JSON.
//!
//! The assembler reads this file to know the body an action is being built for;
//! the backend builds the same [`Rig`] in code. Both hash it with
//! `core::rig::Rig::fingerprint`, and the host refuses to run an image whose
//! fingerprint does not match the body it is attached to.
//!
//! ```json
//! {
//!   "rig": { "name": "live2d-standard", "version": 1 },
//!   "muscles": ["mouth_open", "brow_l"],
//!   "sensors_f": ["blink_speed"],
//!   "sensors_i": ["gaze_lock"]
//! }
//! ```

pub mod manifest;

pub use cyberspine_core::rig::{Fingerprint, Rig};
pub use manifest::{ManifestError, from_json};
