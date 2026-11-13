//! Loading an arbiter from a dynamic library.
//!
//! The library exports one symbol ([`abi::ENTRY_SYMBOL`]) that returns an
//! [`AbiV1`] table. After that the host never resolves another symbol: every
//! operation is `call`, one frame in and one frame out, exactly as it is over a
//! socket.
//!
//! The arbiter is destroyed *before* the library is unloaded, which is what the
//! `Drop` impl is for: the table lives in the library, so calling through it
//! after `dlclose` would be a use-after-free.

use std::ffi::{CStr, CString, c_void};
use std::path::Path;

use libloading::Library;

use crate::ArbiterError;
use crate::abi::{self, AbiV1, status};
use crate::remote::{RemoteArbiter, Transport};

type Entry = unsafe extern "C" fn() -> *const AbiV1;

/// A loaded library and the one arbiter it produced.
pub struct DylibTransport {
    /// Held only to keep the library mapped: the table and the arbiter live in it.
    _library: Library,
    abi: *const AbiV1,
    arbiter: *mut c_void,
}

impl DylibTransport {
    /// Load `path` and create an arbiter.
    ///
    /// `config` is handed to the arbiter as UTF-8 for the duration of the call; an
    /// arbiter that needs it after `create` must copy it.
    pub fn load(path: &Path, config: Option<&str>) -> Result<Self, ArbiterError> {
        let library = unsafe { Library::new(path) }.map_err(|error| {
            ArbiterError(format!("cannot load `{}`: {error}", path.display()).into_boxed_str())
        })?;

        let abi = {
            let entry: libloading::Symbol<Entry> = unsafe { library.get(abi::ENTRY_SYMBOL) }
                .map_err(|error| {
                    ArbiterError(
                        format!(
                            "`{}` does not export `{}`: {error}",
                            path.display(),
                            String::from_utf8_lossy(abi::ENTRY_SYMBOL)
                        )
                        .into_boxed_str(),
                    )
                })?;
            let abi = unsafe { entry() };
            if abi.is_null() {
                return Err(ArbiterError(
                    format!("`{}` returned no function table", path.display()).into_boxed_str(),
                ));
            }
            let table = unsafe { &*abi };
            if table.version != abi::ABI_VERSION {
                return Err(ArbiterError(
                    format!(
                        "`{}` speaks ABI {}, this host speaks {}",
                        path.display(),
                        table.version,
                        abi::ABI_VERSION
                    )
                    .into_boxed_str(),
                ));
            }
            if (table.size as usize) < std::mem::size_of::<AbiV1>() {
                return Err(ArbiterError(
                    format!(
                        "`{}` exports a {}-byte table, this host reads {}",
                        path.display(),
                        table.size,
                        std::mem::size_of::<AbiV1>()
                    )
                    .into_boxed_str(),
                ));
            }
            abi
        };

        let config = config
            .map(CString::new)
            .transpose()
            .map_err(|_| ArbiterError("the config argument contains a NUL byte".into()))?;
        let arbiter = unsafe {
            ((*abi).create)(
                config
                    .as_ref()
                    .map_or(std::ptr::null(), |text| text.as_ptr()),
            )
        };
        if arbiter.is_null() {
            return Err(ArbiterError(
                format!("`{}` could not create an arbiter", path.display()).into_boxed_str(),
            ));
        }

        Ok(Self {
            _library: library,
            abi,
            arbiter,
        })
    }

    fn vtable(&self) -> &AbiV1 {
        unsafe { &*self.abi }
    }

    fn last_error(&self) -> String {
        unsafe {
            let pointer = (self.vtable().last_error)(self.arbiter);
            if pointer.is_null() {
                return String::new();
            }
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    }

    fn failure(&self, code: i32) -> ArbiterError {
        let what = match code {
            status::PROTOCOL => "the arbiter rejected the frame",
            status::ARBITER => "the arbiter refused the call",
            status::PANIC => "the arbiter panicked",
            _ => "the arbiter returned an unknown status",
        };
        let detail = self.last_error();
        if detail.is_empty() {
            ArbiterError(format!("{what} ({code})").into_boxed_str())
        } else {
            ArbiterError(format!("{what} ({code}): {detail}").into_boxed_str())
        }
    }
}

impl Transport for DylibTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, ArbiterError> {
        loop {
            let capacity = response.len();
            let mut needed = 0usize;
            let code = unsafe {
                (self.vtable().call)(
                    self.arbiter,
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
                        return Err(ArbiterError(
                            "the arbiter reported a reply larger than the buffer it was given"
                                .into(),
                        ));
                    }
                    return Ok(needed);
                }
                status::RESPONSE_TOO_SMALL => {
                    if needed <= capacity {
                        return Err(ArbiterError(
                            "the arbiter asked for a bigger buffer without naming a size".into(),
                        ));
                    }
                    if needed > crate::wire::MAX_FRAME_LEN + crate::wire::HEADER_LEN {
                        return Err(ArbiterError(
                            "the arbiter asked for a reply above the frame cap".into(),
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
            (self.vtable().destroy)(self.arbiter);
        }
        // `library` is dropped by the field after this, so the table is still
        // mapped while `destroy` runs.
    }
}

/// Load an arbiter and introduce the host to it.
pub fn open(path: &Path) -> Result<RemoteArbiter<DylibTransport>, ArbiterError> {
    open_with_config(path, None)
}

pub fn open_with_config(
    path: &Path,
    config: Option<&str>,
) -> Result<RemoteArbiter<DylibTransport>, ArbiterError> {
    RemoteArbiter::handshake(DylibTransport::load(path, config)?)
}
