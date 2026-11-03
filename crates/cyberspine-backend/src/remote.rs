//! The host side of a body that lives behind a transport.
//!
//! A [`RemoteBody`] is an ordinary [`Backend`]: the handshake fixes its identity
//! and its shape once, and every tick call then becomes one request frame and one
//! reply frame. The bytes are [`wire`](crate::wire)'s, so the same type drives a
//! dynamic library and a TCP connection.
//!
//! # Failure is terminal
//!
//! A transport failure is not retried. The body is unreachable, so the first
//! error is latched and every later call returns it without touching the
//! transport — which is also why `Session::tick` can stop on the first `Err` and
//! never has to ask again.

use serde::de::DeserializeOwned;

use cyberspine_manifest::from_json;
use cyberspine_rig::{BoundFingerprint, Rig};
use cyberspine_vm::{Backend, BackendError, Shape, Snapshot};

use crate::wire::{self, Ack, Commit, Failure, Hello, Kind, Sample, SampleReply, Step, Welcome};

/// One request frame in, one reply frame out.
///
/// The implementation owns the buffer: after a successful call `response[..n]`
/// is exactly one frame, and `n` is the return value. The length is separate
/// from `response.len()` so a transport can keep its buffer at capacity and
/// reuse it without zeroing or shrinking.
pub trait Transport {
    fn exchange(&mut self, request: &[u8], response: &mut Vec<u8>) -> Result<usize, BackendError>;
}

/// A body reached over a [`Transport`].
pub struct RemoteBody<T: Transport> {
    transport: T,
    rig: Rig,
    fingerprint: BoundFingerprint,
    shape: Shape,
    build_id: String,
    request: Vec<u8>,
    response: Vec<u8>,
    commit: Commit,
    seq: u32,
    failure: Option<Box<str>>,
}

impl<T: Transport> RemoteBody<T> {
    /// Introduce the host, then derive the body's identity from what it declares.
    ///
    /// The body answers with its *manifest*, not with a fingerprint: the host
    /// parses it, validates it and hashes it itself, so a 32-byte value read back
    /// from a device can never become a [`BoundFingerprint`] by being painted with
    /// the type (`docs/rig.md` §4).
    pub fn handshake(mut transport: T) -> Result<Self, BackendError> {
        let hello = Hello {
            version: wire::VERSION,
            client: format!("cyberspine-backend/{}", env!("CARGO_PKG_VERSION")),
        };

        let mut request = Vec::new();
        let mut response = Vec::new();
        wire::encode(Kind::Hello, &hello, &mut request).map_err(BackendError::new)?;
        let len = transport.exchange(&request, &mut response)?;
        if len > response.len() {
            return Err(BackendError::message(
                "the transport returned a reply larger than its buffer",
            ));
        }

        let welcome = claim(&response[..len])?;
        if welcome.version != wire::VERSION {
            return Err(BackendError::message(format!(
                "the body speaks protocol {}, this host speaks {}",
                welcome.version,
                wire::VERSION
            )));
        }
        if welcome.rig.len() > wire::MAX_RIG_JSON {
            return Err(BackendError::message(
                "the body's rig manifest is above the size cap",
            ));
        }

        let rig = from_json(&welcome.rig).map_err(BackendError::new)?;
        let shape = Shape {
            muscles: rig.muscles.len(),
            sensors_f: rig.sensors_f.len(),
            sensors_i: rig.sensors_i.len(),
        };

        Ok(Self {
            transport,
            fingerprint: BoundFingerprint::of(&rig),
            rig,
            shape,
            build_id: welcome.build_id,
            request,
            response,
            commit: Commit {
                seq: 0,
                dt: 0.0,
                mask: Vec::new(),
                values: Vec::new(),
            },
            seq: 0,
            failure: None,
        })
    }

    /// The body's declaration, as parsed and validated by the host.
    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    /// What the body says built it. Opaque to the host; worth recording next to a
    /// replay (`docs/isa.md` §9.4.2).
    pub fn build_id(&self) -> &str {
        &self.build_id
    }

    /// Why this body stopped, once it has.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// The transport underneath, for a caller that has to inspect or reconfigure
    /// it. The body's protocol state is not reachable through it.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn next_seq(&mut self) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    fn check_alive(&self) -> Result<(), BackendError> {
        match &self.failure {
            Some(message) => Err(BackendError::message(format!(
                "the body is already failed: {message}"
            ))),
            None => Ok(()),
        }
    }

    fn fail(&mut self, error: BackendError) -> BackendError {
        if self.failure.is_none() {
            self.failure = Some(error.to_string().into_boxed_str());
        }
        error
    }

    /// Send the frame already in `self.request` and decode the reply as `R`.
    fn call<R: DeserializeOwned>(&mut self, kind: Kind) -> Result<R, BackendError> {
        self.check_alive()?;

        let len = match self.transport.exchange(&self.request, &mut self.response) {
            Ok(len) => len,
            Err(error) => return Err(self.fail(error)),
        };
        if len > self.response.len() {
            return Err(self.fail(BackendError::message(
                "the transport returned a reply larger than its buffer",
            )));
        }

        let outcome: Result<R, BackendError> = match wire::split(&self.response[..len]) {
            Ok((Kind::Error, payload)) => match wire::decode::<Failure>(payload) {
                Ok(failure) => Err(BackendError::message(failure.message)),
                Err(error) => Err(BackendError::new(error)),
            },
            Ok((reply, payload)) if reply == kind => {
                wire::decode(payload).map_err(BackendError::new)
            }
            Ok((reply, _)) => Err(BackendError::message(format!(
                "asked for {kind:?}, got {reply:?}"
            ))),
            Err(error) => Err(BackendError::new(error)),
        };

        match outcome {
            Ok(reply) => Ok(reply),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn check_answer(seq: u32, got: u32) -> Result<(), BackendError> {
        if seq == got {
            return Ok(());
        }
        Err(BackendError::message(format!(
            "the reply answers request {got}, not {seq}"
        )))
    }
}

impl<T: Transport> core::fmt::Debug for RemoteBody<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteBody")
            .field("shape", &self.shape)
            .field("build_id", &self.build_id)
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}

impl<T: Transport> Backend for RemoteBody<T> {
    fn fingerprint(&self) -> BoundFingerprint {
        self.fingerprint
    }

    fn shape(&self) -> Shape {
        self.shape
    }

    fn sample(&mut self, out: &mut Snapshot<'_>) -> Result<(), BackendError> {
        self.check_alive()?;
        let seq = self.next_seq();
        if let Err(error) = wire::encode(Kind::Sample, &Sample { seq }, &mut self.request) {
            return Err(self.fail(BackendError::new(error)));
        }

        let reply: SampleReply = self.call(Kind::Sample)?;
        if let Err(error) = Self::check_answer(seq, reply.seq) {
            return Err(self.fail(error));
        }
        if reply.f.len() != self.shape.sensors_f
            || reply.i.len() != self.shape.sensors_i
            || reply.muscles.len() != self.shape.muscles
        {
            return Err(self.fail(BackendError::message(
                "the snapshot does not match the rig the body declared",
            )));
        }

        out.f.copy_from_slice(&reply.f);
        out.i.copy_from_slice(&reply.i);
        out.muscles.copy_from_slice(&reply.muscles);
        Ok(())
    }

    fn commit(&mut self, commands: &[Option<f32>], dt: f32) -> Result<(), BackendError> {
        self.check_alive()?;
        let muscles = self.shape.muscles;
        if commands.len() != muscles {
            return Err(BackendError::message(
                "the host merged a command vector of the wrong length",
            ));
        }

        let seq = self.next_seq();
        self.commit.seq = seq;
        self.commit.dt = dt;
        self.commit.values.clear();
        self.commit.values.resize(muscles, 0.0);
        self.commit.mask.clear();
        self.commit.mask.resize(muscles.div_ceil(64), 0);
        for (index, command) in commands.iter().enumerate() {
            let Some(value) = command else { continue };
            self.commit.values[index] = *value;
            self.commit.mask[index / 64] |= 1 << (index % 64);
        }

        if let Err(error) = wire::encode(Kind::Commit, &self.commit, &mut self.request) {
            return Err(self.fail(BackendError::new(error)));
        }
        let ack: Ack = self.call(Kind::Commit)?;
        if let Err(error) = Self::check_answer(seq, ack.seq) {
            return Err(self.fail(error));
        }
        Ok(())
    }

    fn step(&mut self, dt: f32) -> Result<(), BackendError> {
        self.check_alive()?;
        let seq = self.next_seq();
        if let Err(error) = wire::encode(Kind::Step, &Step { seq, dt }, &mut self.request) {
            return Err(self.fail(BackendError::new(error)));
        }
        let ack: Ack = self.call(Kind::Step)?;
        if let Err(error) = Self::check_answer(seq, ack.seq) {
            return Err(self.fail(error));
        }
        Ok(())
    }
}

/// Decode the body's answer to the handshake, turning a refusal into an error.
fn claim(response: &[u8]) -> Result<Welcome, BackendError> {
    let (kind, payload) = wire::split(response).map_err(BackendError::new)?;
    match kind {
        Kind::Hello => wire::decode(payload).map_err(BackendError::new),
        Kind::Error => {
            let failure: Failure = wire::decode(payload).map_err(BackendError::new)?;
            Err(BackendError::message(format!(
                "the body refused the handshake: {}",
                failure.message
            )))
        }
        other => Err(BackendError::message(format!(
            "the handshake was answered with {other:?}"
        ))),
    }
}
