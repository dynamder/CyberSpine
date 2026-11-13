//! The host side of an arbiter that lives behind a transport.
//!
//! A [`RemoteArbiter`] is an ordinary [`Arbiter`]: the handshake pins the
//! protocol version once, and every tick then becomes one request frame and one
//! reply frame. The bytes are [`wire`]'s, so the same type drives a
//! dynamic library and a TCP connection.
//!
//! # Delivery is local
//!
//! [`Arbiter::deliver`] only queues the event; the exchange happens on
//! [`Arbiter::take`], which carries every queued event with the host's clock in
//! one frame. That keeps delivery non-blocking even through a synchronous
//! transport, and it means a tick costs at most one round trip.
//!
//! # Failure is terminal
//!
//! A transport failure is not retried. The arbiter is unreachable, so the first
//! error is latched: later `take` calls report no failure again, and later events
//! are answered with `Finished(Stop)` on the spot instead of being queued for a
//! peer that will never answer.

use crate::wire::{self, Kind, Rulings, Tick, Welcome};
use crate::{Arbiter, ArbiterError, Batch, Decision, InterruptEvent, Ruling, Verdict};

/// One request frame in, one reply frame out.
///
/// The implementation owns the buffer: after a successful call `response[..n]`
/// is exactly one frame, and `n` is the return value.
pub trait Transport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, ArbiterError>;
}

/// An arbiter reached over a [`Transport`].
pub struct RemoteArbiter<T: Transport> {
    transport: T,
    request: Vec<u8>,
    response: Vec<u8>,
    build_id: Box<str>,
    pending: Vec<InterruptEvent>,
    doomed: Vec<Decision>,
    terminal: Option<Box<str>>,
    seq: u64,
}

impl<T: Transport> core::fmt::Debug for RemoteArbiter<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteArbiter")
            .field("build_id", &self.build_id)
            .finish_non_exhaustive()
    }
}

impl<T: Transport> RemoteArbiter<T> {
    /// Introduce the host, then pin the protocol version the arbiter speaks.
    pub fn handshake(mut transport: T) -> Result<Self, ArbiterError> {
        let hello = wire::Hello {
            version: wire::VERSION,
            client: format!("cyberspine-arbiter/{}", env!("CARGO_PKG_VERSION")),
        };

        let mut request = Vec::new();
        let mut response = Vec::new();
        wire::encode(Kind::Hello, &hello, &mut request).map_err(failed)?;
        let len = transport.exchange(&request, &mut response)?;
        let (kind, payload) = wire::split(&response[..len]).map_err(failed)?;
        if kind != Kind::Hello {
            return Err(ArbiterError(
                format!("the arbiter answered a Hello with {kind:?}").into_boxed_str(),
            ));
        }

        let welcome: Welcome = wire::decode(payload).map_err(failed)?;
        if welcome.version != wire::VERSION {
            return Err(ArbiterError(
                format!(
                    "the arbiter speaks protocol {}, this host speaks {}",
                    welcome.version,
                    wire::VERSION
                )
                .into_boxed_str(),
            ));
        }

        Ok(Self {
            transport,
            request,
            response,
            build_id: welcome.build_id.into_boxed_str(),
            pending: Vec::new(),
            doomed: Vec::new(),
            terminal: None,
            seq: 0,
        })
    }

    /// What the arbiter says built it. Opaque to the host; worth recording next
    /// to a run's decision log.
    pub fn build_id(&self) -> &str {
        &self.build_id
    }

    fn exchange(&mut self, now: u64, events: Vec<InterruptEvent>) -> Result<Rulings, ArbiterError> {
        let tick = Tick {
            seq: self.seq,
            now,
            events,
        };
        self.seq += 1;
        wire::encode(Kind::Tick, &tick, &mut self.request).map_err(failed)?;
        let len = self.transport.exchange(&self.request, &mut self.response)?;
        let (kind, payload) = wire::split(&self.response[..len]).map_err(failed)?;

        match kind {
            Kind::Tick => wire::decode(payload).map_err(failed),
            Kind::Error => {
                let failure: wire::Failure = wire::decode(payload).map_err(failed)?;
                Err(ArbiterError(failure.message.into_boxed_str()))
            }
            other => Err(ArbiterError(
                format!("the arbiter answered a Tick with {other:?}").into_boxed_str(),
            )),
        }
    }
}

impl<T: Transport> Arbiter for RemoteArbiter<T> {
    fn deliver(&mut self, event: InterruptEvent) {
        if self.terminal.is_some() {
            self.doomed.push(Decision {
                event: event.event,
                ruling: Ruling::finished(Vec::new(), Verdict::Stop),
            });
            return;
        }
        self.pending.push(event);
    }

    fn take(&mut self, now: u64) -> Batch {
        let mut decisions = core::mem::take(&mut self.doomed);
        if self.terminal.is_some() {
            return Batch {
                decisions,
                failure: None,
            };
        }

        let events = core::mem::take(&mut self.pending);
        match self.exchange(now, events) {
            Ok(rulings) => {
                decisions.extend(rulings.decisions);
                Batch {
                    decisions,
                    failure: rulings
                        .failure
                        .map(|text| ArbiterError(text.into_boxed_str())),
                }
            }
            Err(error) => {
                self.terminal = Some(error.0.clone());
                Batch {
                    decisions,
                    failure: Some(error),
                }
            }
        }
    }
}

fn failed(error: wire::WireError) -> ArbiterError {
    ArbiterError(error.to_string().into_boxed_str())
}
