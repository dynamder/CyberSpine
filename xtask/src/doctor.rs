//! What a checkout needs before any step can be trusted.
//!
//! The doctor does not run the gate; it answers the three questions that make a
//! green gate believable: is the target directory this checkout's, is every
//! worktree that git remembers still on disk, and is `rustc` the one
//! `rust-toolchain.toml` pins.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::workspace::{root, target_dir};

pub fn doctor() -> Result<(), String> {
    let root = root();
    let mut complaints = Vec::new();

    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        let dir = PathBuf::from(dir);
        let absolute = if dir.is_absolute() {
            dir
        } else {
            std::env::current_dir()
                .map_err(|error| format!("current directory: {error}"))?
                .join(dir)
        };
        if !absolute.starts_with(&root) {
            complaints.push(format!(
                "CARGO_TARGET_DIR leaves this checkout: {}\n  \
                 a shared target directory reuses artifacts across source trees, which surfaces \
                 as a compile error no source produces; unset it and let cargo use {}",
                absolute.display(),
                root.join("target").display()
            ));
        }
    }

    for path in worktrees(&root)? {
        if !path.exists() {
            complaints.push(format!(
                "stale worktree: {} (run `git worktree prune`)",
                path.display()
            ));
        }
    }

    if let Some(pinned) = toolchain_channel(&root) {
        let rustc = Command::new("rustc")
            .arg("--version")
            .output()
            .map_err(|error| format!("rustc: {error}"))?;
        let line = String::from_utf8_lossy(&rustc.stdout).into_owned();
        let actual = line.split_whitespace().nth(1).unwrap_or_default();
        if actual != pinned {
            complaints.push(format!(
                "rustc is {actual}, rust-toolchain.toml pins {pinned}"
            ));
        }
    }

    if complaints.is_empty() {
        println!("doctor: ok");
        println!("  root    {}", root.display());
        println!("  target  {}", target_dir().display());
        Ok(())
    } else {
        for complaint in &complaints {
            eprintln!("doctor: {complaint}");
        }
        Err(format!("{} problem(s)", complaints.len()))
    }
}

fn worktrees(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|error| format!("git worktree list: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect())
}

fn toolchain_channel(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("rust-toolchain.toml")).ok()?;
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("channel")?.trim();
        let value = value.strip_prefix('=')?.trim();
        Some(value.trim_matches('"').to_owned())
    })
}
