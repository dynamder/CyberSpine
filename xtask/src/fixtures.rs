//! The shared libraries the tests load.
//!
//! A fixture is a workspace member, so `cargo build` produces it; the tests only
//! look for it. Building them here is what keeps "the tests do not build their
//! own fixtures" true.

use std::path::PathBuf;

use crate::workspace::{cargo, run, target_dir};

pub const FIXTURES: [&str; 4] = [
    "cyberspine-backend-fixture",
    "cyberspine-arbiter-fixture",
    "cyberspine-panic-fixture",
    "cyberspine-demo-maze-dylib",
];

pub fn fixtures(args: &[String]) -> Result<(), String> {
    let mut build = cargo();
    build.arg("build").arg("--locked");
    for package in FIXTURES {
        build.arg("-p").arg(package);
    }
    build.args(args);
    run(build, "building the fixtures")?;

    let profile = profile(args);
    for package in FIXTURES {
        let path = fixture_path(package, profile);
        if !path.exists() {
            return Err(format!("cargo built no {} at {}", package, path.display()));
        }
        println!("{package} -> {}", path.display());
    }
    Ok(())
}

fn profile(args: &[String]) -> &'static str {
    if args.iter().any(|arg| arg == "--release") {
        "release"
    } else {
        "debug"
    }
}

fn fixture_path(package: &str, profile: &str) -> PathBuf {
    let stem = package.replace('-', "_");
    let name = format!(
        "{}{stem}.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    );
    target_dir().join(profile).join(name)
}
