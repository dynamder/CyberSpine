//! The optional debug segment (`docs/binfmt.md` §6.7).
//!
//! A debug build appends a source path, a label table and a `pc → source
//! position` table to the image; a release build strips them. The VM never reads
//! the segment: the loader validates it and the host renders it, so a broken one
//! can only mislead a reader, never change what runs.
//!
//! The segment is self-delimiting the same way the metadata tables are — its
//! start is wherever the tables end, its end is the hash — so `flags` carries one
//! bit and the fixed header no offset. `line_map` is fixed size and therefore
//! last: the two variable-length tables are bounded by its start.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::image::pack_version;

/// `b"CSPD"`: a frame that reaches the wrong reader fails here (`docs/binfmt.md` §6.7).
pub const MAGIC: [u8; 4] = *b"CSPD";

/// `flags` bit 0: the image carries a debug segment.
pub const FLAG: u16 = 1 << 0;

const MAJOR: u8 = 1;
const MINOR: u8 = 0;
const PATCH: u8 = 0;

/// The debug segment's own version, packed like the image's (`docs/binfmt.md` §4).
pub const VERSION: u16 = pack_version(MAJOR, MINOR, PATCH);

/// Bytes per `line_map` entry: `line u32` then `column u16`.
pub const LINE_LEN: usize = 6;

/// This version carries exactly one source; the count exists for a later
/// `include`-like split, not for today's writer.
const SOURCE_COUNT: u32 = 1;

/// Magic, version, reserved, and the two counts.
const FIXED_LEN: usize = 16;

/// A one-character name, its NUL, and the `pc` — the smallest possible entry.
const LABEL_MIN_LEN: usize = 6;

/// A one-character path and its NUL.
const SOURCE_MIN_LEN: usize = 2;

/// One instruction's source position (`docs/asm.md` §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugLine {
    pub line: u32,
    pub column: u16,
}

/// A label and the instruction it names (`docs/asm.md` §5).
///
/// Labels are stored by name, not by target: the target is `pc`, and that is
/// already what the code section indexes by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugLabel {
    pub name: Box<str>,
    pub pc: u32,
}

/// Everything the debug segment carries, owned.
///
/// The strings are owned rather than arena handles because this type outlives
/// the image in every place that uses it: the host keeps a per-instance copy so a
/// fault can be rendered after the image has been dropped into the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugInfo {
    source: Box<str>,
    labels: Vec<DebugLabel>,
    lines: Vec<DebugLine>,
}

impl DebugInfo {
    /// Build one from the pieces. Labels are sorted by name here, which is what
    /// makes the encoded bytes canonical (§8) without the assembler having to
    /// promise anything about its own iteration order.
    pub fn new(
        source: impl Into<Box<str>>,
        mut labels: Vec<DebugLabel>,
        lines: Vec<DebugLine>,
    ) -> Self {
        labels.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Self {
            source: source.into(),
            labels,
            lines,
        }
    }

    /// The source path, relative to the `.csp`'s directory.
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn labels(&self) -> &[DebugLabel] {
        &self.labels
    }

    pub fn lines(&self) -> &[DebugLine] {
        &self.lines
    }

    /// Where instruction `pc` came from, or `None` when the map does not cover it.
    pub fn line(&self, pc: usize) -> Option<DebugLine> {
        self.lines.get(pc).copied()
    }

    /// The label naming instruction `pc`, if any. Labels are sorted by name, so
    /// this is a scan; the tables are small and nothing on the tick path calls it.
    pub fn label_at(&self, pc: usize) -> Option<&str> {
        self.labels
            .iter()
            .find(|label| label.pc as usize == pc)
            .map(|label| &*label.name)
    }

    /// Every semantic rule of §6.7 that the bytes alone cannot express.
    ///
    /// Split from [`DebugInfo::parse`] so an image built in memory is held to the
    /// same rules as one decoded from disk.
    pub fn check(&self, code_len: usize) -> Result<(), DebugError> {
        if self.source.is_empty() {
            return Err(DebugError::Empty {
                what: "source path",
            });
        }
        if self.source.contains('\0') {
            return Err(DebugError::Nul {
                what: "source path",
            });
        }
        if self.lines.len() != code_len {
            return Err(DebugError::LineMapLength {
                expected: code_len * LINE_LEN,
                found: self.lines.len() * LINE_LEN,
            });
        }

        let mut previous: Option<&str> = None;
        for label in &self.labels {
            if label.name.is_empty() {
                return Err(DebugError::Empty { what: "label name" });
            }
            if label.name.contains('\0') {
                return Err(DebugError::Nul { what: "label name" });
            }
            if previous.is_some_and(|prev| prev >= &*label.name) {
                return Err(DebugError::LabelOrder(label.name.clone()));
            }
            if label.pc as usize >= code_len {
                return Err(DebugError::LabelOutOfRange {
                    name: label.name.clone(),
                    pc: label.pc,
                    code_len: u32::try_from(code_len).unwrap_or(u32::MAX),
                });
            }
            previous = Some(&label.name);
        }

        for (pc, line) in self.lines.iter().enumerate() {
            if line.line == 0 || line.column == 0 {
                return Err(DebugError::ZeroLocation { pc });
            }
        }

        Ok(())
    }

    /// The segment's bytes, exactly as they sit before the content hash.
    ///
    /// # Panics
    ///
    /// On a builder bug the format cannot express: a line map that does not cover
    /// the code, a NUL inside a string, or more labels than a `u32` can count.
    pub fn encode(&self, code_len: usize) -> Vec<u8> {
        assert_eq!(
            self.lines.len(),
            code_len,
            "the debug line map must cover every instruction"
        );
        assert!(
            !self.source.contains('\0'),
            "a debug source path cannot contain NUL"
        );
        assert!(
            !self.labels.iter().any(|label| label.name.contains('\0')),
            "a debug label name cannot contain NUL"
        );
        let labels = u32::try_from(self.labels.len())
            .expect("a debug label table larger than u32 belongs in no image");

        let mut out = Vec::with_capacity(
            FIXED_LEN
                + self.source.len()
                + 1
                + self
                    .labels
                    .iter()
                    .map(|label| label.name.len() + 5)
                    .sum::<usize>()
                + code_len * LINE_LEN,
        );

        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&SOURCE_COUNT.to_le_bytes());
        out.extend_from_slice(&labels.to_le_bytes());

        push_text(&mut out, &self.source);
        for label in &self.labels {
            push_text(&mut out, &label.name);
            out.extend_from_slice(&label.pc.to_le_bytes());
        }
        for line in &self.lines {
            out.extend_from_slice(&line.line.to_le_bytes());
            out.extend_from_slice(&line.column.to_le_bytes());
        }

        debug_assert_eq!(
            out.len(),
            FIXED_LEN
                + self.source.len()
                + 1
                + self
                    .labels
                    .iter()
                    .map(|label| label.name.len() + 5)
                    .sum::<usize>()
                + code_len * LINE_LEN
        );
        out
    }

    /// Decode one segment from `data`, which must be exactly the segment.
    pub fn parse(data: &[u8], code_len: usize) -> Result<Self, DebugError> {
        let mut reader = Reader::new(data);

        if reader.take(4)? != MAGIC {
            return Err(DebugError::BadMagic);
        }
        let version = reader.u16()?;
        if crate::image::format_parts(version).0 != MAJOR {
            return Err(DebugError::UnsupportedVersion(version));
        }
        if reader.u16()? != 0 {
            return Err(DebugError::ReservedNotZero);
        }
        let source_count = reader.u32()?;
        if source_count != SOURCE_COUNT {
            return Err(DebugError::SourceCount(source_count));
        }
        let label_count = reader.u32()?;

        // C5's rule for this segment: no allocation from a number the bytes did
        // not already pay for.
        let lower_bound = FIXED_LEN as u64
            + SOURCE_COUNT as u64 * SOURCE_MIN_LEN as u64
            + label_count as u64 * LABEL_MIN_LEN as u64
            + code_len as u64 * LINE_LEN as u64;
        if (data.len() as u64) < lower_bound {
            return Err(DebugError::Truncated);
        }

        let source = reader.text()?;

        let mut labels = Vec::with_capacity(label_count as usize);
        for _ in 0..label_count {
            let name = reader.text()?;
            let pc = reader.u32()?;
            labels.push(DebugLabel {
                name: name.into(),
                pc,
            });
        }

        let expected = code_len * LINE_LEN;
        if reader.remaining() != expected {
            return Err(DebugError::LineMapLength {
                expected,
                found: reader.remaining(),
            });
        }
        let mut lines = Vec::with_capacity(code_len);
        for _ in 0..code_len {
            let line = reader.u32()?;
            let column = reader.u16()?;
            lines.push(DebugLine { line, column });
        }
        let info = Self {
            source: source.into(),
            labels,
            lines,
        };
        info.check(code_len)?;
        Ok(info)
    }
}

/// Why a debug segment could not be decoded or trusted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DebugError {
    #[error("the debug segment is shorter than its header and tables require")]
    Truncated,
    #[error("the debug segment does not start with `CSPD`")]
    BadMagic,
    #[error(
        "the debug segment version {0:#06x} is not supported; this reader speaks {MAJOR}.{MINOR}.{PATCH}"
    )]
    UnsupportedVersion(u16),
    #[error("the debug segment's reserved field is not zero")]
    ReservedNotZero,
    #[error("the debug segment declares {0} source(s), but this version carries one")]
    SourceCount(u32),
    #[error("the debug {what} is empty")]
    Empty { what: &'static str },
    #[error("the debug {what} contains NUL, which terminates its string")]
    Nul { what: &'static str },
    #[error("a debug string is not valid UTF-8")]
    Utf8,
    #[error("debug label `{name}` points at pc {pc}, but the image has {code_len} instruction(s)")]
    LabelOutOfRange {
        name: Box<str>,
        pc: u32,
        code_len: u32,
    },
    #[error(
        "debug labels are not strictly ordered by name: `{0}` repeats or precedes its predecessor"
    )]
    LabelOrder(Box<str>),
    #[error("the debug line map is {found} byte(s), but {expected} were required")]
    LineMapLength { expected: usize, found: usize },
    #[error("the debug source position of instruction {pc} has line 0 or column 0")]
    ZeroLocation { pc: usize },
}

fn push_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    out.push(0);
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DebugError> {
        let end = self.pos.checked_add(len).ok_or(DebugError::Truncated)?;
        let slice = self.data.get(self.pos..end).ok_or(DebugError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16, DebugError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DebugError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn text(&mut self) -> Result<&'a str, DebugError> {
        let start = self.pos;
        let offset = self.data[start..]
            .iter()
            .position(|&byte| byte == 0)
            .ok_or(DebugError::Truncated)?;
        let bytes = &self.data[start..start + offset];
        self.pos = start + offset + 1;
        core::str::from_utf8(bytes).map_err(|_| DebugError::Utf8)
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn label(name: &str, pc: u32) -> DebugLabel {
        DebugLabel {
            name: name.into(),
            pc,
        }
    }

    fn info() -> DebugInfo {
        DebugInfo::new(
            "maze.csa",
            vec![label("start", 0), label("loop", 1)],
            vec![
                DebugLine { line: 7, column: 5 },
                DebugLine { line: 9, column: 5 },
            ],
        )
    }

    #[test]
    fn encode_and_parse_round_trip() {
        let info = info();
        let bytes = info.encode(2);
        assert_eq!(DebugInfo::parse(&bytes, 2).expect("parses back"), info);
    }

    #[test]
    fn labels_are_sorted_by_name_when_built() {
        let info = DebugInfo::new(
            "s",
            vec![label("z", 0), label("a", 0)],
            vec![DebugLine { line: 1, column: 1 }],
        );
        assert_eq!(info.labels()[0].name.as_ref(), "a");
        assert_eq!(info.labels()[1].name.as_ref(), "z");
    }

    #[test]
    fn a_duplicate_name_is_an_order_error() {
        let info = DebugInfo {
            source: "s".into(),
            labels: vec![label("a", 0), label("a", 0)],
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        assert!(matches!(
            info.check(1),
            Err(DebugError::LabelOrder(name)) if name.as_ref() == "a"
        ));
    }

    #[test]
    fn a_label_past_the_code_is_refused() {
        let info = DebugInfo {
            source: "s".into(),
            labels: vec![label("a", 1)],
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        assert!(matches!(
            info.check(1),
            Err(DebugError::LabelOutOfRange { pc: 1, .. })
        ));
    }

    #[test]
    fn the_line_map_must_cover_every_instruction() {
        let info = DebugInfo {
            source: "s".into(),
            labels: Vec::new(),
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        assert!(matches!(
            info.check(2),
            Err(DebugError::LineMapLength { .. })
        ));
    }

    #[test]
    fn line_zero_and_column_zero_are_refused() {
        for (line, column) in [(0, 1), (1, 0)] {
            let info = DebugInfo {
                source: "s".into(),
                labels: Vec::new(),
                lines: vec![DebugLine { line, column }],
            };
            assert!(matches!(
                info.check(1),
                Err(DebugError::ZeroLocation { pc: 0 })
            ));
        }
    }

    #[test]
    fn an_empty_source_is_refused() {
        let info = DebugInfo {
            source: "".into(),
            labels: Vec::new(),
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        assert!(matches!(info.check(1), Err(DebugError::Empty { .. })));
    }

    #[test]
    fn a_nul_in_a_label_is_refused() {
        let info = DebugInfo {
            source: "s".into(),
            labels: vec![label("a\0b", 0)],
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        assert!(matches!(
            info.check(1),
            Err(DebugError::Nul { what: "label name" })
        ));
    }

    #[test]
    fn a_nul_in_the_source_is_refused() {
        let info = DebugInfo {
            source: "s\0t".into(),
            labels: Vec::new(),
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        assert!(matches!(
            info.check(1),
            Err(DebugError::Nul {
                what: "source path"
            })
        ));
    }

    #[test]
    #[should_panic(expected = "cannot contain NUL")]
    fn encode_refuses_a_nul_label() {
        let info = DebugInfo {
            source: "s".into(),
            labels: vec![label("a\0b", 0)],
            lines: vec![DebugLine { line: 1, column: 1 }],
        };
        let _ = info.encode(1);
    }

    #[test]
    fn parse_rejects_a_foreign_magic() {
        let mut bytes = info().encode(2);
        bytes[0] = b'X';
        assert_eq!(DebugInfo::parse(&bytes, 2), Err(DebugError::BadMagic));
    }

    #[test]
    fn parse_rejects_a_newer_major() {
        let mut bytes = info().encode(2);
        // The major is the top five bits of the packed `u16`.
        bytes[5] = 0x10;
        assert!(matches!(
            DebugInfo::parse(&bytes, 2),
            Err(DebugError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn parse_rejects_a_segment_that_is_too_long() {
        let mut bytes = info().encode(2);
        bytes.push(0);
        assert!(matches!(
            DebugInfo::parse(&bytes, 2),
            Err(DebugError::LineMapLength { .. })
        ));
    }

    #[test]
    fn parse_rejects_a_truncated_segment() {
        let bytes = info().encode(2);
        assert_eq!(
            DebugInfo::parse(&bytes[..10], 2),
            Err(DebugError::Truncated)
        );
        assert!(matches!(
            DebugInfo::parse(&bytes[..bytes.len() - 1], 2),
            Err(DebugError::LineMapLength { .. })
        ));
    }
}
