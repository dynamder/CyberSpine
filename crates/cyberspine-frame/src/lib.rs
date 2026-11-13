//! The 12-byte frame head shared by every CyberSpine plugin protocol.
//!
//! ```text
//! magic | version u16 | kind u8 | flags u8 = 0 | payload_len u32
//! ```
//!
//! Both integers are little-endian. The head is hand-written and frozen here;
//! the payload is the protocol's own business. A body speaks `CSPB`-magic frames
//! and an arbiter speaks `CSPA`-magic frames, so a frame that reaches the wrong
//! plugin fails at the magic instead of decoding into the wrong message.
//!
//! The type does not allocate: a writer reserves [`HEADER_LEN`] bytes, appends
//! its payload, then fills the head in place, and a reader parses the head
//! before touching a byte of payload.

#![no_std]

extern crate alloc;

/// The first twelve bytes of every frame.
pub const HEADER_LEN: usize = 12;

/// The largest payload a frame may declare or carry.
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// The magic and version one protocol pins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    pub magic: [u8; 4],
    pub version: u16,
}

/// A parsed frame head, with the payload still to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: u8,
    pub len: usize,
}

/// Why a frame head or a framed payload was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    ShortHeader(usize),
    Magic([u8; 4]),
    Version(u16),
    Flags(u8),
    TooLarge(usize),
    Length { declared: usize, actual: usize },
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortHeader(actual) => {
                write!(
                    f,
                    "a frame needs at least the {HEADER_LEN}-byte header, got {actual}"
                )
            }
            Self::Magic(magic) => write!(f, "bad frame magic {magic:02x?}"),
            Self::Version(version) => write!(f, "protocol version {version} is not the pinned one"),
            Self::Flags(flags) => write!(f, "frame flags must be zero, got {flags:#04x}"),
            Self::TooLarge(len) => {
                write!(
                    f,
                    "a payload of {len} bytes is above the {MAX_FRAME_LEN}-byte cap"
                )
            }
            Self::Length { declared, actual } => {
                write!(
                    f,
                    "frame declares {declared} payload bytes but carries {actual}"
                )
            }
        }
    }
}

impl core::error::Error for FrameError {}

/// Fill the reserved head in place.
///
/// `out` must already hold [`HEADER_LEN`] zeroed bytes followed by the payload;
/// a shorter buffer is a caller bug and is refused rather than truncated.
pub fn write_header(
    protocol: &Protocol,
    kind: u8,
    len: usize,
    out: &mut [u8],
) -> Result<(), FrameError> {
    if out.len() < HEADER_LEN {
        return Err(FrameError::ShortHeader(out.len()));
    }
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    out[..4].copy_from_slice(&protocol.magic);
    out[4..6].copy_from_slice(&protocol.version.to_le_bytes());
    out[6] = kind;
    out[7] = 0;
    out[8..12].copy_from_slice(&(len as u32).to_le_bytes());
    Ok(())
}

/// Read a frame head, leaving the payload to the caller.
pub fn parse_header(protocol: &Protocol, bytes: &[u8]) -> Result<Header, FrameError> {
    if bytes.len() < HEADER_LEN {
        return Err(FrameError::ShortHeader(bytes.len()));
    }
    let magic: [u8; 4] = bytes[..4].try_into().expect("four bytes");
    if magic != protocol.magic {
        return Err(FrameError::Magic(magic));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != protocol.version {
        return Err(FrameError::Version(version));
    }
    if bytes[7] != 0 {
        return Err(FrameError::Flags(bytes[7]));
    }
    let len = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes")) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    Ok(Header {
        kind: bytes[6],
        len,
    })
}

/// Split exactly one frame into its kind byte and payload.
pub fn split<'b>(protocol: &Protocol, bytes: &'b [u8]) -> Result<(u8, &'b [u8]), FrameError> {
    let header = parse_header(protocol, bytes)?;
    let actual = bytes.len() - HEADER_LEN;
    if header.len != actual {
        return Err(FrameError::Length {
            declared: header.len,
            actual,
        });
    }
    Ok((header.kind, &bytes[HEADER_LEN..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    const PROTOCOL: Protocol = Protocol {
        magic: *b"TEST",
        version: 1,
    };

    fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0; HEADER_LEN + payload.len()];
        out[HEADER_LEN..].copy_from_slice(payload);
        write_header(&PROTOCOL, kind, payload.len(), &mut out).expect("a legal head");
        out
    }

    #[test]
    fn a_head_round_trips() {
        let bytes = frame(7, b"hello");
        let (kind, payload) = split(&PROTOCOL, &bytes).expect("a legal frame");
        assert_eq!(kind, 7);
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn the_head_is_checked_before_the_payload() {
        let good = frame(7, b"hello");

        assert_eq!(
            parse_header(&PROTOCOL, &good[..4]),
            Err(FrameError::ShortHeader(4))
        );

        let mut bad = good.clone();
        bad[0] = b'X';
        assert_eq!(
            parse_header(&PROTOCOL, &bad),
            Err(FrameError::Magic(*b"XEST"))
        );

        let mut bad = good.clone();
        bad[4] = 2;
        assert_eq!(parse_header(&PROTOCOL, &bad), Err(FrameError::Version(2)));

        let mut bad = good.clone();
        bad[7] = 1;
        assert_eq!(parse_header(&PROTOCOL, &bad), Err(FrameError::Flags(1)));

        let mut bad = good.clone();
        bad[8] = 3;
        assert_eq!(
            split(&PROTOCOL, &bad),
            Err(FrameError::Length {
                declared: 3,
                actual: 5
            })
        );
    }

    #[test]
    fn another_protocols_magic_is_refused() {
        let bytes = frame(1, b"");
        let other = Protocol {
            magic: *b"OTHR",
            version: 1,
        };
        assert_eq!(
            parse_header(&other, &bytes),
            Err(FrameError::Magic(*b"TEST"))
        );
    }
}
