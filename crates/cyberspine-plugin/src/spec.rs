//! The grammar shared by every plugin spec.
//!
//! A linked plugin is not a spec: it is compiled into the binary that drives it.
//! Only the packagings that cross a boundary at run time need a spelling, and
//! both of them spell it the same way:
//!
//! ```text
//! dylib:<path>              a cdylib exporting the plugin's entry symbol
//! rpc:<host>:<port>         a plugin serving the wire protocol over TCP
//! ```
//!
//! The words around the grammar — the noun, the example, the name of the built-in
//! and how to select it — are the caller's, so the error still reads like it
//! belongs to the thing the user named.

/// Which packaging a spec names, with its target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A dynamic library at this path.
    Dylib(String),
    /// A TCP endpoint at this address.
    Rpc(String),
}

/// The words one plugin puts around the shared grammar.
#[derive(Debug, Clone, Copy)]
pub struct Spelling {
    /// What the plugin is called: "backend", "arbiter".
    pub noun: &'static str,
    /// The same word with its article, for the shape error: "a backend",
    /// "an arbiter".
    pub subject: &'static str,
    /// One legal spelling, for the shape error.
    pub example: &'static str,
    /// The kind that names the built-in plugin.
    pub builtin: &'static str,
    /// What to tell a user who wrote the built-in kind.
    pub builtin_hint: &'static str,
    /// The kinds this plugin accepts, for the unknown-kind error.
    pub expected: &'static str,
}

/// Why a spec string is not one of the packaging forms.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    /// There is no colon, so there is no kind and target.
    #[error("{subject} is written `kind:target`, for example `{example}`")]
    Shape {
        /// The noun with its article ("a backend", "an arbiter").
        subject: &'static str,
        /// One legal spelling.
        example: &'static str,
    },
    /// There is a kind but no target.
    #[error("`{kind}` needs a target after the colon")]
    Empty {
        /// The kind the user wrote.
        kind: String,
    },
    /// The user named the built-in plugin, which has no spec.
    #[error("{hint}")]
    Builtin {
        /// How to select the built-in instead.
        hint: &'static str,
    },
    /// The kind is not one this plugin ships.
    #[error("unknown {noun} kind `{kind}`: expected {expected}")]
    Kind {
        /// The plugin's noun ("backend", "arbiter").
        noun: &'static str,
        /// The kind the user wrote.
        kind: String,
        /// The kinds this plugin accepts.
        expected: &'static str,
    },
}

/// Parse `text` under `spelling`.
///
/// A URL-shaped target is accepted too: `rpc://host:port` and `rpc:host:port`
/// name the same endpoint, and dropping the slashes here keeps the two spellings
/// from becoming two grammars.
pub fn parse(text: &str, spelling: &Spelling) -> Result<Target, SpecError> {
    let (kind, target) = text.split_once(':').ok_or(SpecError::Shape {
        subject: spelling.subject,
        example: spelling.example,
    })?;
    if kind == spelling.builtin {
        return Err(SpecError::Builtin {
            hint: spelling.builtin_hint,
        });
    }

    let target = target.strip_prefix("//").unwrap_or(target);
    if target.is_empty() {
        return Err(SpecError::Empty {
            kind: kind.to_owned(),
        });
    }

    match kind {
        "dylib" => Ok(Target::Dylib(target.to_owned())),
        "rpc" => Ok(Target::Rpc(target.to_owned())),
        other => Err(SpecError::Kind {
            noun: spelling.noun,
            kind: other.to_owned(),
            expected: spelling.expected,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{SpecError, Spelling, Target, parse};

    const SPELLING: Spelling = Spelling {
        noun: "body",
        subject: "a body",
        example: "rpc:127.0.0.1:7777",
        builtin: "dummy",
        builtin_hint: "the dummy body is built in",
        expected: "`dylib` or `rpc`",
    };

    #[test]
    fn a_spec_is_a_kind_and_a_target() {
        assert_eq!(
            parse("dylib:bodies/maze.dll", &SPELLING).expect("a library"),
            Target::Dylib("bodies/maze.dll".into())
        );
        assert_eq!(
            parse("rpc:127.0.0.1:7777", &SPELLING).expect("an address"),
            Target::Rpc("127.0.0.1:7777".into())
        );
        assert_eq!(
            parse("rpc://127.0.0.1:7777", &SPELLING).expect("a URL-shaped address"),
            Target::Rpc("127.0.0.1:7777".into())
        );
    }

    #[test]
    fn a_spec_that_names_nothing_is_refused() {
        assert!(matches!(
            parse("rpc", &SPELLING),
            Err(SpecError::Shape { .. })
        ));
        assert!(matches!(
            parse("rpc:", &SPELLING),
            Err(SpecError::Empty { .. })
        ));
        assert!(matches!(
            parse("bluetooth:x", &SPELLING),
            Err(SpecError::Kind { kind, .. }) if kind == "bluetooth"
        ));
    }

    #[test]
    fn the_builtin_is_not_a_spec() {
        assert!(matches!(
            parse("dummy:rigs/maze.rig.json", &SPELLING),
            Err(SpecError::Builtin { .. })
        ));
    }
}
