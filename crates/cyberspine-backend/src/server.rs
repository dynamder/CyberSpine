//! The body side of the protocol: one request frame in, one reply frame out.
//!
//! Both ways of shipping a body — the dynamic-library shim and the TCP server —
//! are thin wrappers around [`Server`], so a body behaves the same whichever way
//! it is packaged.
//!
//! The server checks its manifest against the body **once**, when it is built:
//! the two are hashed independently and a disagreement about identity or shape is
//! refused before the first tick. That is the same check the host makes at load
//! time, moved to the side that owns the body.

use serde::Serialize;

use cyberspine_manifest::from_json;
use cyberspine_rig::BoundFingerprint;
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

use crate::wire::{self, Ack, Commit, Failure, Hello, Kind, Sample, Step, Welcome};

/// The wire form of [`SampleReply`](crate::wire::SampleReply), borrowing the
/// server's buffers so a tick does not clone three vectors only to encode them.
#[derive(Serialize)]
struct SampleReplyRef<'a> {
    seq: u32,
    f: &'a [f32],
    i: &'a [i32],
    muscles: &'a [f32],
}

/// A body plus the buffers its replies are built in.
pub struct Server<B: Backend> {
    body: B,
    rig: String,
    build_id: String,
    shape: Shape,
    f: Vec<f32>,
    i: Vec<i32>,
    muscles: Vec<f32>,
    commands: Vec<Option<f32>>,
    reply: Vec<u8>,
    /// The request sequence last executed, so a transport buffer retry does not
    /// run `Commit` / `Step` a second time.
    last_seq: Option<u32>,
}

impl<B: Backend> Server<B> {
    /// Build a server for `body`, refusing a manifest that does not describe it.
    pub fn new(body: B, rig: String, build_id: String) -> Result<Self, BackendError> {
        let parsed = from_json(&rig).map_err(BackendError::new)?;
        let declared = BoundFingerprint::of(&parsed);
        if declared != body.fingerprint() {
            return Err(BackendError::message(format!(
                "the manifest describes rig {declared}, the body is {}",
                body.fingerprint()
            )));
        }

        let shape = Shape {
            muscles: parsed.muscles.len(),
            sensors_f: parsed.sensors_f.len(),
            sensors_i: parsed.sensors_i.len(),
        };
        if shape != body.shape() {
            return Err(BackendError::message(format!(
                "the manifest describes {shape:?}, the body fills {:?}",
                body.shape()
            )));
        }

        Ok(Self {
            f: vec![0.0; shape.sensors_f],
            i: vec![0; shape.sensors_i],
            muscles: vec![0.0; shape.muscles],
            commands: vec![None; shape.muscles],
            reply: Vec::new(),
            last_seq: None,
            body,
            rig,
            build_id,
            shape,
        })
    }

    pub fn body(&self) -> &B {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut B {
        &mut self.body
    }

    /// Answer exactly one request frame.
    ///
    /// The returned slice is the reply frame, borrowed from the server's reusable
    /// buffer: copy it out before the next call. Every refusal — a bad frame, a
    /// body error, an unexpected kind — comes back as a `Failure` frame, so a
    /// transport never has to invent a message.
    pub fn answer(&mut self, request: &[u8]) -> &[u8] {
        let refusal = match wire::split(request) {
            Ok((kind, payload)) => self.dispatch(kind, payload),
            Err(error) => Err(Failure {
                seq: 0,
                code: wire::code::PROTOCOL,
                message: error.to_string(),
            }),
        };

        match refusal {
            Ok(()) => &self.reply,
            Err(failure) => {
                wire::encode(Kind::Error, &failure, &mut self.reply)
                    .expect("a Failure is always encodable");
                &self.reply
            }
        }
    }

    fn dispatch(&mut self, kind: Kind, payload: &[u8]) -> Result<(), Failure> {
        match kind {
            Kind::Hello => {
                let hello: Hello = wire::decode(payload).map_err(protocol)?;
                if hello.version != wire::VERSION {
                    return Err(Failure {
                        seq: 0,
                        code: wire::code::PROTOCOL,
                        message: format!(
                            "the host speaks protocol {}, this body speaks {}",
                            hello.version,
                            wire::VERSION
                        ),
                    });
                }
                let welcome = Welcome {
                    version: wire::VERSION,
                    rig: self.rig.clone(),
                    build_id: self.build_id.clone(),
                };
                wire::encode(Kind::Hello, &welcome, &mut self.reply).map_err(protocol)
            }

            Kind::Sample => {
                let request: Sample = wire::decode(payload).map_err(protocol)?;
                if self.seen(request.seq) {
                    return Ok(());
                }
                let mut snapshot = Snapshot {
                    f: &mut self.f,
                    i: &mut self.i,
                    muscles: &mut self.muscles,
                };
                self.body
                    .sample(&mut snapshot)
                    .map_err(|error| refused(request.seq, error))?;

                let reply = SampleReplyRef {
                    seq: request.seq,
                    f: &self.f,
                    i: &self.i,
                    muscles: &self.muscles,
                };
                wire::encode(Kind::Sample, &reply, &mut self.reply).map_err(protocol)
            }

            Kind::Commit => {
                let request: Commit = wire::decode(payload).map_err(protocol)?;
                if self.seen(request.seq) {
                    return Ok(());
                }
                wire::validate_commit(&request, self.shape.muscles).map_err(protocol)?;
                for (index, slot) in self.commands.iter_mut().enumerate() {
                    *slot = wire::is_set(&request.mask, index).then(|| request.values[index]);
                }
                self.body
                    .commit(&self.commands, request.dt)
                    .map_err(|error| refused(request.seq, error))?;
                wire::encode(Kind::Commit, &Ack { seq: request.seq }, &mut self.reply)
                    .map_err(protocol)
            }

            Kind::Step => {
                let request: Step = wire::decode(payload).map_err(protocol)?;
                if self.seen(request.seq) {
                    return Ok(());
                }
                self.body
                    .step(request.dt)
                    .map_err(|error| refused(request.seq, error))?;
                wire::encode(Kind::Step, &Ack { seq: request.seq }, &mut self.reply)
                    .map_err(protocol)
            }

            Kind::Error => Err(Failure {
                seq: 0,
                code: wire::code::PROTOCOL,
                message: "a Failure is a reply, not a request".into(),
            }),
        }
    }

    /// Whether this request was already executed.
    ///
    /// The ABI reports a too-small reply buffer *after* running the request, so a
    /// transport that retries would step the body twice. Sequence numbers make
    /// the retry a lookup instead of a second execution; the reply it wants is
    /// still in `self.reply`.
    fn seen(&mut self, seq: u32) -> bool {
        if self.last_seq == Some(seq) {
            return true;
        }
        self.last_seq = Some(seq);
        false
    }
}

fn protocol(error: wire::WireError) -> Failure {
    Failure {
        seq: 0,
        code: wire::code::PROTOCOL,
        message: error.to_string(),
    }
}

fn refused(seq: u32, error: BackendError) -> Failure {
    Failure {
        seq,
        code: wire::code::BODY,
        message: error.to_string(),
    }
}
