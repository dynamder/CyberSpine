//! A transport retry must not execute a request twice.
//!
//! The ABI reports `RESPONSE_TOO_SMALL` *after* running the request, so the
//! dylib transport re-sends the same bytes with a bigger buffer. The server has
//! to turn that retry into a lookup, or a `Step` would advance the body twice.

use cyberspine_backend::wire::{self, Kind, Sample, SampleReply, Step};
use cyberspine_backend::{Connection, Server};
use cyberspine_backend_fixture::{Fixture, RIG_JSON};

#[test]
fn a_retried_step_does_not_advance_the_body_twice() {
    let mut server = Server::new(Fixture::new(), RIG_JSON.to_owned(), "test".to_owned())
        .expect("the fixture manifest describes the fixture body");

    let mut connection = Connection::default();
    let mut request = Vec::new();
    wire::encode(Kind::Step, &Step { seq: 7, dt: 0.1 }, &mut request).expect("the frame encodes");

    server.answer(&mut connection, &request);
    server.answer(&mut connection, &request);

    let mut read = Vec::new();
    wire::encode(Kind::Sample, &Sample { seq: 8 }, &mut read).expect("the frame encodes");
    let reply = server.answer(&mut connection, &read).to_vec();
    let (_, payload) = wire::split(&reply).expect("the reply is one frame");
    let sample: SampleReply = wire::decode(payload).expect("the reply is a sample");

    assert_eq!(sample.i, vec![1], "one step was applied, not two");
}

/// The cursor is the connection's, so a second client's `seq` 7 is a request,
/// not a retry of the first client's.
#[test]
fn a_second_connection_is_not_a_retry() {
    let mut server = Server::new(Fixture::new(), RIG_JSON.to_owned(), "test".to_owned())
        .expect("the fixture manifest describes the fixture body");
    let mut first = Connection::default();
    let mut second = Connection::default();

    let mut request = Vec::new();
    wire::encode(Kind::Step, &Step { seq: 7, dt: 0.1 }, &mut request).expect("the frame encodes");

    server.answer(&mut first, &request);
    server.answer(&mut second, &request);

    let mut read = Vec::new();
    wire::encode(Kind::Sample, &Sample { seq: 8 }, &mut read).expect("the frame encodes");
    let reply = server.answer(&mut second, &read).to_vec();
    let (_, payload) = wire::split(&reply).expect("the reply is one frame");
    let sample: SampleReply = wire::decode(payload).expect("the reply is a sample");

    assert_eq!(
        sample.i,
        vec![2],
        "both connections' steps ran; only a same-connection retry is a lookup"
    );
}
