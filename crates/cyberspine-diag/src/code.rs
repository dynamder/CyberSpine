use alloc::borrow::Cow;
use alloc::string::String;
use core::fmt;

/// The stable identifier of a rule: `namespace.name`.
///
/// The namespace names the crate (or spec) that owns the rule — `asm`, `binfmt`,
/// `exec`, `rt`, `backend`, `arbiter` — and the second half is either the number
/// the spec already uses (`binfmt.C6`) or a semantic name (`asm.unknown-mnemonic`).
///
/// Codes are the machine contract. Messages are not: a caller may match on a
/// code, never on the sentence next to it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(Cow<'static, str>);

impl Code {
    /// Both checks are compile-time where a table uses them: the constant is
    /// evaluated in a `const` item, so an empty or un-namespaced code does not
    /// build.
    pub const fn new(text: &'static str) -> Self {
        let bytes = text.as_bytes();
        let mut dots = 0usize;
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] == b'.' {
                dots += 1;
            }
            index += 1;
        }

        assert!(!text.is_empty(), "a diagnostic code must not be empty");
        assert!(dots > 0, "a diagnostic code is `namespace.name`");
        Self(Cow::Borrowed(text))
    }

    /// A code that only exists at run time: a plugin's, decoded from the wire.
    ///
    /// The namespace convention still applies, but nothing checks it — the
    /// sender is another process and this side can only report what it said.
    pub fn owned(text: impl Into<String>) -> Self {
        Self(Cow::Owned(text.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the first `.`.
    pub fn namespace(&self) -> &str {
        match self.0.split_once('.') {
            Some((namespace, _)) => namespace,
            None => &self.0,
        }
    }

    /// The part after the first `.`.
    pub fn name(&self) -> &str {
        match self.0.split_once('.') {
            Some((_, name)) => name,
            None => &self.0,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
