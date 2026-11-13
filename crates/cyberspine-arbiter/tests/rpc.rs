//! End to end over a real socket.
//!
//! The host half never blocks: a reply can arrive one or more `take` calls after
//! the tick that sent it, which is exactly what an arbiter that thinks for a
//! while needs.

#![cfg(feature = "rpc")]

use std::time::Duration;

use cyberspine_arbiter::{
    Arbiter, Batch, Completion, EventId, InterruptEvent, RpcArbiter, Verdict, rpc,
};
use cyberspine_arbiter_fixture::Scripted;

fn event() -> InterruptEvent {
    InterruptEvent {
        event: EventId::new(9),
        pc: 1,
        tick: 3,
        format: "x={f1}".into(),
        reason: "x=0.5".into(),
        action: "actions/idle.csp".into(),
        instances: Vec::new(),
        image_hash: [0; 32],
    }
}

/// Poll until the arbiter reports something, or give up.
fn next_batch(arbiter: &mut RpcArbiter, now: u64) -> Batch {
    for _ in 0..400 {
        let batch = arbiter.take(now);
        if !batch.decisions.is_empty() || batch.failure.is_some() {
            return batch;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the arbiter did not answer in time");
}

#[test]
fn two_rounds_cross_a_socket() {
    let (address, thread) = rpc::spawn(
        "127.0.0.1:0",
        Scripted::new(None),
        "fixture/over-tcp".to_owned(),
    )
    .expect("the listener binds");

    let mut arbiter = rpc::connect_with_timeout(&address.to_string(), Duration::from_secs(2))
        .expect("the arbiter connects");

    arbiter.deliver(event());

    let first = next_batch(&mut arbiter, 0);
    assert!(first.failure.is_none(), "{:?}", first.failure);
    assert_eq!(first.decisions.len(), 1);
    assert_eq!(first.decisions[0].event, EventId::new(9));
    assert!(matches!(
        first.decisions[0].ruling.completion,
        Completion::Unfinished
    ));

    let second = next_batch(&mut arbiter, 1);
    assert!(second.failure.is_none(), "{:?}", second.failure);
    assert!(matches!(
        second.decisions[0].ruling.completion,
        Completion::Finished(Verdict::Continue)
    ));

    drop(arbiter);
    let _ = thread;
}

#[test]
fn a_missing_arbiter_names_the_address_it_could_not_reach() {
    let error = rpc::connect_with_timeout("127.0.0.1:1", Duration::from_millis(200))
        .expect_err("nothing listens there");

    assert!(error.to_string().contains("127.0.0.1:1"), "{error}");
}
