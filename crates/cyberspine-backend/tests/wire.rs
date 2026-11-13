//! The wire format, pinned. The golden bytes are the contract: a plugin in
//! another language implements these, not whatever `rmp-serde` happens to do
//! next. If a dependency changes its mind, this test is what notices.

use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;

use cyberspine_backend::wire::{
    self, Ack, Commit, Failure, Hello, Kind, Sample, SampleReply, Step, Welcome,
};

fn frame(kind: Kind, value: &impl Serialize) -> Vec<u8> {
    let mut bytes = Vec::new();
    wire::encode(kind, value, &mut bytes).expect("the fixture encodes");
    bytes
}

fn round_trip<T>(kind: Kind, value: T)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let bytes = frame(kind, &value);
    let (got, payload) = wire::split(&bytes).expect("one frame");
    assert_eq!(got, kind);
    let back: T = wire::decode(payload).expect("the payload decodes");
    assert_eq!(back, value);
}

#[test]
fn a_step_frame_is_the_frozen_header_and_payload() {
    let bytes = frame(Kind::Step, &Step { seq: 1, dt: 0.5 });

    assert_eq!(
        bytes,
        [
            0x43, 0x53, 0x50, 0x42, // "CSPB"
            0x01, 0x00, // version 1, little-endian
            0x04, // Step
            0x00, // flags
            0x07, 0x00, 0x00, 0x00, // seven payload bytes
            0x92, // two fields
            0x01, // seq = 1, a positive fixint
            0xca, 0x3f, 0x00, 0x00, 0x00, // f32 0.5
        ]
    );
}

#[test]
fn a_hello_frame_carries_the_version_as_a_fixint() {
    let bytes = frame(
        Kind::Hello,
        &Hello {
            version: 1,
            client: "csp".into(),
        },
    );

    assert_eq!(
        bytes,
        [
            0x43, 0x53, 0x50, 0x42, 0x01, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00, 0x92, 0x01,
            0xa3, b'c', b's', b'p',
        ]
    );
}

#[test]
fn a_reply_keeps_its_requests_kind() {
    let bytes = frame(Kind::Commit, &Ack { seq: 1 });

    assert_eq!(
        bytes,
        [
            0x43, 0x53, 0x50, 0x42, 0x01, 0x00, 0x03, 0x00, 0x02, 0x00, 0x00, 0x00, 0x91, 0x01,
        ]
    );
}

#[test]
fn every_message_round_trips() {
    round_trip(
        Kind::Hello,
        Hello {
            version: 1,
            client: "host".into(),
        },
    );
    round_trip(
        Kind::Hello,
        Welcome {
            version: 1,
            rig: "{\"rig\":{\"name\":\"r\",\"version\":1}}".into(),
            build_id: "0.1.0".into(),
        },
    );
    round_trip(Kind::Sample, Sample { seq: 7 });
    round_trip(
        Kind::Sample,
        SampleReply {
            seq: 7,
            f: vec![0.25, -1.5],
            i: vec![-3, 9],
            muscles: vec![1.0, 0.0],
        },
    );
    round_trip(
        Kind::Commit,
        Commit {
            seq: 8,
            dt: 1.0 / 60.0,
            mask: vec![0b101],
            values: vec![1.0, 0.0, 0.5],
        },
    );
    round_trip(Kind::Step, Step { seq: 9, dt: 0.25 });
    round_trip(Kind::Commit, Ack { seq: 10 });
    round_trip(
        Kind::Error,
        Failure {
            seq: 11,
            code: wire::code::BODY,
            message: "no".into(),
        },
    );
}

#[test]
fn the_header_is_checked_before_the_payload() {
    let good = frame(Kind::Step, &Step { seq: 1, dt: 0.5 });

    assert!(matches!(
        wire::parse_header(&good[..4]),
        Err(wire::WireError::Frame(wire::FrameError::ShortHeader(4)))
    ));

    let mut bad = good.clone();
    bad[0] = b'X';
    assert!(matches!(
        wire::parse_header(&bad),
        Err(wire::WireError::Frame(wire::FrameError::Magic(magic))) if magic == *b"XSPB"
    ));

    let mut bad = good.clone();
    bad[4] = 2;
    assert!(matches!(
        wire::parse_header(&bad),
        Err(wire::WireError::Frame(wire::FrameError::Version(2)))
    ));

    let mut bad = good.clone();
    bad[6] = 0x66;
    assert!(matches!(
        wire::parse_header(&bad),
        Err(wire::WireError::Kind(0x66))
    ));

    let mut bad = good.clone();
    bad[7] = 1;
    assert!(matches!(
        wire::parse_header(&bad),
        Err(wire::WireError::Frame(wire::FrameError::Flags(1)))
    ));

    let mut bad = good.clone();
    bad[8] = 8;
    assert!(matches!(
        wire::split(&bad),
        Err(wire::WireError::Frame(wire::FrameError::Length {
            declared: 8,
            actual: 7
        }))
    ));
}

#[test]
fn a_commit_with_a_value_under_a_clear_bit_is_refused() {
    let commit = Commit {
        seq: 0,
        dt: 0.25,
        mask: vec![0b101],
        values: vec![1.0, 2.0, 0.5],
    };

    assert!(matches!(
        wire::validate_commit(&commit, 3),
        Err(wire::WireError::Commit(
            "a value is present under a clear mask bit"
        ))
    ));
}

#[test]
fn a_commit_must_match_the_rig_it_was_made_for() {
    let commit = Commit {
        seq: 0,
        dt: 0.25,
        mask: vec![0b1],
        values: vec![1.0],
    };

    assert!(matches!(
        wire::validate_commit(&commit, 2),
        Err(wire::WireError::Commit(
            "the value count does not match the rig"
        ))
    ));
    wire::validate_commit(&commit, 1).expect("one muscle, one value");

    let padded = Commit {
        seq: 0,
        dt: 0.25,
        mask: vec![0b10],
        values: vec![0.0],
    };
    assert!(matches!(
        wire::validate_commit(&padded, 1),
        Err(wire::WireError::Commit(
            "mask bits past the last muscle are set"
        ))
    ));
}

#[test]
fn an_empty_rig_has_an_empty_commit() {
    let commit = Commit {
        seq: 0,
        dt: 0.25,
        mask: Vec::new(),
        values: Vec::new(),
    };

    wire::validate_commit(&commit, 0).expect("no muscles, nothing to check");
}
