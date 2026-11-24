//! The arbiter side of the protocol: one request frame in, one reply frame out.
//!
//! Both ways of shipping an arbiter — the dynamic-library shim and the TCP
//! server — are thin wrappers around [`Server`], so an arbiter behaves the same
//! whichever way it is packaged.

use crate::wire::{self, Failure, Hello, Kind, Rulings, Tick, Welcome, WireCause};
use crate::{Action, Arbiter, Decision};

/// An arbiter plus the buffer its replies are built in.
pub struct Server<A: Arbiter> {
    arbiter: A,
    build_id: String,
    reply: Vec<u8>,
}

impl<A: Arbiter> Server<A> {
    pub fn new(arbiter: A, build_id: String) -> Self {
        Self {
            arbiter,
            build_id,
            reply: Vec::new(),
        }
    }

    pub fn arbiter(&self) -> &A {
        &self.arbiter
    }

    pub fn arbiter_mut(&mut self) -> &mut A {
        &mut self.arbiter
    }

    /// Answer exactly one request frame.
    ///
    /// The returned slice is the reply frame, borrowed from the server's reusable
    /// buffer: copy it out before the next call. Every refusal comes back as a
    /// `Failure` frame, so a transport never has to invent a message.
    pub fn answer(&mut self, connection: &mut Connection, request: &[u8]) -> &[u8] {
        let refusal = match wire::split(request) {
            Ok((kind, payload)) => self.dispatch(connection, kind, payload),
            Err(error) => Err(protocol(error.to_string())),
        };

        match refusal {
            Ok(()) => &self.reply,
            Err(failure) => {
                wire::encode(Kind::Error, &failure, &mut self.reply)
                    .expect("a Failure is always encodable");
                &self.reply
            }
        }
    }

    fn dispatch(
        &mut self,
        connection: &mut Connection,
        kind: Kind,
        payload: &[u8],
    ) -> Result<(), Failure> {
        match kind {
            Kind::Hello => {
                let hello: Hello = wire::decode(payload).map_err(protocol_error)?;
                if hello.version != wire::VERSION {
                    return Err(protocol(format!(
                        "the host speaks protocol {}, this arbiter speaks {}",
                        hello.version,
                        wire::VERSION
                    )));
                }
                let welcome = Welcome {
                    version: wire::VERSION,
                    build_id: self.build_id.clone(),
                };
                wire::encode(Kind::Hello, &welcome, &mut self.reply).map_err(protocol_error)
            }

            Kind::Tick => {
                let tick: Tick = wire::decode(payload).map_err(protocol_error)?;
                if connection.is_repeated(tick.seq) {
                    return Ok(());
                }
                for event in tick.events {
                    self.arbiter.deliver(event);
                }
                let batch = self.arbiter.take(tick.now);
                let rulings = Rulings {
                    decisions: batch.decisions,
                    failure: batch.failure.map(|error| {
                        let cause = error.cause();
                        WireCause {
                            code: cause.code.as_str().to_owned(),
                            message: cause.message.clone(),
                            help: cause.help.clone(),
                        }
                    }),
                };
                if let Some(overlong) = first_overlong(&rulings.decisions) {
                    return Err(protocol(overlong));
                }
                wire::encode(Kind::Tick, &rulings, &mut self.reply).map_err(protocol_error)
            }

            Kind::Error => Err(protocol("a Failure is a reply, not a request".to_owned())),
        }
    }
}

/// One transport's session with this server.
///
/// Deduplication belongs here, not on the [`Server`]: a host's `seq` restarts at
/// 0 every time it connects, so a cursor kept by the server would hand a fresh
/// host the reply the previous host was waiting for.
#[derive(Debug, Default)]
pub struct Connection {
    last_seq: Option<u64>,
}

impl Connection {
    /// Whether this is the frame the previous reply was built from.
    fn is_repeated(&mut self, seq: u64) -> bool {
        if self.last_seq == Some(seq) {
            return true;
        }
        self.last_seq = Some(seq);
        false
    }
}

fn protocol(message: String) -> Failure {
    Failure {
        code: wire::code::PROTOCOL,
        message,
    }
}

fn protocol_error(error: wire::WireError) -> Failure {
    protocol(error.to_string())
}

/// The first ruling text above the per-field cap, if any.
fn first_overlong(decisions: &[Decision]) -> Option<String> {
    let over = |text: &str| text.len() > wire::MAX_TEXT_LEN;
    for decision in decisions {
        let ruling = &decision.ruling;
        let a_text_is_long = over(&ruling.source)
            || over(&ruling.note)
            || ruling.actions.iter().any(|action| match action {
                Action::Load { action } => over(action),
                _ => false,
            });
        if a_text_is_long {
            return Some(format!(
                "event {}: a ruling text is above the {}-byte cap",
                decision.event,
                wire::MAX_TEXT_LEN
            ));
        }
    }
    None
}
