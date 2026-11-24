//! End to end through a real shared library.
//!
//! The library is a workspace member, so a build or a workspace test run has
//! already produced it; the test only has to find it.

#![cfg(feature = "dylib")]

use std::path::PathBuf;

use cyberspine_arbiter::{
    Arbiter, Completion, EventId, InstanceRef, InterruptEvent, Verdict, dylib,
};
use cyberspine_testkit as testkit;

fn fixture() -> PathBuf {
    testkit::fixture_library("cyberspine-arbiter-fixture")
}

fn event() -> InterruptEvent {
    InterruptEvent {
        event: EventId::new(7),
        instance: InstanceRef::new(0),
        pc: 3,
        tick: 12,
        format: "x={f1}".into(),
        reason: "x=1.5".into(),
        action: "actions/idle.csp".into(),
        instances: Vec::new(),
        actions: Vec::new(),
        image_hash: [0; 32],
    }
}

#[test]
fn two_rounds_cross_the_library_boundary() {
    let mut arbiter = dylib::open(&fixture()).expect("the fixture loads");
    assert_eq!(arbiter.build_id(), "cyberspine-arbiter-fixture/0.1.0");

    arbiter.deliver(event());

    let first = arbiter.take(0);
    assert!(first.failure.is_none(), "{:?}", first.failure);
    assert_eq!(first.decisions.len(), 1);
    assert_eq!(first.decisions[0].event, EventId::new(7));
    assert!(matches!(
        first.decisions[0].ruling.completion,
        Completion::Unfinished
    ));

    let second = arbiter.take(1);
    assert!(second.failure.is_none(), "{:?}", second.failure);
    assert_eq!(second.decisions.len(), 1);
    assert!(matches!(
        second.decisions[0].ruling.completion,
        Completion::Finished(Verdict::Continue)
    ));
}

#[test]
fn a_missing_library_names_the_path_it_could_not_load() {
    let missing = std::env::temp_dir().join("cyberspine-absent-arbiter.dll");

    let error = dylib::open(&missing).expect_err("the file does not exist");

    assert!(
        error.to_string().contains("cyberspine-absent-arbiter"),
        "{error}"
    );
}
