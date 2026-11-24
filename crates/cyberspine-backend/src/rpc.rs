//! A body that serves the wire protocol over TCP.
//!
//! The host is the client: a body lives in a process that owns its device, so the
//! body listens and the host connects. The tick loop stays synchronous — the
//! runtime lives inside the shared [`TcpTransport`], and `Session::tick` never
//! sees a future.
//!
//! Two round trips per tick is the shape of the protocol: the host has to resume
//! the instances between `Sample` and `Commit`. What that costs is the achievable
//! tick rate, not the excitation stream: a slow network changes when a tick
//! happens, never what it contains (`docs/isa.md` §9.4).

use std::net::SocketAddr;
use std::thread::JoinHandle;
use std::time::Duration;

use cyberspine_frame as frame;
use cyberspine_vm::{Backend, BackendError};

pub use cyberspine_plugin::rpc::TcpTransport;

use crate::remote::{RemoteBody, Transport};
use crate::server::Server;
use crate::wire;

/// How long one exchange may take before the body is declared gone.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// The head this crate's frames carry, for the shared socket loop.
const PROTOCOL: frame::Protocol = frame::Protocol {
    magic: wire::MAGIC,
    version: wire::VERSION,
};

impl Transport for TcpTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, BackendError> {
        self.exchange_frame(PROTOCOL, request, response)
            .map_err(BackendError::message)
    }
}

/// Connect to a body and introduce the host to it.
pub fn connect(address: &str) -> Result<RemoteBody<TcpTransport>, BackendError> {
    connect_with_timeout(address, DEFAULT_TIMEOUT)
}

/// Connect with an exchange deadline other than [`DEFAULT_TIMEOUT`].
pub fn connect_with_timeout(
    address: &str,
    timeout: Duration,
) -> Result<RemoteBody<TcpTransport>, BackendError> {
    let transport = TcpTransport::connect(
        address,
        timeout,
        "body",
        "a remote body blocks its caller; connect from a thread that is not running a tokio \
         runtime",
    )
    .map_err(BackendError::message)?;
    RemoteBody::handshake(transport)
}

/// Serve `body` on `address`.
///
/// The listener is bound before the thread starts, so a bad address or a taken
/// port comes back as an error instead of disappearing into a background thread.
/// The returned handle carries the thread's own result, which is only `Err` when
/// the listener fails; a broken connection is logged and the next one is served.
pub fn spawn<B: Backend + Send + 'static>(
    address: &str,
    body: B,
    rig: String,
    build_id: String,
) -> Result<(SocketAddr, JoinHandle<Result<(), BackendError>>), BackendError> {
    let server = Server::new(body, rig, build_id)?;
    cyberspine_plugin::rpc::spawn(address, PROTOCOL, server, BackendError::message)
}

/// Serve until the process ends.
pub fn serve<B: Backend + Send + 'static>(
    address: &str,
    body: B,
    rig: String,
    build_id: String,
) -> Result<(), BackendError> {
    let server = Server::new(body, rig, build_id)?;
    cyberspine_plugin::rpc::serve(address, PROTOCOL, server, BackendError::message)
}
