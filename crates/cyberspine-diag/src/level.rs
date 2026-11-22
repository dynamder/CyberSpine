use core::fmt;

/// How much a diagnostic matters.
///
/// Ordered so that combining two of them is `max`: a sink that has seen only
/// notes is quiet, and one that has seen an error stops the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Context for another diagnostic; never printed on its own.
    Note,
    /// Reported, but the operation continues.
    Warning,
    /// The operation stopped.
    Error,
}

impl Level {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    pub const fn is_error(self) -> bool {
        matches!(self, Self::Error)
    }

    pub const fn is_warning(self) -> bool {
        matches!(self, Self::Warning)
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
