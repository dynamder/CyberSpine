//! End to end over a real socket.
//!
//! The body is the same fixture a dynamic library and a linked host use, so this
//! is the third packaging of one `impl Backend`: if the traces differ, the
//! protocol or a transport is wrong, not the body.

#![cfg(feature = "rpc")]

#[path = "../../cyberspine-backend-fixture/src/body.rs"]
mod body;

use cyberspine_backend::rpc;
use cyberspine_testkit as testkit;
use cyberspine_vm::{Backend, Host};

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

#[test]
fn a_port_with_no_body_is_a_connect_error() {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = probe.local_addr().expect("the bound address");
    drop(probe);

    let error =
        rpc::connect_with_timeout(&address.to_string(), std::time::Duration::from_millis(250))
            .expect_err("nobody is listening there");

    assert!(
        error.to_string().contains(&address.to_string()),
        "the failure names the body it could not reach: {error}"
    );
}
