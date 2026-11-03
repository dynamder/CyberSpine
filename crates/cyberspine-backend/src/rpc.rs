//! A body that serves the wire protocol over TCP.
//!
//! The host is the client: a body lives in a process that owns its device, so the
//! body listens and the host connects. The tick loop stays synchronous — the
//! runtime lives inside [`TcpTransport`], and `Session::tick` never sees a future.
//!
//! Two round trips per tick is the shape of the protocol: the host has to resume
//! the instances between `Sample` and `Commit`. What that costs is the achievable
//! tick rate, not the excitation stream: a slow network changes when a tick
//! happens, never what it contains (`docs/isa.md` §9.4).

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Handle, Runtime};

use cyberspine_vm::{Backend, BackendError};

use crate::remote::{RemoteBody, Transport};
use crate::server::Server;
use crate::wire::{self, HEADER_LEN};

/// How long one exchange may take before the body is declared gone.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// The client half: a blocking view of one connection.
pub struct TcpTransport {
    runtime: Runtime,
    stream: TcpStream,
    timeout: Duration,
}

/// Connect to a body and introduce the host to it.
pub fn connect(address: &str) -> Result<RemoteBody<TcpTransport>, BackendError> {
    connect_with_timeout(address, DEFAULT_TIMEOUT)
}

pub fn connect_with_timeout(
    address: &str,
    timeout: Duration,
) -> Result<RemoteBody<TcpTransport>, BackendError> {
    if Handle::try_current().is_ok() {
        return Err(BackendError::message(
            "a remote body blocks its caller; connect from a thread that is not running a tokio runtime",
        ));
    }

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(BackendError::new)?;

    let stream = runtime
        .block_on(async { tokio::time::timeout(timeout, TcpStream::connect(address)).await })
        .map_err(|_| BackendError::message(format!("connecting to `{address}` timed out")))?
        .map_err(|error| {
            BackendError::message(format!("cannot connect to `{address}`: {error}"))
        })?;

    stream.set_nodelay(true).map_err(BackendError::new)?;

    RemoteBody::handshake(TcpTransport {
        runtime,
        stream,
        timeout,
    })
}

impl Transport for TcpTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, BackendError> {
        let Self {
            runtime,
            stream,
            timeout,
        } = self;
        response.clear();

        runtime.block_on(async {
            let exchange = async {
                stream.write_all(request).await.map_err(BackendError::new)?;
                stream.flush().await.map_err(BackendError::new)?;

                let mut header = [0u8; HEADER_LEN];
                stream
                    .read_exact(&mut header)
                    .await
                    .map_err(BackendError::new)?;
                let head = wire::parse_header(&header).map_err(BackendError::new)?;

                let total = HEADER_LEN + head.len;
                response.resize(total, 0);
                response[..HEADER_LEN].copy_from_slice(&header);
                stream
                    .read_exact(&mut response[HEADER_LEN..])
                    .await
                    .map_err(BackendError::new)?;

                Ok(total)
            };

            tokio::time::timeout(*timeout, exchange)
                .await
                .map_err(|_| BackendError::message("the body did not answer in time"))?
        })
    }
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
) -> Result<
    (
        SocketAddr,
        std::thread::JoinHandle<Result<(), BackendError>>,
    ),
    BackendError,
> {
    let server = Server::new(body, rig, build_id)?;

    let listener = std::net::TcpListener::bind(address)
        .map_err(|error| BackendError::message(format!("cannot listen on `{address}`: {error}")))?;
    listener.set_nonblocking(true).map_err(BackendError::new)?;
    let local = listener.local_addr().map_err(BackendError::new)?;

    let thread = std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(BackendError::new)?;
        runtime.block_on(async move {
            let listener = TcpListener::from_std(listener).map_err(BackendError::new)?;
            accept_loop(listener, server).await
        })
    });

    Ok((local, thread))
}

/// Serve until the process ends.
pub fn serve<B: Backend + Send + 'static>(
    address: &str,
    body: B,
    rig: String,
    build_id: String,
) -> Result<(), BackendError> {
    let (_, thread) = spawn(address, body, rig, build_id)?;
    match thread.join() {
        Ok(result) => result,
        Err(_) => Err(BackendError::message("the serving thread panicked")),
    }
}

async fn accept_loop<B: Backend>(
    listener: TcpListener,
    mut server: Server<B>,
) -> Result<(), BackendError> {
    loop {
        let (stream, peer) = listener.accept().await.map_err(BackendError::new)?;
        if let Err(error) = answer(stream, &mut server).await {
            eprintln!("csp: the connection from {peer} ended: {error}");
        }
    }
}

async fn answer<B: Backend>(
    mut stream: TcpStream,
    server: &mut Server<B>,
) -> Result<(), BackendError> {
    stream.set_nodelay(true).map_err(BackendError::new)?;
    let mut frame = Vec::new();

    loop {
        let mut header = [0u8; HEADER_LEN];
        let read = stream
            .read(&mut header[..1])
            .await
            .map_err(BackendError::new)?;
        if read == 0 {
            return Ok(());
        }
        stream
            .read_exact(&mut header[1..])
            .await
            .map_err(BackendError::new)?;

        let head = wire::parse_header(&header).map_err(BackendError::new)?;
        frame.clear();
        frame.extend_from_slice(&header);
        frame.resize(HEADER_LEN + head.len, 0);
        stream
            .read_exact(&mut frame[HEADER_LEN..])
            .await
            .map_err(BackendError::new)?;

        let reply = server.answer(&frame);
        stream.write_all(reply).await.map_err(BackendError::new)?;
        stream.flush().await.map_err(BackendError::new)?;
    }
}
