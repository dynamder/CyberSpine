//! The C function table a dynamic-library body exports.
//!
//! One symbol, one table, one entry point: everything a loaded body can do is
//! [`AbiV1::call`] carrying one [`wire`](crate::wire) frame in and one out. The
//! library path and the socket path therefore decode the same bytes, and a body
//! written for one can be served by the other without a second protocol.
//!
//! The table is `#[repr(C)]` and contains nothing but integers, C pointers and
//! `extern "C"` function pointers, so a body built by a different compiler — or
//! in a different language — can produce it.

use std::ffi::{c_char, c_void};

/// The value [`AbiV1::version`] must hold.
pub const ABI_VERSION: u32 = 1;

/// The only symbol a body exports.
pub const ENTRY_SYMBOL: &[u8] = b"csp_backend_v1";

/// What [`AbiV1::call`] returns.
///
/// `0` is success. Everything else means the host must not trust the response
/// buffer: a status is a failure signal, not a partial result.
pub mod status {
    /// The request was handled and `*response_len` bytes of `response` are valid.
    pub const OK: i32 = 0;
    /// The response buffer was too small; `*response_len` holds the needed size.
    pub const RESPONSE_TOO_SMALL: i32 = 1;
    /// The request was not a valid frame, or not one this body expects here.
    pub const PROTOCOL: i32 = 2;
    /// The body itself refused the call; the reply frame carries the reason.
    pub const BODY: i32 = 3;
    /// The body panicked. The SDK catches it; a raw body that does not will abort
    /// the process, which is why the SDK exists.
    pub const PANIC: i32 = 4;
}

/// The function table behind [`ENTRY_SYMBOL`].
///
/// `version` and `size` come first so an older host can read the table an older
/// body produced: read the two words, then refuse if `version` is not
/// [`ABI_VERSION`] or `size` is smaller than the fields the host is about to use.
#[repr(C)]
pub struct AbiV1 {
    pub version: u32,
    /// `size_of::<AbiV1>()` as the *body* compiled it.
    pub size: u32,
    /// Allocate a body. `config` may be null; when it is not, it is a
    /// NUL-terminated UTF-8 string that is only valid for this call, so a body
    /// that needs it later must copy it.
    ///
    /// Null means the library could not create a handle at all. A body that
    /// exists but was configured wrongly returns a handle anyway and reports the
    /// reason through [`AbiV1::last_error`], so the host can say what went wrong
    /// instead of "refused".
    pub create: unsafe extern "C" fn(config: *const c_char) -> *mut c_void,
    /// Free a body created by [`AbiV1::create`]. Must run before the library is
    /// unloaded.
    pub destroy: unsafe extern "C" fn(body: *mut c_void),
    /// Handle one request frame. On success `*response_len` is the reply length;
    /// on [`status::RESPONSE_TOO_SMALL`] it is the size the caller must provide.
    pub call: unsafe extern "C" fn(
        body: *mut c_void,
        request: *const u8,
        request_len: usize,
        response: *mut u8,
        response_cap: usize,
        response_len: *mut usize,
    ) -> i32,
    /// The text behind the last failed [`AbiV1::call`], cleared by the next
    /// successful one and by `destroy`. Null when there is nothing to say.
    pub last_error: unsafe extern "C" fn(body: *mut c_void) -> *const c_char,
}
