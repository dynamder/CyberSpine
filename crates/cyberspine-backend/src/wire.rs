//! The bytes that cross a process or library boundary.
//!
//! A frame is a 12-byte header and a MessagePack payload:
//!
//! ```text
//! magic "CSPB" | version u16 | kind u8 | flags u8 = 0 | payload_len u32 | payload
//! ```
//!
//! Both integers are little-endian; the header is hand-written and frozen here.
//! The payload is a `serde` struct encoded by `rmp-serde` with structs as arrays,
//! so it is positional and a plugin in another language reads a plain list. The
//! floats are MessagePack `float32` (`0xca`) and keep their bit pattern, NaN and
//! all — a command vector has to survive the trip exactly.
//!
//! # Replies
//!
//! A reply carries its request's kind: `Sample` answers with a [`SampleReply`],
//! `Commit` and `Step` answer with an [`Ack`], `Hello` with a [`Welcome`].
//! [`Kind::Error`] is reply-only and carries a [`Failure`].
//!
//! # Canonical commands
//!
//! [`Commit`] separates values from a bitmask, and a clear bit must carry `+0.0`.
//! That gives one command vector exactly one encoding, so a recorded trace can be
//! compared or hashed without normalising it first.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The first four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"CSPB";

/// The protocol version this crate speaks. A peer that disagrees is refused
/// before a single message is interpreted.
pub const VERSION: u16 = 1;

/// `magic | version | kind | flags | payload_len`.
pub const HEADER_LEN: usize = 12;

/// The largest payload this crate will read or write.
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// The largest manifest a body may send in its [`Welcome`].
pub const MAX_RIG_JSON: usize = 64 << 10;

/// What a frame carries. It stays in the header as a byte; the payload's type is
/// implied by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 1,
    Sample = 2,
    Commit = 3,
    Step = 4,
    Error = 0x7f,
}

impl TryFrom<u8> for Kind {
    type Error = WireError;

    fn try_from(byte: u8) -> Result<Self, WireError> {
        Ok(match byte {
            1 => Self::Hello,
            2 => Self::Sample,
            3 => Self::Commit,
            4 => Self::Step,
            0x7f => Self::Error,
            other => return Err(WireError::Kind(other)),
        })
    }
}

/// The client's opening frame: its protocol version and who it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: u16,
    pub client: String,
}

/// The body's answer to [`Hello`]: its identity, its rig, and what built it.
///
/// `rig` is the body's own manifest text. The host parses it, validates it and
/// derives the fingerprint itself — a 32-byte hash is never taken on faith
/// (`docs/rig.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub version: u16,
    pub rig: String,
    pub build_id: String,
}

/// Ask for the body's current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    pub seq: u32,
}

/// One instant of the body: its observables, and every muscle read back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleReply {
    pub seq: u32,
    pub f: Vec<f32>,
    pub i: Vec<i32>,
    pub muscles: Vec<f32>,
}

/// Hand over this tick's merged command vector.
///
/// `mask` has one bit per muscle, least significant bit first; `values` has one
/// entry per muscle. A clear bit means "no instance published this muscle", which
/// the body must read as "keep the last command", not "release".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    pub seq: u32,
    pub dt: f32,
    pub mask: Vec<u64>,
    pub values: Vec<f32>,
}

/// Advance the body by one tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub seq: u32,
    pub dt: f32,
}

/// The body's "done" for a `Commit` or `Step`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    pub seq: u32,
}

/// A refusal, in reply to any frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub seq: u32,
    pub code: u16,
    pub message: String,
}

/// What a [`Failure`] means. The text is for a human; the code is for the caller.
pub mod code {
    /// The frame itself was wrong: wrong kind here, wrong shape, bad ordering.
    pub const PROTOCOL: u16 = 1;
    /// The body refused the call (the equivalent of `Backend::sample` returning
    /// an error).
    pub const BODY: u16 = 2;
    /// The body panicked while handling the request.
    pub const PANIC: u16 = 3;
}

/// A parsed frame header, with the payload still to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub len: usize,
}

/// Why a frame or payload was refused.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("a frame needs at least the {HEADER_LEN}-byte header, got {0}")]
    ShortHeader(usize),
    #[error("bad frame magic {0:02x?}")]
    Magic([u8; 4]),
    #[error("protocol version {0} is not {VERSION}")]
    Version(u16),
    #[error("unknown frame kind {0}")]
    Kind(u8),
    #[error("frame flags must be zero, got {0:#04x}")]
    Flags(u8),
    #[error("frame declares {declared} payload bytes but carries {actual}")]
    Length { declared: usize, actual: usize },
    #[error("a payload of {0} bytes is above the {MAX_FRAME_LEN}-byte cap")]
    TooLarge(usize),
    #[error("the payload does not decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("the payload does not encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("the commit vector is not canonical: {0}")]
    Commit(&'static str),
}

/// Encode one complete frame into `out`, replacing its contents.
///
/// The buffer is reused: after the first call it only grows when the payload
/// does, so the steady-state tick path does not allocate here.
pub fn encode<T: Serialize>(kind: Kind, value: &T, out: &mut Vec<u8>) -> Result<(), WireError> {
    out.clear();
    out.resize(HEADER_LEN, 0);
    rmp_serde::encode::write(out, value)?;

    let len = out.len() - HEADER_LEN;
    if len > MAX_FRAME_LEN {
        return Err(WireError::TooLarge(len));
    }
    write_header(kind, len, out);
    Ok(())
}

fn write_header(kind: Kind, len: usize, out: &mut [u8]) {
    out[..4].copy_from_slice(&MAGIC);
    out[4..6].copy_from_slice(&VERSION.to_le_bytes());
    out[6] = kind as u8;
    out[7] = 0;
    out[8..12].copy_from_slice(&(len as u32).to_le_bytes());
}

/// Read a frame header, leaving the payload to the caller.
pub fn parse_header(bytes: &[u8]) -> Result<Header, WireError> {
    if bytes.len() < HEADER_LEN {
        return Err(WireError::ShortHeader(bytes.len()));
    }
    let magic: [u8; 4] = bytes[..4].try_into().expect("four bytes");
    if magic != MAGIC {
        return Err(WireError::Magic(magic));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let kind = Kind::try_from(bytes[6])?;
    if bytes[7] != 0 {
        return Err(WireError::Flags(bytes[7]));
    }
    let len = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes")) as usize;
    if len > MAX_FRAME_LEN {
        return Err(WireError::TooLarge(len));
    }
    Ok(Header { kind, len })
}

/// Split exactly one frame into its kind and payload.
pub fn split(bytes: &[u8]) -> Result<(Kind, &[u8]), WireError> {
    let header = parse_header(bytes)?;
    let actual = bytes.len() - HEADER_LEN;
    if header.len != actual {
        return Err(WireError::Length {
            declared: header.len,
            actual,
        });
    }
    Ok((header.kind, &bytes[HEADER_LEN..]))
}

/// Decode a payload.
pub fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, WireError> {
    Ok(rmp_serde::decode::from_slice(payload)?)
}

/// Check a [`Commit`] against the body's muscle count, and its canonical form.
///
/// The count is the body's, from the handshake; only the body and the host know
/// it, so it is a parameter rather than a field that could disagree with `mask`.
pub fn validate_commit(commit: &Commit, muscles: usize) -> Result<(), WireError> {
    if commit.values.len() != muscles {
        return Err(WireError::Commit("the value count does not match the rig"));
    }
    if commit.mask.len() != muscles.div_ceil(64) {
        return Err(WireError::Commit("the mask length does not match the rig"));
    }

    for (index, value) in commit.values.iter().enumerate() {
        if !is_set(&commit.mask, index) && value.to_bits() != 0 {
            return Err(WireError::Commit(
                "a value is present under a clear mask bit",
            ));
        }
    }

    let tail = muscles % 64;
    if tail != 0 {
        let last = commit.mask.last().copied().expect("at least one word");
        if last >> tail != 0 {
            return Err(WireError::Commit("mask bits past the last muscle are set"));
        }
    }
    Ok(())
}

/// Whether `mask` has `index` set. Out-of-range indexes are clear, which is what
/// [`validate_commit`] uses after checking the length.
pub fn is_set(mask: &[u64], index: usize) -> bool {
    mask.get(index / 64)
        .is_some_and(|word| word & (1 << (index % 64)) != 0)
}
