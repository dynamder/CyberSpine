//! Choosing a body from a string.
//!
//! The dev CLI wants to be told which body to drive without knowing anything
//! about how that body is packaged, and nothing here belongs to the CLI: parsing
//! and construction live in the library so a production caller can use the same
//! spelling in a config file.
//!
//! ```text
//! dummy:<manifest.json>     a zero body, for "what does this image do?"
//! dylib:<path>              a cdylib exporting `csp_backend_v1`
//! rpc:<host>:<port>         a body serving the wire protocol over TCP
//! ```

use std::path::{Path, PathBuf};

use cyberspine_manifest::from_json;
use cyberspine_vm::{Backend, BackendError, DummyBackend};

/// A body, described without constructing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSpec {
    Dummy(PathBuf),
    Dylib(PathBuf),
    Rpc(String),
}

/// Why a spec string is not one of the three forms.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("a backend is written `kind:target`, for example `rpc:127.0.0.1:7777`")]
    Shape,
    #[error("`{kind}` needs a target after the colon")]
    Empty { kind: String },
    #[error("unknown backend kind `{0}`: expected `dummy`, `dylib` or `rpc`")]
    Kind(String),
}

impl BackendSpec {
    pub fn parse(text: &str) -> Result<Self, SpecError> {
        let (kind, target) = text.split_once(':').ok_or(SpecError::Shape)?;
        let target = target.strip_prefix("//").unwrap_or(target);
        if target.is_empty() {
            return Err(SpecError::Empty {
                kind: kind.to_owned(),
            });
        }

        Ok(match kind {
            "dummy" => Self::Dummy(target.into()),
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
            Self::Dummy(path) => dummy(path),
            Self::Dylib(path) => dylib(path),
            Self::Rpc(address) => rpc(address),
        }
    }
}

fn dummy(path: &Path) -> Result<Box<dyn Backend>, BackendError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        BackendError::message(format!("cannot read `{}`: {error}", path.display()))
    })?;
    let rig = from_json(&text).map_err(BackendError::new)?;
    Ok(Box::new(DummyBackend::new(rig)))
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
