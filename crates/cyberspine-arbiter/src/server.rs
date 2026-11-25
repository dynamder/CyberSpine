//! The arbiter side of the protocol: one request frame in, one reply frame out.
//!
//! Both ways of shipping an arbiter — the dynamic-library shim and the TCP
//! server — are thin wrappers around [`Server`], so an arbiter behaves the same
//! whichever way it is packaged.

pub use cyberspine_plugin::server::{Connection, Serve, panic_message};

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
                wire::encode(Kind::Tick, &rulings, &mut self.reply).map_err(protocol_error)?;
                connection.commit(tick.seq);
                Ok(())
            }

            Kind::Error => Err(protocol("a Failure is a reply, not a request".to_owned())),
        }
    }
}

impl<A: Arbiter + Send + 'static> Serve for Server<A> {
    fn reply(&mut self, connection: &mut Connection, request: &[u8]) -> Vec<u8> {
        self.answer(connection, request).to_vec()
    }

    fn panic_reply(&mut self, message: &str) -> Vec<u8> {
        let failure = Failure::new(wire::code::PANIC, message);
        let mut reply = Vec::new();
        wire::encode(Kind::Error, &failure, &mut reply)
            .expect("a capped panic failure is encodable");
        reply
    }
}

fn protocol(message: String) -> Failure {
    Failure::new(wire::code::PROTOCOL, message)
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

#[cfg(test)]
mod tests {
    use super::{Connection, Serve, Server};
    use crate::wire::{self, Failure, Kind, Tick};

    struct Exploding;

    impl crate::Arbiter for Exploding {
        fn deliver(&mut self, _event: crate::InterruptEvent) {}

        fn take(&mut self, _now: u64) -> crate::Batch {
            panic!("the arbiter exploded")
        }
    }

    /// The body side has a TCP test for this; the arbiter goes through the same
    /// shared guard, and this pins it without a socket.
    #[test]
    fn a_panicking_arbiter_still_answers_one_frame() {
        let mut server = Server::new(Exploding, "build".to_owned());
        let mut connection = Connection::default();
        let mut request = Vec::new();
        wire::encode(
            Kind::Tick,
            &Tick {
                seq: 1,
                now: 0,
                events: Vec::new(),
            },
            &mut request,
        )
        .expect("the tick encodes");

        let reply = server.answer_frame(&mut connection, &request);

        let (kind, payload) = wire::split(&reply).expect("the reply is one frame");
        assert_eq!(kind, Kind::Error);
        let failure: Failure = wire::decode(payload).expect("the failure decodes");
        assert_eq!(failure.code, wire::code::PANIC);
        assert!(
            failure.message().contains("the arbiter exploded"),
            "{}",
            failure.message()
        );
    }
}
