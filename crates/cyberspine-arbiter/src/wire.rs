//! The bytes that cross an arbiter boundary.
//!
//! An arbiter speaks the shared [`cyberspine_frame`] head under its own magic
//! (`CSPA`), so a body frame that reaches an arbiter is refused at the magic
//! instead of decoding into the wrong message.
//!
//! # The exchange
//!
//! One request brings everything the arbiter needs for a tick: the host's clock
//! and the events it delivered since the last call. One reply brings every round
//! the arbiter has to report. That makes `deliver` a local enqueue on the host
//! side and keeps a slow arbiter from blocking a tick with a second round trip.
//!
//! `Hello` is the only other exchange: it pins the protocol version and records
//! what built the arbiter, before the first event is sent.

use cyberspine_frame as frame;
pub use cyberspine_frame::{FrameError, HEADER_LEN, MAX_FRAME_LEN};
use serde::{Deserialize, Serialize};

use crate::{Decision, InterruptEvent};

/// The first four bytes of every arbiter frame.
pub const MAGIC: [u8; 4] = *b"CSPA";

/// The protocol version this crate speaks.
pub const VERSION: u16 = 1;

/// The largest a single `source`, `note` or action reference may be.
///
/// The frame cap already bounds the whole reply; this bounds one field, so a
/// buggy arbiter cannot make the host keep a megabyte of text per round.
pub const MAX_TEXT_LEN: usize = 4 << 10;

const PROTOCOL: frame::Protocol = frame::Protocol {
    magic: MAGIC,
    version: VERSION,
};

/// What a frame carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 1,
    Tick = 2,
    Error = 0x7f,
}

impl TryFrom<u8> for Kind {
    type Error = WireError;

    fn try_from(byte: u8) -> Result<Self, WireError> {
        Ok(match byte {
            1 => Self::Hello,
            2 => Self::Tick,
            0x7f => Self::Error,
            other => return Err(WireError::Kind(other)),
        })
    }
}

/// The host's opening frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: u16,
    pub client: String,
}

/// What the arbiter says about itself. Opaque to the host; worth recording next
/// to a run's decision log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub version: u16,
    pub build_id: String,
}

/// One tick: the host's clock and the events delivered since the last call.
///
/// `seq` makes the exchange replayable: a transport that has to retry with a
/// larger buffer re-sends the same `seq`, and the arbiter answers from its cache
/// instead of delivering the events a second time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tick {
    pub seq: u64,
    pub now: u64,
    pub events: Vec<InterruptEvent>,
}

/// The arbiter's answer to a [`Tick`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rulings {
    pub decisions: Vec<Decision>,
    /// Set when the arbiter itself gave up; the host fails the rest.
    pub failure: Option<String>,
}

/// A refusal, in reply to any frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub code: u16,
    pub message: String,
}

/// What a [`Failure`] means.
pub mod code {
    /// The frame itself was wrong: wrong kind here, wrong shape, bad ordering.
    pub const PROTOCOL: u16 = 1;
    /// The arbiter gave up on its own stream.
    pub const ARBITER: u16 = 2;
    /// The arbiter panicked while handling the request.
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
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("unknown frame kind {0}")]
    Kind(u8),
    #[error("the payload does not decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("the payload does not encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
}

/// Encode one complete frame into `out`, replacing its contents.
pub fn encode<T: Serialize>(kind: Kind, value: &T, out: &mut Vec<u8>) -> Result<(), WireError> {
    out.clear();
    out.resize(HEADER_LEN, 0);
    rmp_serde::encode::write(out, value)?;

    let len = out.len() - HEADER_LEN;
    frame::write_header(&PROTOCOL, kind as u8, len, out)?;
    Ok(())
}

/// Read a frame header, leaving the payload to the caller.
pub fn parse_header(bytes: &[u8]) -> Result<Header, WireError> {
    let header = frame::parse_header(&PROTOCOL, bytes)?;
    Ok(Header {
        kind: Kind::try_from(header.kind)?,
        len: header.len,
    })
}

/// Split exactly one frame into its kind and payload.
pub fn split(bytes: &[u8]) -> Result<(Kind, &[u8]), WireError> {
    let (kind, payload) = frame::split(&PROTOCOL, bytes)?;
    Ok((Kind::try_from(kind)?, payload))
}

/// Decode a payload.
pub fn decode<T: serde::de::DeserializeOwned>(payload: &[u8]) -> Result<T, WireError> {
    Ok(rmp_serde::decode::from_slice(payload)?)
}
