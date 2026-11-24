//! Where the repository is and how a command is run in it.
//!
//! Everything here is a fact about the checkout, not about `xtask`: the root is
//! `CARGO_MANIFEST_DIR`'s parent, the target directory honours
//! `CARGO_TARGET_DIR`, and a command is `cargo` as the running cargo named it.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives beside the workspace root")
        .to_path_buf()
}

pub fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
}

pub fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// Run a command to completion and report a non-zero exit as an error.
pub fn run(mut command: Command, what: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("{what}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} exited with {status}"))
    }
}
