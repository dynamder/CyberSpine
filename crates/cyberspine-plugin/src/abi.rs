//! The C function table a plugin exports from a dynamic library.
//!
//! One symbol, one table, one entry point: everything a loaded plugin can do is
//! [`AbiV1::call`] carrying one frame in and one out. A body and an arbiter export
//! the same table under different symbols and magics, so a host that loads the
//! wrong one fails at the symbol instead of decoding the wrong messages.
//!
//! The table is `#[repr(C)]` and contains nothing but integers, C pointers and
//! `extern "C"` function pointers, so a plugin built by a different compiler — or
//! in a different language — can produce it.
//!
//! # Reading a table is two steps
//!
//! [`AbiV1::size`] exists so a host can refuse a table it does not understand. It
//! is therefore read *first*, together with [`AbiV1::version`], before the host
//! forms a reference to the rest of the table. [`check_table`] is the only way to
//! do that, and the loader must call it before touching any other field.

use std::ffi::{c_char, c_void};

/// The value [`AbiV1::version`] must hold.
pub const ABI_VERSION: u32 = 1;

/// What [`AbiV1::call`] returns.
///
/// `0` is success. Everything else means the host must not trust the response
/// buffer: a status is a failure signal, not a partial result.
pub mod status {
    /// The request was handled and `*response_len` bytes of `response` are valid.
    pub const OK: i32 = 0;
    /// The response buffer was too small; `*response_len` holds the needed size.
    pub const RESPONSE_TOO_SMALL: i32 = 1;
    /// The request was not a valid frame, or not one this plugin expects here.
    pub const PROTOCOL: i32 = 2;
    /// The plugin itself refused the call; the reply frame carries the reason.
    pub const REFUSED: i32 = 3;
    /// The plugin panicked. The SDK catches it; a raw plugin that does not will
    /// abort the process, which is why the SDK exists.
    pub const PANIC: i32 = 4;
}

/// The function table behind a plugin's entry symbol.
///
/// `version` and `size` come first so an older host can read the table an older
/// plugin produced: read the two words, then refuse if `version` is not
/// [`ABI_VERSION`] or `size` is smaller than the table the host uses.
#[repr(C)]
pub struct AbiV1 {
    /// The ABI version the plugin was built for.
    pub version: u32,
    /// `size_of::<AbiV1>()` as the *plugin* compiled it.
    pub size: u32,
    /// Allocate a plugin. `config` may be null; when it is not, it is a
    /// NUL-terminated UTF-8 string that is only valid for this call, so a plugin
    /// that needs it later must copy it.
    ///
    /// Null means the library could not create a handle at all. A plugin that
    /// exists but was configured wrongly returns a handle anyway and reports the
    /// reason through [`AbiV1::last_error`], so the host can say what went wrong
    /// instead of "refused".
    pub create: unsafe extern "C" fn(config: *const c_char) -> *mut c_void,
    /// Free a plugin created by [`AbiV1::create`]. Must run before the library is
    /// unloaded.
    pub destroy: unsafe extern "C" fn(plugin: *mut c_void),
    /// Handle one request frame. On success `*response_len` is the reply length;
    /// on [`status::RESPONSE_TOO_SMALL`] it is the size the caller must provide.
    pub call: unsafe extern "C" fn(
        plugin: *mut c_void,
        request: *const u8,
        request_len: usize,
        response: *mut u8,
        response_cap: usize,
        response_len: *mut usize,
    ) -> i32,
    /// The text behind the last failed [`AbiV1::call`], cleared by the next
    /// successful one and by `destroy`. Null when there is nothing to say.
    pub last_error: unsafe extern "C" fn(plugin: *mut c_void) -> *const c_char,
}

/// What a plugin calls itself in a message: its entry symbol and its noun.
///
/// The symbol is what a host resolves; the noun ("body", "arbiter") is the word
/// the shared loader puts in the sentence, so the message still describes the
/// thing the user loaded.
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    /// The only symbol the plugin exports.
    pub symbol: &'static [u8],
    /// The noun those messages use.
    pub noun: &'static str,
}

/// Why a function table was refused.
#[derive(Debug, thiserror::Error)]
pub enum AbiError {
    /// The plugin speaks another ABI version.
    #[error("speaks ABI {found}, this host speaks {expected}")]
    Version {
        /// The version the table declares.
        found: u32,
        /// The version the host implements.
        expected: u32,
    },
    /// The table is smaller than the one the host reads.
    #[error("exports a {found}-byte table, this host reads {needed}")]
    Size {
        /// The size the table declares.
        found: u32,
        /// The size the host reads.
        needed: usize,
    },
    /// A function pointer the host would call is null.
    #[error("exports a table whose `{0}` is null")]
    Null(&'static str),
}

/// Check a table before the host forms a reference to it.
///
/// # Safety
///
/// `table` must point to at least two `u32`s — the `version` and `size` fields —
/// that stay readable for the call. Every other field is read only after `size`
/// says the table is big enough.
pub unsafe fn check_table(table: *const AbiV1) -> Result<(), AbiError> {
    let words = table.cast::<u32>();
    // The two leading words are the one promise every ABI version makes, so they
    // are readable before `size` is known.
    let version = unsafe { words.read_unaligned() };
    let size = unsafe { words.add(1).read_unaligned() };

    if version != ABI_VERSION {
        return Err(AbiError::Version {
            found: version,
            expected: ABI_VERSION,
        });
    }
    if (size as usize) < std::mem::size_of::<AbiV1>() {
        return Err(AbiError::Size {
            found: size,
            needed: std::mem::size_of::<AbiV1>(),
        });
    }

    // The function pointers are read as raw pointers, not through the typed
    // table: a null `fn` is not a value the Rust type system can represent, so a
    // comparison on the field would be optimized away. The raw pointer is the
    // only honest view of what a C plugin actually wrote.
    let (create, destroy, call, last_error) = unsafe {
        (
            field(table, std::mem::offset_of!(AbiV1, create)),
            field(table, std::mem::offset_of!(AbiV1, destroy)),
            field(table, std::mem::offset_of!(AbiV1, call)),
            field(table, std::mem::offset_of!(AbiV1, last_error)),
        )
    };
    if create.is_null() {
        return Err(AbiError::Null("create"));
    }
    if destroy.is_null() {
        return Err(AbiError::Null("destroy"));
    }
    if call.is_null() {
        return Err(AbiError::Null("call"));
    }
    if last_error.is_null() {
        return Err(AbiError::Null("last_error"));
    }
    Ok(())
}

/// Read the pointer-sized word at `offset` as a raw pointer.
unsafe fn field(table: *const AbiV1, offset: usize) -> *const () {
    unsafe {
        table
            .cast::<u8>()
            .add(offset)
            .cast::<*const ()>()
            .read_unaligned()
    }
}

#[cfg(test)]
mod tests {
    use super::{ABI_VERSION, AbiError, AbiV1, check_table, status};
    use std::ffi::{c_char, c_void};
    use std::ptr;

    unsafe extern "C" fn create(_config: *const c_char) -> *mut c_void {
        ptr::null_mut()
    }

    unsafe extern "C" fn destroy(_plugin: *mut c_void) {}

    unsafe extern "C" fn call(
        _plugin: *mut c_void,
        _request: *const u8,
        _request_len: usize,
        _response: *mut u8,
        _response_cap: usize,
        _response_len: *mut usize,
    ) -> i32 {
        status::OK
    }

    unsafe extern "C" fn last_error(_plugin: *mut c_void) -> *const c_char {
        ptr::null()
    }

    #[test]
    fn a_complete_table_passes() {
        let table = AbiV1 {
            version: ABI_VERSION,
            size: std::mem::size_of::<AbiV1>() as u32,
            create,
            destroy,
            call,
            last_error,
        };
        unsafe { check_table(&table) }.expect("every field is present");
    }

    #[test]
    fn a_narrower_table_is_refused_before_it_is_read() {
        // Only the two words every version promises: reading `create` here would
        // be past the end of the allocation.
        let words: [u32; 2] = [ABI_VERSION, 8];

        let error = unsafe { check_table(words.as_ptr().cast::<AbiV1>()) }
            .expect_err("the host needs more than eight bytes");

        assert!(matches!(error, AbiError::Size { found: 8, .. }), "{error}");
    }

    #[test]
    fn another_version_is_refused() {
        let mut table = AbiV1 {
            version: ABI_VERSION,
            size: std::mem::size_of::<AbiV1>() as u32,
            create,
            destroy,
            call,
            last_error,
        };
        table.version = ABI_VERSION + 1;

        let error = unsafe { check_table(&table) }.expect_err("the host speaks one version");

        assert!(
            matches!(error, AbiError::Version { found, .. } if found == ABI_VERSION + 1),
            "{error}"
        );
    }

    #[test]
    fn a_null_function_pointer_is_refused() {
        // A C plugin that writes a null into the table has a layout Rust cannot
        // spell as an `AbiV1` value, so the test writes the same words a C
        // compiler would: version, size, then the four pointers.
        #[repr(C)]
        struct RawTable {
            version: u32,
            size: u32,
            create: *const (),
            destroy: *const (),
            call: *const (),
            last_error: *const (),
        }

        let complete = RawTable {
            version: ABI_VERSION,
            size: std::mem::size_of::<RawTable>() as u32,
            create: create as *const (),
            destroy: destroy as *const (),
            call: call as *const (),
            last_error: last_error as *const (),
        };
        assert_eq!(
            complete.size as usize,
            std::mem::size_of::<AbiV1>(),
            "the test's raw layout is the exported one"
        );
        unsafe { check_table((&raw const complete).cast::<AbiV1>()) }
            .expect("every pointer is present");

        let null = RawTable {
            last_error: std::ptr::null(),
            ..complete
        };
        let error = unsafe { check_table((&raw const null).cast::<AbiV1>()) }
            .expect_err("the host would jump to null");

        assert!(matches!(error, AbiError::Null("last_error")), "{error}");
    }
}
