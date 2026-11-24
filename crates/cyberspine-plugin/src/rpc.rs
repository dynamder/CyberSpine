//! Serving a plugin over TCP, and the blocking client transport that reaches it.
//!
//! The host is the client and the plugin listens. The tick loop stays
//! synchronous — the runtime lives inside [`TcpTransport`] — and the serving side
//! gives every connection its own task and its own [`Connection`] cursor.
//!
//! The frame head is [`cyberspine_frame`]'s; this module only adds the socket
//! framing above it. The protocol (magic and version) is a parameter because a
//! body and an arbiter speak different ones.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Handle, Runtime};

use cyberspine_frame::{self as frame, HEADER_LEN};

use crate::server::{Connection, Serve};

/// The client half: a blocking view of one connection.
pub struct TcpTransport {
    runtime: Runtime,
    stream: TcpStream,
    timeout: Duration,
    noun: &'static str,
}

impl TcpTransport {
    /// Connect to `address`, refusing to run inside a runtime that would deadlock.
    ///
    /// `blocked` is the caller's sentence for that refusal: the body's caller and
    /// the arbiter's worker are different things to tell the user about. `noun` is
    /// the word the exchange's own errors use.
    pub fn connect(
        address: &str,
        timeout: Duration,
        noun: &'static str,
        blocked: &'static str,
    ) -> Result<Self, String> {
        if Handle::try_current().is_ok() {
            return Err(blocked.to_owned());
        }

        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;

        let stream = runtime
            .block_on(async { tokio::time::timeout(timeout, TcpStream::connect(address)).await })
            .map_err(|_| format!("connecting to `{address}` timed out"))?
            .map_err(|error| format!("cannot connect to `{address}`: {error}"))?;

        stream
            .set_nodelay(true)
            .map_err(|error| error.to_string())?;

        Ok(Self {
            runtime,
            stream,
            timeout,
            noun,
        })
    }

    /// One request frame in, one reply frame out.
    pub fn exchange_frame(
        &mut self,
        protocol: frame::Protocol,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<usize, String> {
        let Self {
            runtime,
            stream,
            timeout,
            noun,
        } = self;
        response.clear();

        runtime.block_on(async {
            let exchange = async {
                stream
                    .write_all(request)
                    .await
                    .map_err(|error| error.to_string())?;
                stream.flush().await.map_err(|error| error.to_string())?;

                let mut header = [0u8; HEADER_LEN];
                stream
                    .read_exact(&mut header)
                    .await
                    .map_err(|error| error.to_string())?;
                let head =
                    frame::parse_header(&protocol, &header).map_err(|error| error.to_string())?;

                let total = HEADER_LEN + head.len;
                response.resize(total, 0);
                response[..HEADER_LEN].copy_from_slice(&header);
                stream
                    .read_exact(&mut response[HEADER_LEN..])
                    .await
                    .map_err(|error| error.to_string())?;

                Ok(total)
            };

            tokio::time::timeout(*timeout, exchange)
                .await
                .map_err(|_| format!("the {noun} did not answer in time"))?
        })
    }
}

/// Serve `server` on `address`.
///
/// The listener is bound before the thread starts, so a bad address or a taken
/// port comes back as an error instead of disappearing into a background thread.
/// The returned handle carries the thread's own result, which is only `Err` when
/// the listener fails; a broken connection is logged and the next one is served.
///
/// `fail` is the caller's error constructor: the loop knows the sentence but not
/// the type the crate that owns the plugin wants it wrapped in.
pub fn spawn<S, E, F>(
    address: &str,
    protocol: frame::Protocol,
    server: S,
    fail: F,
) -> Result<(SocketAddr, JoinHandle<Result<(), E>>), E>
where
    S: Serve,
    E: Send + 'static,
    F: Fn(String) -> E + Send + Clone + 'static,
{
    let listener = std::net::TcpListener::bind(address)
        .map_err(|error| fail(format!("cannot listen on `{address}`: {error}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| fail(error.to_string()))?;
    let local = listener
        .local_addr()
        .map_err(|error| fail(error.to_string()))?;

    let thread = std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| fail(error.to_string()))?;
        runtime.block_on(async move {
            let listener =
                TcpListener::from_std(listener).map_err(|error| fail(error.to_string()))?;
            accept_loop(listener, protocol, server).await
        })
    });

    Ok((local, thread))
}

/// Serve until the process ends.
pub fn serve<S, E, F>(address: &str, protocol: frame::Protocol, server: S, fail: F) -> Result<(), E>
where
    S: Serve,
    E: Send + 'static,
    F: Fn(String) -> E + Send + Clone + 'static,
{
    let (_, thread) = spawn(address, protocol, server, fail.clone())?;
    match thread.join() {
        Ok(result) => result,
        Err(_) => Err(fail("the serving thread panicked".to_owned())),
    }
}

/// How long a connection may say nothing before it is presumed gone.
///
/// A host sends a frame every tick, so a minute of silence is a half-open socket
/// or a client that never intended to speak; either way the task should not
/// outlive it.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait after a failed `accept`, so a listener that cannot accept
/// (no descriptors, say) does not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

async fn accept_loop<S: Serve>(listener: TcpListener, protocol: frame::Protocol, server: S) -> ! {
    // The plugin is shared; the connection state is not. Every host that connects
    // gets its own task and its own sequence cursor.
    let server = Arc::new(Mutex::new(server));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // A failed accept — a peer that reset between SYN and accept, a
                // descriptor limit — is about one connection, not the service.
                eprintln!("csp: accept failed: {error}");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };

        let server = Arc::clone(&server);
        tokio::spawn(async move {
            if let Err(error) = answer(stream, protocol, server).await {
                eprintln!("csp: the connection from {peer} ended: {error}");
            }
        });
    }
}

async fn answer<S: Serve>(
    mut stream: TcpStream,
    protocol: frame::Protocol,
    server: Arc<Mutex<S>>,
) -> Result<(), String> {
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    let mut connection = Connection::default();
    let mut frame = Vec::new();

    loop {
        let mut header = [0u8; HEADER_LEN];
        let read = tokio::time::timeout(IDLE_TIMEOUT, stream.read(&mut header[..1]))
            .await
            .map_err(|_| quiet())?
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        tokio::time::timeout(IDLE_TIMEOUT, stream.read_exact(&mut header[1..]))
            .await
            .map_err(|_| quiet())?
            .map_err(|error| error.to_string())?;

        // The head is parsed for its length only; the server parses it again with
        // its own kind so a bad kind becomes a `Failure` frame, not a dead socket.
        let head = frame::parse_header(&protocol, &header).map_err(|error| error.to_string())?;
        frame.clear();
        frame.extend_from_slice(&header);
        frame.resize(HEADER_LEN + head.len, 0);
        tokio::time::timeout(IDLE_TIMEOUT, stream.read_exact(&mut frame[HEADER_LEN..]))
            .await
            .map_err(|_| quiet())?
            .map_err(|error| error.to_string())?;

        let reply = server
            .lock()
            .expect("the plugin lock is not poisoned")
            .answer_frame(&mut connection, &frame);
        stream
            .write_all(&reply)
            .await
            .map_err(|error| error.to_string())?;
        stream.flush().await.map_err(|error| error.to_string())?;
    }
}

fn quiet() -> String {
    format!("no frame for {}s", IDLE_TIMEOUT.as_secs())
}
