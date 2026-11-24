//! Loading an arbiter from a dynamic library.
//!
//! The loader and its retry loop are shared with a body
//! ([`cyberspine_plugin::dylib`]); this module is the arbiter's spelling of them:
//! its entry symbol, its noun, and the [`Transport`] impl that turns the loader's
//! sentence into an [`ArbiterError`].
//!
//! The arbiter is destroyed *before* the library is unloaded, which is what the
//! shared transport's `Drop` impl does: the table lives in the library, so calling
//! through it after `dlclose` would be a use-after-free.

use std::path::Path;

pub use cyberspine_plugin::dylib::DylibTransport;

use crate::ArbiterError;
use crate::abi;
use crate::remote::{RemoteArbiter, Transport};

const ENDPOINT: cyberspine_plugin::abi::Endpoint = cyberspine_plugin::abi::Endpoint {
    symbol: abi::ENTRY_SYMBOL,
    noun: "arbiter",
};

impl Transport for DylibTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, ArbiterError> {
        self.exchange_frame(request, response)
            .map_err(ArbiterError::new)
    }
}

/// Load an arbiter and introduce the host to it.
pub fn open(path: &Path) -> Result<RemoteArbiter<DylibTransport>, ArbiterError> {
    open_with_config(path, None)
}

/// Load an arbiter with the config string its `create` will see.
pub fn open_with_config(
    path: &Path,
    config: Option<&str>,
) -> Result<RemoteArbiter<DylibTransport>, ArbiterError> {
    let transport = DylibTransport::load(path, config, ENDPOINT).map_err(ArbiterError::new)?;
    RemoteArbiter::handshake(transport)
}
