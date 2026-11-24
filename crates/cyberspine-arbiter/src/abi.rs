//! The C function table an arbiter exports from a dynamic library.
//!
//! The table's shape is [`cyberspine_plugin::abi`]'s, because a body exports the
//! same one under a different symbol. This module pins what makes it an
//! *arbiter*: the symbol a host resolves and the arbiter's spelling of the status
//! vocabulary.

pub use cyberspine_plugin::abi::{ABI_VERSION, AbiV1};

/// The only symbol an arbiter exports.
pub const ENTRY_SYMBOL: &[u8] = b"csp_arbiter_v1";

/// What [`AbiV1::call`] returns.
pub mod status {
    pub use cyberspine_plugin::abi::status::{OK, PANIC, PROTOCOL, REFUSED, RESPONSE_TOO_SMALL};

    /// The arbiter itself refused the call; the reply frame carries the reason.
    pub const ARBITER: i32 = REFUSED;
}
