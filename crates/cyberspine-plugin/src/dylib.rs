//! Loading a plugin from a dynamic library.
//!
//! The library exports one symbol that returns an [`AbiV1`] table. After that the
//! host never resolves another symbol: every operation is `call`, one frame in and
//! one frame out, exactly as it is over a socket.
//!
//! The plugin is destroyed *before* the library is unloaded, which is what the
//! `Drop` impl is for: the table lives in the library, so calling through it after
//! `dlclose` would be a use-after-free.

use std::ffi::{CStr, CString, c_void};
use std::path::Path;

use libloading::Library;

use crate::abi::{self, AbiV1, Endpoint, status};

type Entry = unsafe extern "C" fn() -> *const AbiV1;

/// A loaded library and the one plugin it produced.
pub struct DylibTransport {
    /// Held only to keep the library mapped: the table and the plugin live in it.
    _library: Library,
    abi: *const AbiV1,
    plugin: *mut c_void,
    endpoint: Endpoint,
}

impl DylibTransport {
    /// Load `path` and create a plugin.
    ///
    /// `config` is handed to the plugin as UTF-8 for the duration of the call; a
    /// plugin that needs it after `create` must copy it.
    ///
    /// A failure is a sentence with no state behind it: the loader cannot describe
    /// the plugin before it has one, so it returns the text the caller will wrap
    /// in its own error type.
    pub fn load(path: &Path, config: Option<&str>, endpoint: Endpoint) -> Result<Self, String> {
        let library = unsafe { Library::new(path) }
            .map_err(|error| format!("cannot load `{}`: {error}", path.display()))?;

        let abi = {
            let entry: libloading::Symbol<Entry> = unsafe { library.get(endpoint.symbol) }
                .map_err(|error| {
                    format!(
                        "`{}` does not export `{}`: {error}",
                        path.display(),
                        String::from_utf8_lossy(endpoint.symbol)
                    )
                })?;
            let abi = unsafe { entry() };
            if abi.is_null() {
                return Err(format!("`{}` returned no function table", path.display()));
            }
            unsafe { abi::check_table(abi) }
                .map_err(|error| format!("`{}` {error}", path.display()))?;
            abi
        };

        let config = config
            .map(CString::new)
            .transpose()
            .map_err(|_| "the config argument contains a NUL byte".to_owned())?;
        let plugin = unsafe {
            ((*abi).create)(
                config
                    .as_ref()
                    .map_or(std::ptr::null(), |text| text.as_ptr()),
            )
        };
        if plugin.is_null() {
            return Err(format!(
                "`{}` could not create a {}",
                path.display(),
                endpoint.noun
            ));
        }

        Ok(Self {
            _library: library,
            abi,
            plugin,
            endpoint,
        })
    }

    fn vtable(&self) -> &AbiV1 {
        unsafe { &*self.abi }
    }

    fn failure(&self, code: i32) -> String {
        let what = match code {
            status::PROTOCOL => "rejected the frame",
            status::REFUSED => "refused the call",
            status::PANIC => "panicked",
            _ => "returned an unknown status",
        };
        let detail = self.last_error();
        if detail.is_empty() {
            format!("the {} {what} ({code})", self.endpoint.noun)
        } else {
            format!("the {} {what} ({code}): {detail}", self.endpoint.noun)
        }
    }

    fn last_error(&self) -> String {
        unsafe {
            let pointer = (self.vtable().last_error)(self.plugin);
            if pointer.is_null() {
                return String::new();
            }
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    }

    /// One request frame in, one reply frame out.
    ///
    /// A reply that does not fit runs the request again with a bigger buffer; the
    /// protocol's `seq` makes that a lookup on the plugin side rather than a second
    /// delivery. The length is separate from `response.len()` so the transport can
    /// keep its buffer at capacity and reuse it without zeroing or shrinking.
    pub fn exchange_frame(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<usize, String> {
        loop {
            let capacity = response.len();
            let mut needed = 0usize;
            let code = unsafe {
                (self.vtable().call)(
                    self.plugin,
                    request.as_ptr(),
                    request.len(),
                    response.as_mut_ptr(),
                    capacity,
                    &mut needed,
                )
            };

            match code {
                status::OK => {
                    if needed > capacity {
                        return Err(format!(
                            "the {} reported a reply larger than the buffer it was given",
                            self.endpoint.noun
                        ));
                    }
                    return Ok(needed);
                }
                status::RESPONSE_TOO_SMALL => {
                    if needed <= capacity {
                        return Err(format!(
                            "the {} asked for a bigger buffer without naming a size",
                            self.endpoint.noun
                        ));
                    }
                    if needed > cyberspine_frame::MAX_FRAME_LEN + cyberspine_frame::HEADER_LEN {
                        return Err(format!(
                            "the {} asked for a reply above the frame cap",
                            self.endpoint.noun
                        ));
                    }
                    response.resize(needed, 0);
                }
                other => return Err(self.failure(other)),
            }
        }
    }
}

impl Drop for DylibTransport {
    fn drop(&mut self) {
        unsafe {
            (self.vtable().destroy)(self.plugin);
        }
        // `library` is dropped by the field after this, so the table is still
        // mapped while `destroy` runs.
    }
}
