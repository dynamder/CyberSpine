//! Names that identify table entries (`docs/binfmt.md` §6.1, `docs/rig.md` §2).
//!
//! A [`Name`] is an ASCII identifier: non-empty, `[A-Za-z_][A-Za-z0-9_]*`. That
//! grammar is the whole of §9.2 M1, and it also implies "no NUL", which is what
//! makes the image writer total instead of having to assert.
//!
//! The reserved spellings (`dt`, `tick`, `f0`..`i63`) are *not* rejected here:
//! an image legitimately contains the built-in sensors under those names. They
//! are refused where a body's vocabulary is declared ([`Rig::validate`]).

use alloc::boxed::Box;
use core::fmt;
use core::ops::Deref;
use core::str::FromStr;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NameError {
    #[error("a name must be at least one character")]
    Empty,
    #[error("a name must be an ASCII identifier ([A-Za-z_][A-Za-z0-9_]*)")]
    NotAnIdentifier,
}

/// A validated table-entry name.
///
/// The field is private and the only safe constructor validates, so a `Name`
/// that exists is a name the image format accepts.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(Box<str>);

impl Name {
    pub fn new(text: &str) -> Result<Self, NameError> {
        let mut chars = text.chars();
        match chars.next() {
            None => return Err(NameError::Empty),
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
            Some(_) => return Err(NameError::NotAnIdentifier),
        }
        if !chars.all(|character| character.is_ascii_alphanumeric() || character == '_') {
            return Err(NameError::NotAnIdentifier);
        }
        Ok(Self(text.into()))
    }

    /// Build a name without validating it.
    ///
    /// Only for tests that have to hand the loader a file no legal producer can
    /// write (`docs/binfmt.md` §9.2 M1). Nothing in this workspace's runtime
    /// path may call it.
    #[doc(hidden)]
    pub fn new_unchecked(text: &str) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Name {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::new(text)
    }
}

impl From<Name> for Box<str> {
    fn from(name: Name) -> Self {
        name.0
    }
}

/// `dt`/`tick` and the register spellings `f0`..`i63` are the language's, so no
/// declared name may use them (`docs/asm.md` §1, `docs/rig.md` §2).
///
/// Lives here rather than in the assembler because the rig and the source are
/// two declarations of the same vocabulary and must agree on the rule.
pub fn is_reserved(name: &str) -> bool {
    matches!(name, "dt" | "tick") || is_register_spelling(name)
}

fn is_register_spelling(name: &str) -> bool {
    let mut chars = name.chars();
    if !matches!(chars.next(), Some('f' | 'i')) {
        return false;
    }
    let mut rest = chars.peekable();
    if rest.peek().is_none() {
        return false;
    }
    rest.all(|character| character.is_ascii_digit())
}
