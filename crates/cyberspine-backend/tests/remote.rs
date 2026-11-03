//! The client half of the protocol, against a scripted peer. A transport that
//! hands back exactly the frames a body would is enough to pin the parts that
//! matter: the handshake, the per-tick calls, and what a failure does.

use std::collections::VecDeque;

use cyberspine_backend::wire::{self, Ack, Commit, Failure, Kind, SampleReply, Welcome};
use cyberspine_backend::{RemoteBody, Transport};
use cyberspine_manifest::from_json;
use cyberspine_rig::BoundFingerprint;
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

const RIG: &str = r#"{"rig":{"name":"probe","version":1},"muscles":["left","right"],"sensors_f":["load"],"sensors_i":["mode"]}"#;

struct Scripted {
    replies: VecDeque<Vec<u8>>,
    requests: Vec<Vec<u8>>,
    fail: bool,
}

impl Scripted {
    fn new(replies: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            requests: Vec::new(),
            fail: false,
        }
    }
}

impl Transport for Scripted {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, BackendError> {
        self.requests.push(request.to_vec());
        if self.fail {
            return Err(BackendError::message("the link is gone"));
        }
        let reply = self
            .replies
            .pop_front()
            .ok_or_else(|| BackendError::message("no scripted reply"))?;
        *response = reply;
        Ok(response.len())
    }
}

fn frame<T: serde::Serialize>(kind: Kind, value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    wire::encode(kind, value, &mut bytes).expect("the fixture encodes");
    bytes
}

fn welcome(rig: &str) -> Vec<u8> {
    frame(
        Kind::Hello,
        &Welcome {
            version: wire::VERSION,
            rig: rig.into(),
            build_id: "test".into(),
        },
    )
}

fn refusal(message: &str) -> Vec<u8> {
    frame(
        Kind::Error,
        &Failure {
            seq: 0,
            code: wire::code::PROTOCOL,
            message: message.into(),
        },
    )
}

fn sample_reply(seq: u32) -> Vec<u8> {
    frame(
        Kind::Sample,
        &SampleReply {
            seq,
            f: vec![0.75],
            i: vec![4],
            muscles: vec![0.5, -0.5],
        },
    )
}

fn ack(seq: u32) -> Vec<u8> {
    frame(Kind::Commit, &Ack { seq })
}

fn snapshot<'a>(f: &'a mut [f32], i: &'a mut [i32], muscles: &'a mut [f32]) -> Snapshot<'a> {
    Snapshot { f, i, muscles }
}

fn body(replies: impl IntoIterator<Item = Vec<u8>>) -> RemoteBody<Scripted> {
    RemoteBody::handshake(Scripted::new(replies)).expect("the handshake succeeds")
}

#[test]
fn the_handshake_derives_the_identity_from_the_declared_rig() {
    let body = body([welcome(RIG)]);

    assert_eq!(body.build_id(), "test");
    assert_eq!(
        body.shape(),
        Shape {
            muscles: 2,
            sensors_f: 1,
            sensors_i: 1
        }
    );
    assert_eq!(
        body.fingerprint(),
        BoundFingerprint::of(&from_json(RIG).expect("the fixture parses"))
    );

    let hello = wire::split(&body.transport().requests[0]).expect("one frame");
    assert_eq!(hello.0, Kind::Hello);
}

#[test]
fn a_sample_reply_fills_the_snapshot() {
    let mut body = body([welcome(RIG), sample_reply(1)]);
    let (mut f, mut i, mut muscles) = ([0.0; 1], [0; 1], [0.0; 2]);

    body.sample(&mut Snapshot {
        f: &mut f,
        i: &mut i,
        muscles: &mut muscles,
    })
    .expect("the reply answers the request");

    assert_eq!(f, [0.75]);
    assert_eq!(i, [4]);
    assert_eq!(muscles, [0.5, -0.5]);
}

#[test]
fn a_commit_carries_the_mask_and_the_values() {
    let mut body = body([welcome(RIG), ack(1)]);

    body.commit(&[Some(0.5), None], 0.25)
        .expect("the body acks");

    let (kind, payload) = wire::split(&body.transport().requests[1]).expect("one frame");
    assert_eq!(kind, Kind::Commit);
    let commit: Commit = wire::decode(payload).expect("the payload decodes");
    assert_eq!(commit.dt, 0.25);
    assert_eq!(commit.mask, [0b01]);
    assert_eq!(commit.values, [0.5, 0.0]);
    wire::validate_commit(&commit, 2).expect("the client wrote a canonical vector");
}

#[test]
fn a_transport_failure_is_latched_and_never_retried() {
    let mut body = body([welcome(RIG)]);
    body.transport_mut().fail = true;

    let (mut f, mut i, mut muscles) = ([0.0; 1], [0; 1], [0.0; 2]);

    let first = body
        .sample(&mut snapshot(&mut f, &mut i, &mut muscles))
        .expect_err("the link is gone");
    assert!(first.to_string().contains("the link is gone"), "{first}");

    let sent = body.transport().requests.len();
    let second = body
        .sample(&mut snapshot(&mut f, &mut i, &mut muscles))
        .expect_err("the body stays failed");
    assert!(second.to_string().contains("already failed"), "{second}");
    assert_eq!(
        body.transport().requests.len(),
        sent,
        "a failed body is never asked again"
    );
    assert_eq!(body.failure(), Some("the link is gone"));
}

#[test]
fn a_reply_of_the_wrong_kind_is_fatal() {
    let mut body = body([welcome(RIG), ack(1)]);
    let (mut f, mut i, mut muscles) = ([0.0; 1], [0; 1], [0.0; 2]);

    let error = body
        .sample(&mut Snapshot {
            f: &mut f,
            i: &mut i,
            muscles: &mut muscles,
        })
        .expect_err("a Commit ack cannot answer a Sample");

    assert!(error.to_string().contains("asked for Sample"), "{error}");
}

#[test]
fn a_snapshot_that_does_not_match_the_rig_is_refused() {
    let mut body = body([
        welcome(RIG),
        frame(
            Kind::Sample,
            &SampleReply {
                seq: 1,
                f: vec![0.0],
                i: vec![0],
                muscles: vec![0.0; 3],
            },
        ),
    ]);
    let (mut f, mut i, mut muscles) = ([0.0; 1], [0; 1], [0.0; 2]);

    let error = body
        .sample(&mut Snapshot {
            f: &mut f,
            i: &mut i,
            muscles: &mut muscles,
        })
        .expect_err("three muscles against a two-muscle rig");

    assert!(
        error.to_string().contains("does not match the rig"),
        "{error}"
    );
}

#[test]
fn a_body_that_refuses_the_handshake_fails_to_open() {
    let error = RemoteBody::handshake(Scripted::new([refusal("not today")]))
        .expect_err("the refusal is an error");

    assert!(error.to_string().contains("not today"), "{error}");
}

#[test]
fn a_rig_the_host_rejects_is_a_handshake_failure() {
    let error = RemoteBody::handshake(Scripted::new([welcome("{}")]))
        .expect_err("an empty manifest is not a rig");

    assert!(error.to_string().contains("manifest") || error.to_string().contains("JSON"));
}
