//! The C function table a dynamic-library body exports.
//!
//! The table's shape is [`cyberspine_plugin::abi`]'s, because an arbiter exports
//! the same one under a different symbol. This module pins what makes it a
//! *body*: the symbol a host resolves and the body's spelling of the status
//! vocabulary.

pub use cyberspine_plugin::abi::{ABI_VERSION, AbiV1};

/// The only symbol a body exports.
pub const ENTRY_SYMBOL: &[u8] = b"csp_backend_v1";

/// What [`AbiV1::call`] returns.
///
/// `0` is success. Everything else means the host must not trust the response
/// buffer: a status is a failure signal, not a partial result.
pub mod status {
    pub use cyberspine_plugin::abi::status::{OK, PANIC, PROTOCOL, REFUSED, RESPONSE_TOO_SMALL};

    /// The body refused the call; the reply frame carries the reason.
    pub const BODY: i32 = REFUSED;
}
