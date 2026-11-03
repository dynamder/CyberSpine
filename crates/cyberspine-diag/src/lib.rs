//! The vocabulary errors and warnings share.
//!
//! An error and a warning are the same kind of thing to the caller — a diagnostic
//! with a severity, a rule identifier and a place — and only the decision differs:
//! an error stops the operation, a warning does not. This crate holds the shape
//! and nothing else, so the format loader, the assembler and the host can all
//! speak it without any of them depending on another's rendering.
//!
//! It is `#![no_std]` with only `alloc`: the loader uses it on a microcontroller,
//! where rendering is somebody else's business.
//!
//! Rendering is deliberately absent. A source-level diagnostic wants a line, a
//! caret and colour, which needs a text renderer and a terminal; a byte-level one
//! wants an offset. Each crate renders its own, and this one only names them.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// How much a diagnostic matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Reported, but the operation continues.
    Warning,
    /// The operation stopped.
    Error,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

/// A place in a source text, one-based, as an editor counts.
///
/// `usize` because that is what a source index already is: the assembler looks a
/// location back up in the text it came from, and a second integer width between
/// the two would only be a conversion to get wrong.
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

/// Something a caller can report, and a tool can filter by rule.
///
/// `Display` is the human half: it has to read as a sentence. `code` is the
/// machine half: a stable identifier from the specification the rule lives in
/// (`"C5"`, `"M4"`, `"I7"`), so "which rule rejected this" is answerable without
/// matching on English.
pub trait DiagnosticKind: fmt::Display {
    fn level(&self) -> Level;

    /// The specification's identifier for the rule, when there is one.
    ///
    /// Empty for a kind that is not in a numbered checklist.
    fn code(&self) -> &'static str {
        ""
    }
}

/// One diagnostic, detached from the type that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    pub code: &'static str,
    pub message: String,
    pub location: Option<Location>,
}

impl Diagnostic {
    pub fn of(kind: &impl DiagnosticKind, location: Option<Location>) -> Self {
        Self {
            level: kind.level(),
            code: kind.code(),
            message: alloc::format!("{kind}"),
            location,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = self.location {
            write!(f, "{location}: ")?;
        }
        write!(f, "{}", self.level)?;
        if !self.code.is_empty() {
            write!(f, " [{}]", self.code)?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A sink for everything one operation noticed.
///
/// A warning never travels alone: the operation it came from also has a `Result`,
/// and a warning does not make that `Result` `Ok`. The two channels are separate
/// because they answer different questions — "may I proceed" and "what should I
/// know" — and because a run can fail for a reason unrelated to the warning it
/// emitted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
    deny_warnings: bool,
}

impl Diagnostics {
    pub fn new(deny_warnings: bool) -> Self {
        Self {
            items: Vec::new(),
            deny_warnings,
        }
    }

    pub fn push(&mut self, kind: &impl DiagnosticKind, location: Option<Location>) {
        self.items.push(Diagnostic::of(kind, location));
    }

    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    pub fn into_items(self) -> Vec<Diagnostic> {
        self.items
    }

    pub fn has_warnings(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.level == Level::Warning)
    }

    /// Whether warnings are treated as fatal by this run's policy.
    pub fn denies_warnings(&self) -> bool {
        self.deny_warnings
    }

    /// Whether the caller should stop: a warning under `--deny-warnings`, or an
    /// error. The diagnostics themselves are the caller's to render.
    pub fn failed(&self) -> bool {
        self.items.iter().any(|item| item.level == Level::Error)
            || (self.deny_warnings && self.has_warnings())
    }
}
