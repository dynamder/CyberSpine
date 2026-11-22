use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::{Anchor, Code, Coded, Label, Level};

/// One thing a producer noticed, with where it happened and what it was about.
///
/// Built by the crate that detected the problem and pushed into a
/// [`crate::Diagnostics`] sink. It carries no rendering: the same value becomes a
/// caret block on a terminal and a JSON object on a pipe, and only the report
/// layer decides which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    /// The stable rule identifier. This is the contract; `message` is not.
    pub code: Code,
    /// A sentence for a human or a model, in the producer's own words.
    pub message: String,
    /// Where the diagnostic points, if the producer knows a place.
    pub primary: Option<Anchor>,
    /// Secondary anchors, possibly in other artifacts.
    pub labels: Vec<Label>,
    /// Extra context: counts, expected values, "declared here".
    pub notes: Vec<String>,
    /// One actionable suggestion, rendered as `help:`.
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(level: Level, code: Code, message: impl Into<String>) -> Self {
        Self {
            level,
            code,
            message: message.into(),
            primary: None,
            labels: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn error(code: Code, message: impl Into<String>) -> Self {
        Self::new(Level::Error, code, message)
    }

    pub fn warning(code: Code, message: impl Into<String>) -> Self {
        Self::new(Level::Warning, code, message)
    }

    /// Build from an error enum that knows its own code and level.
    ///
    /// The place is attached separately: only the caller holding the `Location`
    /// or the `pc` can build an [`Anchor`], and guessing it here would be the
    /// bug this whole redesign removes.
    pub fn of<T: Coded + ?Sized>(kind: &T) -> Self {
        Self {
            level: T::LEVEL,
            code: kind.code(),
            message: kind.to_string(),
            primary: None,
            labels: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn at(mut self, anchor: Anchor) -> Self {
        self.primary = Some(anchor);
        self
    }

    pub fn label(mut self, anchor: Anchor, message: impl Into<String>) -> Self {
        self.labels.push(Label::new(anchor, message));
        self
    }

    pub fn note(mut self, text: impl Into<String>) -> Self {
        self.notes.push(text.into());
        self
    }

    pub fn with_help(mut self, text: impl Into<String>) -> Self {
        self.help = Some(text.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.level.is_error()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]: {}", self.level, self.code, self.message)?;
        if let Some(anchor) = &self.primary {
            write!(f, " ({anchor})")?;
        }
        for label in &self.labels {
            write!(f, "\n  {}: {}", label.anchor, label.message)?;
        }
        for note in &self.notes {
            write!(f, "\n  note: {note}")?;
        }
        if let Some(help) = &self.help {
            write!(f, "\n  help: {help}")?;
        }
        Ok(())
    }
}
