//! What the body server does with a failure larger than a frame, and with a
//! body that panics: both must still be one legal reply.

use cyberspine_backend::server::{Connection, Serve, Server};
use cyberspine_backend::wire::{self, Failure, Kind, MAX_FRAME_LEN, Sample};
use cyberspine_manifest::from_json;
use cyberspine_rig::BoundFingerprint;
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

const RIG: &str =
    r#"{"rig":{"name":"probe","version":1},"muscles":["m0"],"sensors_f":[],"sensors_i":[]}"#;

fn fingerprint() -> BoundFingerprint {
    BoundFingerprint::of(&from_json(RIG).expect("the test manifest parses"))
}

fn sample_request(seq: u32) -> Vec<u8> {
    let mut request = Vec::new();
    wire::encode(Kind::Sample, &Sample { seq }, &mut request).expect("the request encodes");
    request
}

fn failure(reply: &[u8]) -> Failure {
    let (kind, payload) = wire::split(reply).expect("the refusal is one frame");
    assert_eq!(kind, Kind::Error);
    wire::decode(payload).expect("the failure decodes")
}

struct HugeError {
    fingerprint: BoundFingerprint,
}

impl HugeError {
    fn new() -> Self {
        Self {
            fingerprint: fingerprint(),
        }
    }
}

impl Backend for HugeError {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    fn shape(&self) -> Shape {
        Shape {
            muscles: 1,
            sensors_f: 0,
            sensors_i: 0,
        }
    }

    fn sample(&mut self, _out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        Err(BackendError::message("x".repeat(2 << 20)))
    }

    fn commit(&mut self, _commands: &[Option<f32>], _dt: f32) -> Result<(), BackendError> {
        Ok(())
    }

    fn step(&mut self, _dt: f32) -> Result<(), BackendError> {
        Ok(())
    }
}

#[test]
fn an_overlong_body_error_is_truncated_to_a_legal_frame() {
    let mut server = Server::new(HugeError::new(), RIG.to_owned(), "test".to_owned())
        .expect("the manifest describes the body");
    let mut connection = Connection::default();

    let failure = failure(server.answer(&mut connection, &sample_request(1)));

    assert_eq!(failure.code, wire::code::BODY);
    assert!(
        failure.message.len() <= MAX_FRAME_LEN,
        "the message is {} bytes",
        failure.message.len()
    );
    assert!(
        failure.message.ends_with("…(truncated)"),
        "a capped message says it was cut"
    );
    assert!(failure.message.starts_with('x'), "the prefix is kept");
}

struct Panicky {
    fingerprint: BoundFingerprint,
}

impl Panicky {
    fn new() -> Self {
        Self {
            fingerprint: fingerprint(),
        }
    }
}

impl Backend for Panicky {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    fn shape(&self) -> Shape {
        Shape {
            muscles: 1,
            sensors_f: 0,
            sensors_i: 0,
        }
    }

    fn sample(&mut self, _out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        panic!("the body exploded");
    }

    fn commit(&mut self, _commands: &[Option<f32>], _dt: f32) -> Result<(), BackendError> {
        Ok(())
    }

    fn step(&mut self, _dt: f32) -> Result<(), BackendError> {
        Ok(())
    }
}

#[test]
fn a_panicking_body_becomes_a_failure_and_the_server_keeps_answering() {
    let mut server = Server::new(Panicky::new(), RIG.to_owned(), "test".to_owned())
        .expect("the manifest describes the body");
    let mut connection = Connection::default();

    let first = server.answer_frame(&mut connection, &sample_request(1));
    let refusal = failure(&first);
    assert_eq!(refusal.code, wire::code::PANIC);
    assert!(
        refusal.message.contains("the body exploded"),
        "{}",
        refusal.message
    );

    let second = server.answer_frame(&mut connection, &sample_request(2));
    let refusal = failure(&second);
    assert_eq!(refusal.code, wire::code::PANIC);
}
