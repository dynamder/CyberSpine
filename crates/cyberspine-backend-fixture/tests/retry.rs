//! A transport retry must not execute a request twice.
//!
//! The ABI reports `RESPONSE_TOO_SMALL` *after* running the request, so the
//! dylib transport re-sends the same bytes with a bigger buffer. The server has
//! to turn that retry into a lookup, or a `Step` would advance the body twice.

use cyberspine_backend::Server;
use cyberspine_backend::wire::{self, Kind, Sample, SampleReply, Step};
use cyberspine_backend_fixture::{Fixture, RIG_JSON};

#[test]
fn a_retried_step_does_not_advance_the_body_twice() {
    let mut server = Server::new(Fixture::new(), RIG_JSON.to_owned(), "test".to_owned())
        .expect("the fixture manifest describes the fixture body");

    let mut request = Vec::new();
    wire::encode(Kind::Step, &Step { seq: 7, dt: 0.1 }, &mut request).expect("the frame encodes");

    server.answer(&request);
    server.answer(&request);

    let mut read = Vec::new();
    wire::encode(Kind::Sample, &Sample { seq: 8 }, &mut read).expect("the frame encodes");
    let reply = server.answer(&read).to_vec();
    let (_, payload) = wire::split(&reply).expect("the reply is one frame");
    let sample: SampleReply = wire::decode(payload).expect("the reply is a sample");

    assert_eq!(sample.i, vec![1], "one step was applied, not two");
}
