//! Choosing an external body from a string.
//!
//! A linked body is not a spec: it is compiled into the binary that drives it.
//! Only the two bodies that cross a boundary at run time need a spelling, and
//! nothing here belongs to the CLI: parsing and construction live in the library
//! so a production caller can use the same spelling in a config file.
//!
//! ```text
//! dylib:<path>              a cdylib exporting `csp_backend_v1`
//! rpc:<host>:<port>         a body serving the wire protocol over TCP
//! ```

use std::path::{Path, PathBuf};

use cyberspine_vm::{Backend, BackendError};

/// An external body, described without constructing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSpec {
    Dylib(PathBuf),
    Rpc(String),
}

/// Why a spec string is not one of the two forms.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("a backend is written `kind:target`, for example `rpc:127.0.0.1:7777`")]
    Shape,
    #[error("`{kind}` needs a target after the colon")]
    Empty { kind: String },
    #[error("the dummy body is built in; omit `--backend` to use it")]
    BuiltinDummy,
    #[error("unknown backend kind `{0}`: expected `dylib` or `rpc`")]
    Kind(String),
}

impl BackendSpec {
    pub fn parse(text: &str) -> Result<Self, SpecError> {
        let (kind, target) = text.split_once(':').ok_or(SpecError::Shape)?;
        if kind == "dummy" {
            return Err(SpecError::BuiltinDummy);
        }

        let target = target.strip_prefix("//").unwrap_or(target);
        if target.is_empty() {
            return Err(SpecError::Empty {
                kind: kind.to_owned(),
            });
        }

        Ok(match kind {
            "dylib" => Self::Dylib(target.into()),
            "rpc" => Self::Rpc(target.to_owned()),
            other => return Err(SpecError::Kind(other.to_owned())),
        })
    }

    /// The body this spec names, boxed so the caller stays non-generic.
    ///
    /// The handshake runs here, which is the point: a body that cannot introduce
    /// itself fails at startup instead of in the middle of a tick.
    pub fn open(&self) -> Result<Box<dyn Backend>, BackendError> {
        match self {
            Self::Dylib(path) => dylib(path),
            Self::Rpc(address) => rpc(address),
        }
    }
}

#[cfg(feature = "dylib")]
fn dylib(path: &Path) -> Result<Box<dyn Backend>, BackendError> {
    Ok(Box::new(crate::dylib::open(path)?))
}

#[cfg(not(feature = "dylib"))]
fn dylib(_path: &Path) -> Result<Box<dyn Backend>, BackendError> {
    Err(BackendError::message(
        "this build has no `dylib` feature, so it cannot load a shared library",
    ))
}

#[cfg(feature = "rpc")]
fn rpc(address: &str) -> Result<Box<dyn Backend>, BackendError> {
    Ok(Box::new(crate::rpc::connect(address)?))
}

#[cfg(not(feature = "rpc"))]
fn rpc(_address: &str) -> Result<Box<dyn Backend>, BackendError> {
    Err(BackendError::message(
        "this build has no `rpc` feature, so it cannot open a connection",
    ))
}
