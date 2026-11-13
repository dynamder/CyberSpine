//! An arbiter that serves the wire protocol over TCP.
//!
//! The host is the client and the arbiter listens, mirroring the body protocol.
//! Unlike a body, an arbiter may take a long time to answer (an LLM), so the
//! connection lives on a worker thread and the host only touches local queues:
//! [`Arbiter::deliver`] never blocks, and [`Arbiter::take`] drains whatever the
//! worker has already received.
//!
//! One `Tick` frame is one round trip. The host sends its clock plus every event
//! queued since the last successful exchange, and the arbiter answers with every
//! round it has to report. A slow network therefore delays a ruling, never a tick.

use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Handle, Runtime};

use crate::remote::{RemoteArbiter, Transport};
use crate::server::Server;
use crate::wire::{self, HEADER_LEN};
use crate::{Arbiter, ArbiterError, Batch, Decision, InterruptEvent, Ruling, Verdict};

/// How long one exchange may take before the arbiter is declared gone.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The client half: a blocking view of one connection.
pub struct TcpTransport {
    runtime: Runtime,
    stream: TcpStream,
    timeout: Duration,
}

impl Transport for TcpTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, ArbiterError> {
        let Self {
            runtime,
            stream,
            timeout,
        } = self;
        response.clear();

        runtime.block_on(async {
            let exchange = async {
                stream.write_all(request).await.map_err(failed)?;
                stream.flush().await.map_err(failed)?;

                let mut header = [0u8; HEADER_LEN];
                stream.read_exact(&mut header).await.map_err(failed)?;
                let head = wire::parse_header(&header).map_err(failed)?;

                let total = HEADER_LEN + head.len;
                response.resize(total, 0);
                response[..HEADER_LEN].copy_from_slice(&header);
                stream
                    .read_exact(&mut response[HEADER_LEN..])
                    .await
                    .map_err(failed)?;

                Ok(total)
            };

            tokio::time::timeout(*timeout, exchange)
                .await
                .map_err(|_| ArbiterError("the arbiter did not answer in time".into()))?
        })
    }
}

/// Connect to a remote arbiter and start its worker thread.
pub fn connect(address: &str) -> Result<RpcArbiter, ArbiterError> {
    connect_with_timeout(address, DEFAULT_TIMEOUT)
}

pub fn connect_with_timeout(address: &str, timeout: Duration) -> Result<RpcArbiter, ArbiterError> {
    if Handle::try_current().is_ok() {
        return Err(ArbiterError(
            "a remote arbiter blocks its worker; connect from a thread that is not running a \
             tokio runtime"
                .into(),
        ));
    }

    let remote = connect_remote(address, timeout)?;
    Ok(RpcArbiter::start(remote))
}

fn connect_remote(
    address: &str,
    timeout: Duration,
) -> Result<RemoteArbiter<TcpTransport>, ArbiterError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failed)?;

    let stream = runtime
        .block_on(async { tokio::time::timeout(timeout, TcpStream::connect(address)).await })
        .map_err(|_| ArbiterError(format!("connecting to `{address}` timed out").into()))?
        .map_err(|error| ArbiterError(format!("cannot connect to `{address}`: {error}").into()))?;

    stream.set_nodelay(true).map_err(failed)?;

    RemoteArbiter::handshake(TcpTransport {
        runtime,
        stream,
        timeout,
    })
}

enum Command {
    Tick {
        now: u64,
        events: Vec<InterruptEvent>,
    },
}

enum Event {
    Batch(Batch),
}

/// The host half of a remote arbiter.
///
/// Every exchange happens on the worker thread. The host side is a pair of local
/// queues, so neither the tick loop nor a verdict waits on the network.
pub struct RpcArbiter {
    commands: Sender<Command>,
    replies: Receiver<Event>,
    pending: Vec<InterruptEvent>,
    doomed: Vec<Decision>,
    awaiting: bool,
    dead: bool,
    reported: bool,
}

impl RpcArbiter {
    fn start(remote: RemoteArbiter<TcpTransport>) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, replies) = mpsc::channel();

        std::thread::spawn(move || worker(remote, command_rx, event_tx));

        Self {
            commands,
            replies,
            pending: Vec::new(),
            doomed: Vec::new(),
            awaiting: false,
            dead: false,
            reported: false,
        }
    }

    fn doom(&mut self, event: InterruptEvent) {
        self.doomed.push(Decision {
            event: event.event,
            ruling: Ruling::finished(Vec::new(), Verdict::Stop),
        });
    }
}

impl core::fmt::Debug for RpcArbiter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RpcArbiter")
            .field("dead", &self.dead)
            .finish_non_exhaustive()
    }
}

impl Arbiter for RpcArbiter {
    fn deliver(&mut self, event: InterruptEvent) {
        if self.dead {
            self.doom(event);
            return;
        }
        self.pending.push(event);
    }

    fn take(&mut self, now: u64) -> Batch {
        let mut decisions = core::mem::take(&mut self.doomed);
        let mut failure = None;

        loop {
            match self.replies.try_recv() {
                Ok(Event::Batch(batch)) => {
                    self.awaiting = false;
                    decisions.extend(batch.decisions);
                    if batch.failure.is_some() {
                        failure = failure.or(batch.failure);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.dead = true;
                    break;
                }
            }
        }

        if failure.is_some() {
            self.dead = true;
        }
        if self.dead {
            if !self.reported {
                self.reported = true;
                failure = failure.or_else(|| Some(ArbiterError("the arbiter stopped".into())));
            }
            return Batch { decisions, failure };
        }

        if !self.awaiting {
            let events = core::mem::take(&mut self.pending);
            let sent = self.commands.send(Command::Tick { now, events }).is_ok();
            if sent {
                self.awaiting = true;
            } else {
                self.dead = true;
                self.reported = true;
                failure = Some(ArbiterError("the arbiter worker stopped".into()));
            }
        }

        Batch { decisions, failure }
    }
}

fn worker(
    mut remote: RemoteArbiter<TcpTransport>,
    commands: Receiver<Command>,
    replies: Sender<Event>,
) {
    while let Ok(command) = commands.recv() {
        match command {
            Command::Tick { now, events } => {
                for event in events {
                    remote.deliver(event);
                }
                let batch = remote.take(now);
                if replies.send(Event::Batch(batch)).is_err() {
                    break;
                }
            }
        }
    }
}

/// Serve `arbiter` on `address`.
///
/// The listener is bound before the thread starts, so a bad address or a taken
/// port comes back as an error instead of disappearing into a background thread.
pub fn spawn<A: Arbiter + Send + 'static>(
    address: &str,
    arbiter: A,
    build_id: String,
) -> Result<(SocketAddr, JoinHandle<Result<(), ArbiterError>>), ArbiterError> {
    let server = Server::new(arbiter, build_id);

    let listener = std::net::TcpListener::bind(address)
        .map_err(|error| ArbiterError(format!("cannot listen on `{address}`: {error}").into()))?;
    listener.set_nonblocking(true).map_err(failed)?;
    let local = listener.local_addr().map_err(failed)?;

    let thread = std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failed)?;
        runtime.block_on(async move {
            let listener = TcpListener::from_std(listener).map_err(failed)?;
            accept_loop(listener, server).await
        })
    });

    Ok((local, thread))
}

/// Serve until the process ends.
pub fn serve<A: Arbiter + Send + 'static>(
    address: &str,
    arbiter: A,
    build_id: String,
) -> Result<(), ArbiterError> {
    let (_, thread) = spawn(address, arbiter, build_id)?;
    match thread.join() {
        Ok(result) => result,
        Err(_) => Err(ArbiterError("the serving thread panicked".into())),
    }
}

async fn accept_loop<A: Arbiter>(
    listener: TcpListener,
    mut server: Server<A>,
) -> Result<(), ArbiterError> {
    loop {
        let (stream, peer) = listener.accept().await.map_err(failed)?;
        if let Err(error) = answer(stream, &mut server).await {
            eprintln!("csp: the connection from {peer} ended: {error}");
        }
    }
}

async fn answer<A: Arbiter>(
    mut stream: TcpStream,
    server: &mut Server<A>,
) -> Result<(), ArbiterError> {
    stream.set_nodelay(true).map_err(failed)?;
    let mut frame = Vec::new();

    loop {
        let mut header = [0u8; HEADER_LEN];
        let read = stream.read(&mut header[..1]).await.map_err(failed)?;
        if read == 0 {
            return Ok(());
        }
        stream.read_exact(&mut header[1..]).await.map_err(failed)?;

        let head = wire::parse_header(&header).map_err(failed)?;
        frame.clear();
        frame.extend_from_slice(&header);
        frame.resize(HEADER_LEN + head.len, 0);
        stream
            .read_exact(&mut frame[HEADER_LEN..])
            .await
            .map_err(failed)?;

        let reply = server.answer(&frame);
        stream.write_all(reply).await.map_err(failed)?;
        stream.flush().await.map_err(failed)?;
    }
}

fn failed(error: impl core::fmt::Display) -> ArbiterError {
    ArbiterError(error.to_string().into_boxed_str())
}
