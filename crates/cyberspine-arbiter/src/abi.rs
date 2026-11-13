//! The C function table an arbiter exports from a dynamic library.
//!
//! The shape is deliberately identical to `cyberspine_backend::abi`: one symbol,
//! one table, one entry point, and every operation is [`AbiV1::call`] carrying
//! one [`wire`](crate::wire) frame in and one out. The symbol and the version
//! namespace are separate because a body and an arbiter are separate plugins; a
//! host that loads the wrong one fails at the symbol.
//!
//! The table is `#[repr(C)]` and contains nothing but integers, C pointers and
//! `extern "C"` function pointers, so an arbiter built by a different compiler —
//! or in a different language — can produce it.

use std::ffi::{c_char, c_void};

/// The value [`AbiV1::version`] must hold.
pub const ABI_VERSION: u32 = 1;

/// The only symbol an arbiter exports.
pub const ENTRY_SYMBOL: &[u8] = b"csp_arbiter_v1";

/// What [`AbiV1::call`] returns.
pub mod status {
    /// The request was handled and `*response_len` bytes of `response` are valid.
    pub const OK: i32 = 0;
    /// The response buffer was too small; `*response_len` holds the needed size.
    pub const RESPONSE_TOO_SMALL: i32 = 1;
    /// The request was not a valid frame, or not one this arbiter expects here.
    pub const PROTOCOL: i32 = 2;
    /// The arbiter itself refused the call; the reply frame carries the reason.
    pub const ARBITER: i32 = 3;
    /// The arbiter panicked. The SDK catches it; a raw arbiter that does not will
    /// abort the process, which is why the SDK exists.
    pub const PANIC: i32 = 4;
}

/// The function table behind [`ENTRY_SYMBOL`].
#[repr(C)]
pub struct AbiV1 {
    pub version: u32,
    /// `size_of::<AbiV1>()` as the *arbiter* compiled it.
    pub size: u32,
    pub create: unsafe extern "C" fn(config: *const c_char) -> *mut c_void,
    pub destroy: unsafe extern "C" fn(arbiter: *mut c_void),
    pub call: unsafe extern "C" fn(
        arbiter: *mut c_void,
        request: *const u8,
        request_len: usize,
        response: *mut u8,
        response_cap: usize,
        response_len: *mut usize,
    ) -> i32,
    pub last_error: unsafe extern "C" fn(arbiter: *mut c_void) -> *const c_char,
}
