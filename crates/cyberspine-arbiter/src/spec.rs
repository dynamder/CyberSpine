//! Choosing an arbiter from a string.
//!
//! A linked arbiter is not a spec: it is compiled into the binary that drives it.
//! Only the two arbiters that cross a boundary at run time need a spelling, and
//! nothing here belongs to the CLI: parsing and construction live in the library
//! so a production caller can use the same spelling in a config file.
//!
//! The grammar is [`cyberspine_plugin::spec`]'s; this module adds the arbiter's
//! words and the step that opens the named packaging.
//!
//! ```text
//! dylib:<path>              a cdylib exporting `csp_arbiter_v1`
//! rpc:<host>:<port>         an arbiter serving the wire protocol over TCP
//! ```
//!
//! The built-in single-round arbiter has no spelling: a host selects it by not
//! naming one at all.

use std::path::{Path, PathBuf};

use cyberspine_plugin::spec::{self, Spelling, Target};

pub use cyberspine_plugin::spec::SpecError;

use crate::{Arbiter, ArbiterError};

const SPELLING: Spelling = Spelling {
    noun: "arbiter",
    subject: "an arbiter",
    example: "rpc:127.0.0.1:7777",
    builtin: "builtin",
    builtin_hint: "the built-in arbiter is selected by omitting `--interrupt`",
    expected: "`dylib` or `rpc`",
};

/// An external arbiter, described without constructing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArbiterSpec {
    /// A dynamic library at this path.
    Dylib(PathBuf),
    /// An arbiter serving the wire protocol at this address.
    Rpc(String),
}

impl ArbiterSpec {
    /// Parse `text` as an arbiter spec.
    pub fn parse(text: &str) -> Result<Self, SpecError> {
        match spec::parse(text, &SPELLING)? {
            Target::Dylib(path) => Ok(Self::Dylib(path.into())),
            Target::Rpc(address) => Ok(Self::Rpc(address)),
        }
    }

    /// The arbiter this spec names, boxed so the caller stays non-generic.
    ///
    /// The handshake runs here, which is the point: an arbiter that cannot
    /// introduce itself fails at startup instead of in the middle of a tick.
    pub fn open(&self) -> Result<Box<dyn Arbiter>, ArbiterError> {
        match self {
            Self::Dylib(path) => dylib(path),
            Self::Rpc(address) => rpc(address),
        }
    }

    /// The shared library this spec names, if it names one.
    ///
    /// A caller that writes to a path wants to know whether it is about to write
    /// over an arbiter it is also reading.
    pub fn dylib_path(&self) -> Option<&Path> {
        match self {
            Self::Dylib(path) => Some(path),
            Self::Rpc(_) => None,
        }
    }
}

#[cfg(feature = "dylib")]
fn dylib(path: &Path) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Ok(Box::new(crate::dylib::open(path)?))
}

#[cfg(not(feature = "dylib"))]
fn dylib(_path: &Path) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Err(ArbiterError::new(
        "this build has no `dylib` feature, so it cannot load a shared library",
    ))
}

#[cfg(feature = "rpc")]
fn rpc(address: &str) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Ok(Box::new(crate::rpc::connect(address)?))
}

#[cfg(not(feature = "rpc"))]
fn rpc(_address: &str) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Err(ArbiterError::new(
        "this build has no `rpc` feature, so it cannot open a connection",
    ))
}
