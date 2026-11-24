//! Loading a body from a dynamic library.
//!
//! The loader and its retry loop are shared with the arbiter
//! ([`cyberspine_plugin::dylib`]); this module is the body's spelling of them: its
//! entry symbol, its noun, and the [`Transport`] impl that turns the loader's
//! sentence into a [`BackendError`].
//!
//! The body is destroyed *before* the library is unloaded, which is what the
//! shared transport's `Drop` impl does: the table lives in the library, so calling
//! through it after `dlclose` would be a use-after-free.

use std::path::Path;

use cyberspine_plugin::abi::Endpoint;
use cyberspine_vm::BackendError;

pub use cyberspine_plugin::dylib::DylibTransport;

use crate::abi;
use crate::remote::{RemoteBody, Transport};

const ENDPOINT: Endpoint = Endpoint {
    symbol: abi::ENTRY_SYMBOL,
    noun: "body",
};

impl Transport for DylibTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, BackendError> {
        self.exchange_frame(request, response)
            .map_err(BackendError::message)
    }
}

/// Load a body and introduce the host to it.
pub fn open(path: &Path) -> Result<RemoteBody<DylibTransport>, BackendError> {
    open_with_config(path, None)
}

/// Load a body with the config string its `create` will see.
pub fn open_with_config(
    path: &Path,
    config: Option<&str>,
) -> Result<RemoteBody<DylibTransport>, BackendError> {
    let transport = DylibTransport::load(path, config, ENDPOINT).map_err(BackendError::message)?;
    RemoteBody::handshake(transport)
}
