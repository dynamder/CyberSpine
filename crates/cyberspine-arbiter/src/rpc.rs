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

use cyberspine_frame as frame;

pub use cyberspine_plugin::rpc::TcpTransport;

use crate::remote::{RemoteArbiter, Transport};
use crate::server::Server;
use crate::wire;
use crate::{Arbiter, ArbiterError, Batch, Decision, InterruptEvent, Ruling, Verdict};

/// How long one exchange may take before the arbiter is declared gone.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The head this crate's frames carry, for the shared socket loop.
const PROTOCOL: frame::Protocol = frame::Protocol {
    magic: wire::MAGIC,
    version: wire::VERSION,
};

impl Transport for TcpTransport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, ArbiterError> {
        self.exchange_frame(PROTOCOL, request, response)
            .map_err(ArbiterError::new)
    }
}

/// Connect to a remote arbiter and start its worker thread.
pub fn connect(address: &str) -> Result<RpcArbiter, ArbiterError> {
    connect_with_timeout(address, DEFAULT_TIMEOUT)
}

/// Connect with an exchange deadline other than [`DEFAULT_TIMEOUT`].
pub fn connect_with_timeout(address: &str, timeout: Duration) -> Result<RpcArbiter, ArbiterError> {
    let remote = connect_remote(address, timeout)?;
    Ok(RpcArbiter::start(remote))
}

fn connect_remote(
    address: &str,
    timeout: Duration,
) -> Result<RemoteArbiter<TcpTransport>, ArbiterError> {
    let transport = TcpTransport::connect(
        address,
        timeout,
        "arbiter",
        "a remote arbiter blocks its worker; connect from a thread that is not running a tokio \
         runtime",
    )
    .map_err(ArbiterError::new)?;
    RemoteArbiter::handshake(transport)
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
                failure = failure.or_else(|| Some(ArbiterError::new("the arbiter stopped")));
            }
            return Batch {
                decisions,
                failure,
                ..Batch::default()
            };
        }

        if !self.awaiting {
            let events = core::mem::take(&mut self.pending);
            let sent = self.commands.send(Command::Tick { now, events }).is_ok();
            if sent {
                self.awaiting = true;
            } else {
                self.dead = true;
                self.reported = true;
                failure = Some(ArbiterError::new("the arbiter worker stopped"));
            }
        }

        Batch {
            decisions,
            failure,
            ..Batch::default()
        }
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
    cyberspine_plugin::rpc::spawn(address, PROTOCOL, server, ArbiterError::new)
}

/// Serve until the process ends.
pub fn serve<A: Arbiter + Send + 'static>(
    address: &str,
    arbiter: A,
    build_id: String,
) -> Result<(), ArbiterError> {
    let server = Server::new(arbiter, build_id);
    cyberspine_plugin::rpc::serve(address, PROTOCOL, server, ArbiterError::new)
}
