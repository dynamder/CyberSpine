//! The server's per-connection state, without a socket.
//!
//! `seq` deduplication exists so a transport buffer retry is a lookup instead of
//! a second delivery — a fact about one connection. These tests pin that a fresh
//! connection starts with a fresh cursor.

use cyberspine_arbiter::wire::{self, Hello, Kind, Tick};
use cyberspine_arbiter::{
    Arbiter, Batch, Connection, Decision, EventId, InstanceRef, InterruptEvent, Ruling, Server,
    Verdict,
};

/// Counts what it was asked to deliver and answers one `Finished` per event.
#[derive(Default)]
struct Counter {
    delivered: Vec<EventId>,
}

impl Arbiter for Counter {
    fn deliver(&mut self, event: InterruptEvent) {
        self.delivered.push(event.event);
    }

    fn take(&mut self, _now: u64) -> Batch {
        Batch::from(
            self.delivered
                .drain(..)
                .map(|event| Decision {
                    event,
                    ruling: Ruling::finished(Vec::new(), Verdict::Continue),
                })
                .collect::<Vec<_>>(),
        )
    }
}

fn event(seq: u64) -> InterruptEvent {
    InterruptEvent {
        event: EventId::new(seq),
        instance: InstanceRef::new(0),
        pc: 0,
        tick: 0,
        format: "x".to_owned().into_boxed_str(),
        reason: "x".to_owned().into_boxed_str(),
        action: "a.csp".to_owned().into_boxed_str(),
        instances: Vec::new(),
        actions: Vec::new(),
        image_hash: [0; 32],
    }
}

fn tick(seq: u64) -> Vec<u8> {
    let mut frame = Vec::new();
    wire::encode(
        Kind::Tick,
        &Tick {
            seq,
            now: 0,
            events: vec![event(seq)],
        },
        &mut frame,
    )
    .expect("the tick encodes");
    frame
}

fn hello() -> Vec<u8> {
    let mut frame = Vec::new();
    wire::encode(
        Kind::Hello,
        &Hello {
            version: wire::VERSION,
            client: "test".to_owned(),
        },
        &mut frame,
    )
    .expect("the hello encodes");
    frame
}

fn kind(frame: &[u8]) -> Kind {
    let (kind, _) = wire::split(frame).expect("a reply is one frame");
    kind
}

#[test]
fn a_new_connection_does_not_answer_with_the_previous_connections_reply() {
    let mut server = Server::new(Counter::default(), "test".to_owned());
    let mut first = Connection::default();
    let mut second = Connection::default();

    // The first host ticks once and goes away. Its reply is what the server's
    // buffer holds when the second host introduces itself.
    assert_eq!(kind(server.answer(&mut first, &tick(0))), Kind::Tick);
    assert_eq!(kind(server.answer(&mut second, &hello())), Kind::Hello);

    // The second host's first tick reuses `seq` 0. It must be answered, not
    // served from the previous connection's buffer.
    let reply = server.answer(&mut second, &tick(0));
    assert_eq!(
        kind(reply),
        Kind::Tick,
        "a fresh connection was handed the previous connection's reply"
    );

    let (_, payload) = wire::split(reply).expect("a reply is one frame");
    let rulings: wire::Rulings = wire::decode(payload).expect("the reply is rulings");
    assert_eq!(
        rulings.decisions.len(),
        1,
        "the second host's event was delivered and answered"
    );
}

#[test]
fn a_retry_on_one_connection_is_still_a_lookup() {
    let mut server = Server::new(Counter::default(), "test".to_owned());
    let mut connection = Connection::default();

    let first = server.answer(&mut connection, &tick(0)).to_vec();
    let retry = server.answer(&mut connection, &tick(0)).to_vec();

    assert_eq!(first, retry, "the retry is answered from the buffer");
    let (_, payload) = wire::split(&retry).expect("a reply is one frame");
    let rulings: wire::Rulings = wire::decode(payload).expect("the reply is rulings");
    assert_eq!(
        rulings.decisions.len(),
        1,
        "the retried frame was not delivered a second time"
    );
}
