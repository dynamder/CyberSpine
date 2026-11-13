//! Choosing an arbiter from a string.
//!
//! A linked arbiter is not a spec: it is compiled into the binary that drives it.
//! Only the two arbiters that cross a boundary at run time need a spelling, and
//! nothing here belongs to the CLI: parsing and construction live in the library
//! so a production caller can use the same spelling in a config file.
//!
//! ```text
//! dylib:<path>              a cdylib exporting `csp_arbiter_v1`
//! rpc:<host>:<port>         an arbiter serving the wire protocol over TCP
//! ```
//!
//! The built-in single-round arbiter has no spelling: a host selects it by not
//! naming one at all.

use std::path::{Path, PathBuf};

use crate::{Arbiter, ArbiterError};

/// An external arbiter, described without constructing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArbiterSpec {
    Dylib(PathBuf),
    Rpc(String),
}

/// Why a spec string is not one of the two forms.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("an arbiter is written `kind:target`, for example `rpc:127.0.0.1:7777`")]
    Shape,
    #[error("`{kind}` needs a target after the colon")]
    Empty { kind: String },
    #[error("the built-in arbiter is selected by omitting `--interrupt`")]
    Builtin,
    #[error("unknown arbiter kind `{0}`: expected `dylib` or `rpc`")]
    Kind(String),
}

impl ArbiterSpec {
    pub fn parse(text: &str) -> Result<Self, SpecError> {
        let (kind, target) = text.split_once(':').ok_or(SpecError::Shape)?;
        if kind == "builtin" {
            return Err(SpecError::Builtin);
        }

        let target = target.strip_prefix("//").unwrap_or(target);
        if target.is_empty() {
            return Err(SpecError::Empty {
                kind: kind.to_owned(),
            });
        }

        Ok(match kind {
            "dylib" => Self::Dylib(PathBuf::from(target)),
            "rpc" => Self::Rpc(target.to_owned()),
            other => return Err(SpecError::Kind(other.to_owned())),
        })
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
}

#[cfg(feature = "dylib")]
fn dylib(path: &Path) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Ok(Box::new(crate::dylib::open(path)?))
}

#[cfg(not(feature = "dylib"))]
fn dylib(_path: &Path) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Err(ArbiterError(
        "this build has no `dylib` feature, so it cannot load a shared library".into(),
    ))
}

#[cfg(feature = "rpc")]
fn rpc(address: &str) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Ok(Box::new(crate::rpc::connect(address)?))
}

#[cfg(not(feature = "rpc"))]
fn rpc(_address: &str) -> Result<Box<dyn Arbiter>, ArbiterError> {
    Err(ArbiterError(
        "this build has no `rpc` feature, so it cannot open a connection".into(),
    ))
}
