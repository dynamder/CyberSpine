use core::fmt;

use crate::Site;

/// A 1-based source position, as an editor counts it.
///
/// `usize` because that is what a source index already is: the assembler looks a
/// location back up in the text it came from, and a second integer width between
/// the two would only be a conversion to get wrong. It becomes a [`Site::Source`]
/// at the diagnostic boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

impl From<Location> for Site {
    fn from(location: Location) -> Self {
        Self::Source {
            line: u32::try_from(location.line).unwrap_or(u32::MAX),
            column: u32::try_from(location.column).unwrap_or(u32::MAX),
        }
    }
}
