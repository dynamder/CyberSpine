//! End to end over a real socket.
//!
//! The body is the same fixture a dynamic library and a linked host use, so this
//! is the third packaging of one `impl Backend`: if the traces differ, the
//! protocol or a transport is wrong, not the body.

#![cfg(feature = "rpc")]

#[path = "../../cyberspine-backend-fixture/src/body.rs"]
mod body;

use std::time::Duration;

use cyberspine_backend::rpc;
use cyberspine_backend::wire::{self, Failure, Hello, Kind, Sample, SampleReply};
use cyberspine_frame as frame;
use cyberspine_rig::BoundFingerprint;
use cyberspine_testkit as testkit;
use cyberspine_vm::{Backend, BackendError, Host, Shape, Snapshot};

/// Four ticks of a program that publishes both muscles every tick.
fn trace<B: Backend>(host: Host<B>) -> Vec<Vec<Option<f32>>> {
    let image = testkit::verified(testkit::action(
        vec![
            testkit::sense(1, 2),
            testkit::excite(1, 1),
            testkit::fmovi(2, 0.5),
            testkit::excite(0, 2),
            testkit::flush(),
            testkit::yield_(),
            testkit::jump(-7),
        ],
        &body::rig(),
    ));

    let (mut session, _) = host.load(image).expect("the image fits the body");
    session.set_tracing(true);
    for _ in 0..4 {
        session.tick().expect("the body answers");
    }
    session.trace().map(|tick| tick.commands.to_vec()).collect()
}

#[test]
fn a_remote_body_produces_the_same_trace_as_the_linked_body() {
    let (address, _server) = rpc::spawn(
        "127.0.0.1:0",
        body::Fixture::new(),
        body::RIG_JSON.to_owned(),
        "test".to_owned(),
    )
    .expect("the server binds");

    let remote = rpc::connect(&address.to_string()).expect("the body answers");
    assert_eq!(remote.build_id(), "test");

    assert_eq!(
        trace(Host::new(remote)),
        trace(Host::new(body::Fixture::new())),
        "one body, two packagings, one trace"
    );
}

#[test]
fn a_manifest_that_disagrees_with_the_body_never_gets_to_listen() {
    let swapped = body::RIG_JSON.replace("\"m0\", \"m1\"", "\"m1\", \"m0\"");
    assert_ne!(swapped, body::RIG_JSON);

    let error = rpc::spawn(
        "127.0.0.1:0",
        body::Fixture::new(),
        swapped,
        "test".to_owned(),
    )
    .expect_err("the server refuses a manifest that is not its body");

    assert!(error.to_string().contains("manifest"), "{error}");
}

/// A connection that says nothing must not own the body.
#[test]
fn a_silent_connection_does_not_block_a_real_host() {
    let (address, _server) = rpc::spawn(
        "127.0.0.1:0",
        body::Fixture::new(),
        body::RIG_JSON.to_owned(),
        "test".to_owned(),
    )
    .expect("the server binds");

    // A host that connects and never speaks, or a socket that was partitioned.
    let silent = std::net::TcpStream::connect(address).expect("the raw connection lands");
    std::thread::sleep(std::time::Duration::from_millis(50));

    let remote = rpc::connect_with_timeout(&address.to_string(), std::time::Duration::from_secs(2))
        .expect("the body answers while another connection is silent");
    assert_eq!(
        trace(Host::new(remote)),
        trace(Host::new(body::Fixture::new()))
    );

    drop(silent);
}

#[test]
fn a_port_with_no_body_is_a_connect_error() {
    let address = "127.0.0.1:1";

    let error = rpc::connect_with_timeout(address, std::time::Duration::from_secs(2))
        .expect_err("nobody is listening there");

    assert!(
        error.to_string().contains(address),
        "the failure names the body it could not reach: {error}"
    );
}

/// A body that panics on its first sample and is ordinary after that.
struct PanicOnce {
    inner: body::Fixture,
    panicked: bool,
}

impl PanicOnce {
    fn new(inner: body::Fixture) -> Self {
        Self {
            inner,
            panicked: false,
        }
    }
}

impl Backend for PanicOnce {
    fn fingerprint(&self) -> BoundFingerprint {
        self.inner.fingerprint()
    }

    fn shape(&self) -> Shape {
        self.inner.shape()
    }

    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        if !self.panicked {
            self.panicked = true;
            panic!("boom");
        }
        self.inner.sample(out)
    }

    fn commit(&mut self, commands: &[Option<f32>], dt: f32) -> Result<(), BackendError> {
        self.inner.commit(commands, dt)
    }

    fn step(&mut self, dt: f32) -> Result<(), BackendError> {
        self.inner.step(dt)
    }
}

fn exchange(
    transport: &mut rpc::TcpTransport,
    protocol: frame::Protocol,
    request: &[u8],
) -> (Kind, Vec<u8>) {
    let mut response = Vec::new();
    let len = transport
        .exchange_frame(protocol, request, &mut response)
        .expect("the body answers");
    let (kind, payload) = wire::split(&response[..len]).expect("the reply is one frame");
    (kind, payload.to_vec())
}

#[test]
fn a_panicking_body_is_a_failure_and_the_connection_survives() {
    let (address, _server) = rpc::spawn(
        "127.0.0.1:0",
        PanicOnce::new(body::Fixture::new()),
        body::RIG_JSON.to_owned(),
        "test".to_owned(),
    )
    .expect("the server binds");

    let protocol = frame::Protocol {
        magic: wire::MAGIC,
        version: wire::VERSION,
    };
    let mut transport = rpc::TcpTransport::connect(
        &address.to_string(),
        Duration::from_secs(2),
        "body",
        "the test is not inside a runtime",
    )
    .expect("the transport connects");

    let mut request = Vec::new();
    wire::encode(
        Kind::Hello,
        &Hello {
            version: wire::VERSION,
            client: "test".to_owned(),
        },
        &mut request,
    )
    .expect("the hello encodes");
    let (kind, _) = exchange(&mut transport, protocol, &request);
    assert_eq!(kind, Kind::Hello);

    wire::encode(Kind::Sample, &Sample { seq: 1 }, &mut request).expect("the sample encodes");
    let (kind, payload) = exchange(&mut transport, protocol, &request);
    assert_eq!(kind, Kind::Error, "a panic is a failure frame");
    let failure: Failure = wire::decode(&payload).expect("the failure decodes");
    assert_eq!(failure.code, wire::code::PANIC);
    assert!(failure.message().contains("boom"), "{}", failure.message());

    wire::encode(Kind::Sample, &Sample { seq: 2 }, &mut request).expect("the sample encodes");
    let (kind, payload) = exchange(&mut transport, protocol, &request);
    assert_eq!(kind, Kind::Sample, "the connection survived the panic");
    let reply: SampleReply = wire::decode(&payload).expect("the sample reply decodes");
    assert_eq!(reply.seq, 2);
    assert_eq!(reply.f.len(), 1);
    assert_eq!(reply.i.len(), 1);
    assert_eq!(reply.muscles.len(), 2);
}
