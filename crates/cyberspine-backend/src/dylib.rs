//! Loading a body from a dynamic library.
//!
//! The library exports one symbol ([`abi::ENTRY_SYMBOL`]) that returns a
//! [`AbiV1`] table. After that the host never resolves another symbol: every
//! operation is `call`, one frame in and one frame out, exactly as it is over a
//! socket.
//!
//! The body is destroyed *before* the library is unloaded, which is what the
//! `Drop` impl is for: the table lives in the library, so calling through it
//! after `dlclose` would be a use-after-free.

use std::ffi::{CStr, CString, c_void};
use std::path::Path;

use libloading::Library;

use cyberspine_vm::BackendError;

use crate::abi::{self, AbiV1, status};
use crate::remote::{RemoteBody, Transport};

type Entry = unsafe extern "C" fn() -> *const AbiV1;

/// A loaded library and the one body it produced.
pub struct DylibTransport {
    /// Held only to keep the library mapped: the table and the body live in it.
    _library: Library,
    abi: *const AbiV1,
    body: *mut c_void,
}

impl DylibTransport {
    /// Load `path` and create a body.
    ///
    /// `config` is handed to the body as UTF-8 for the duration of the call; a
    /// body that needs it after `create` must copy it.
    pub fn load(path: &Path, config: Option<&str>) -> Result<Self, BackendError> {
        let library = unsafe { Library::new(path) }.map_err(|error| {
            BackendError::message(format!("cannot load `{}`: {error}", path.display()))
        })?;

        let abi = {
            let entry: libloading::Symbol<Entry> = unsafe { library.get(abi::ENTRY_SYMBOL) }
                .map_err(|error| {
                    BackendError::message(format!(
                        "`{}` does not export `{}`: {error}",
                        path.display(),
                        String::from_utf8_lossy(abi::ENTRY_SYMBOL)
                    ))
                })?;
            let abi = unsafe { entry() };
            if abi.is_null() {
                return Err(BackendError::message(format!(
                    "`{}` returned no function table",
                    path.display()
                )));
            }
            let table = unsafe { &*abi };
            if table.version != abi::ABI_VERSION {
                return Err(BackendError::message(format!(
                    "`{}` speaks ABI {}, this host speaks {}",
                    path.display(),
                    table.version,
                    abi::ABI_VERSION
                )));
            }
            if (table.size as usize) < std::mem::size_of::<AbiV1>() {
                return Err(BackendError::message(format!(
                    "`{}` exports a {}-byte table, this host reads {}",
                    path.display(),
                    table.size,
                    std::mem::size_of::<AbiV1>()
                )));
            }
            abi
        };

        let config = config
            .map(CString::new)
            .transpose()
            .map_err(|_| BackendError::message("the config argument contains a NUL byte"))?;
        let body = unsafe {
            ((*abi).create)(
                config
                    .as_ref()
                    .map_or(std::ptr::null(), |text| text.as_ptr()),
            )
        };
        if body.is_null() {
            return Err(BackendError::message(format!(
                "`{}` could not create a body",
                path.display()
            )));
        }

        Ok(Self {
            _library: library,
            abi,
            body,
        })
    }

    fn vtable(&self) -> &AbiV1 {
        unsafe { &*self.abi }
    }

    fn failure(&self, code: i32) -> BackendError {
        let what = match code {
            status::PROTOCOL => "the body rejected the frame",
            status::BODY => "the body refused the call",
            status::PANIC => "the body panicked",
            _ => "the body returned an unknown status",
        };
        let detail = self.last_error();
        if detail.is_empty() {
            BackendError::message(format!("{what} ({code})"))
        } else {
            BackendError::message(format!("{what} ({code}): {detail}"))
        }
    }

    fn last_error(&self) -> String {
        unsafe {
            let pointer = (self.vtable().last_error)(self.body);
            if pointer.is_null() {
                return String::new();
            }
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    }
}

impl Transport for DylibTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, BackendError> {
        loop {
            let capacity = response.len();
            let mut needed = 0usize;
            let code = unsafe {
                (self.vtable().call)(
                    self.body,
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
                        return Err(BackendError::message(
                            "the body reported a reply larger than the buffer it was given",
                        ));
                    }
                    return Ok(needed);
                }
                status::RESPONSE_TOO_SMALL => {
                    if needed <= capacity {
                        return Err(BackendError::message(
                            "the body asked for a bigger buffer without naming a size",
                        ));
                    }
                    if needed > crate::wire::MAX_FRAME_LEN + crate::wire::HEADER_LEN {
                        return Err(BackendError::message(
                            "the body asked for a reply above the frame cap",
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
            (self.vtable().destroy)(self.body);
        }
        // `library` is dropped by the field after this, so the table is still
        // mapped while `destroy` runs.
    }
}

/// Load a body and introduce the host to it.
pub fn open(path: &Path) -> Result<RemoteBody<DylibTransport>, BackendError> {
    open_with_config(path, None)
}

pub fn open_with_config(
    path: &Path,
    config: Option<&str>,
) -> Result<RemoteBody<DylibTransport>, BackendError> {
    RemoteBody::handshake(DylibTransport::load(path, config)?)
}
