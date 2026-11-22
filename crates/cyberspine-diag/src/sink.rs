use alloc::vec::Vec;

use crate::{Coded, Diagnostic};

/// Everything one operation noticed, in the order it noticed it.
///
/// A warning never travels alone: the operation it came from also returns a
/// value or an error, and "may I proceed" is not the same question as "what
/// should I know". This sink answers both from one place — [`Diagnostics::failed`]
/// is the only decision point, and `--deny-warnings` is one constructor argument
/// rather than a second code path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
    deny_warnings: bool,
    limit: usize,
    dropped: usize,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Diagnostics {
    /// Enough to show the shape of a broken source without letting a runaway
    /// generator turn into an unbounded allocation.
    pub const DEFAULT_LIMIT: usize = 64;

    pub fn new(deny_warnings: bool) -> Self {
        Self {
            items: Vec::new(),
            deny_warnings,
            limit: Self::DEFAULT_LIMIT,
            dropped: 0,
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        if self.items.len() >= self.limit {
            self.dropped += 1;
            return;
        }
        self.items.push(diagnostic);
    }

    /// Push a diagnostic that has no place of its own yet.
    pub fn push_of(&mut self, kind: &impl Coded) {
        self.push(Diagnostic::of(kind));
    }

    pub fn push_all(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        for diagnostic in diagnostics {
            self.push(diagnostic);
        }
    }

    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    pub fn into_items(self) -> Vec<Diagnostic> {
        self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many diagnostics the limit refused, so a renderer can say so instead
    /// of pretending the list is complete.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn deny_warnings(&self) -> bool {
        self.deny_warnings
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    pub fn has_warnings(&self) -> bool {
        self.items.iter().any(|item| item.level.is_warning())
    }

    /// Whether the caller should stop.
    pub fn failed(&self) -> bool {
        self.has_errors() || (self.deny_warnings && self.has_warnings())
    }
}
