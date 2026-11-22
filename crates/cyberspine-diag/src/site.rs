use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;

/// A coordinate in one artifact's system.
///
/// The variants are the systems that actually exist, not a union kept for
/// symmetry: an instruction index is not a byte offset, and a session position is
/// not either of them. Which variant is primary is also the exit-code classifier
/// (`report` maps `Source`/`Byte` to "bad input", `Instruction` to "bad run",
/// `Session`/`Whole` to "environment").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Site {
    /// A line and column in a text file, one-based.
    Source { line: u32, column: u32 },
    /// A byte offset into an image.
    Byte { offset: u64 },
    /// An instruction index in an image.
    Instruction { pc: u32 },
    /// A position in a live run.
    Session { instance: u32, tick: u64 },
    /// The artifact as a whole: a manifest that will not parse, a file that
    /// cannot be written. There is a file, but no place inside it.
    Whole,
}

impl Site {
    pub const fn is_place(self) -> bool {
        !matches!(self, Self::Whole)
    }
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source { line, column } => write!(f, "{line}:{column}"),
            Self::Byte { offset } => write!(f, "byte {offset}"),
            Self::Instruction { pc } => write!(f, "instruction {pc}"),
            Self::Session { instance, tick } => {
                write!(f, "instance {instance} at tick {tick}")
            }
            Self::Whole => Ok(()),
        }
    }
}

/// A [`Site`] plus the artifact it is a position in.
///
/// The artifact is always a path the reader can open, resolved by the producer:
/// the debug segment stores the source path *relative to the image*, and the one
/// place that knows the image's directory is the one that builds the anchor. A
/// relative path in a rendered message is a path that only works from one
/// directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Anchor {
    pub artifact: Box<str>,
    pub site: Site,
}

impl Anchor {
    pub fn new(artifact: impl Into<Box<str>>, site: Site) -> Self {
        Self {
            artifact: artifact.into(),
            site,
        }
    }

    pub fn at_source(artifact: impl Into<Box<str>>, line: u32, column: u32) -> Self {
        Self::new(artifact, Site::Source { line, column })
    }

    pub fn whole(artifact: impl Into<Box<str>>) -> Self {
        Self::new(artifact, Site::Whole)
    }
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.site {
            Site::Whole => f.write_str(&self.artifact),
            _ => write!(f, "{}:{}", self.artifact, self.site),
        }
    }
}

/// A secondary anchor: "first defined here", "the instruction came from here".
///
/// A label is allowed to be in another artifact than the diagnostic's primary —
/// that is what a source map *is* — so it carries a whole [`Anchor`] rather than
/// an offset that assumes the primary's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub anchor: Anchor,
    pub message: String,
}

impl Label {
    pub fn new(anchor: Anchor, message: impl Into<String>) -> Self {
        Self {
            anchor,
            message: message.into(),
        }
    }
}
