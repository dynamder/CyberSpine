//! Repository tasks that a single cargo invocation cannot express.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const FIXTURES: [&str; 4] = [
    "cyberspine-backend-fixture",
    "cyberspine-arbiter-fixture",
    "cyberspine-panic-fixture",
    "cyberspine-demo-maze-dylib",
];

const STEPS: [(&str, &[&str]); 8] = [
    ("fmt", &["fmt", "--all", "--check"]),
    (
        "clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--no-deps",
            "--",
            "-D",
            "warnings",
        ],
    ),
    (
        "nostd",
        &[
            "check",
            "-p",
            "cyberspine-isa",
            "-p",
            "cyberspine-rig",
            "-p",
            "cyberspine-binfmt",
            "-p",
            "cyberspine-exec",
            "-p",
            "cyberspine-core",
            "--target",
            "thumbv7em-none-eabihf",
        ],
    ),
    (
        "runtime-no-default",
        &[
            "clippy",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--all-targets",
            "--no-deps",
            "--",
            "-D",
            "warnings",
        ],
    ),
    (
        "runtime-dylib",
        &[
            "check",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--features",
            "dylib",
        ],
    ),
    (
        "runtime-rpc",
        &[
            "check",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--features",
            "rpc",
        ],
    ),
    (
        "backend-export",
        &[
            "check",
            "-p",
            "cyberspine-backend",
            "--no-default-features",
            "--features",
            "export",
        ],
    ),
    (
        "arbiter-export",
        &[
            "check",
            "-p",
            "cyberspine-arbiter",
            "--no-default-features",
            "--features",
            "export",
        ],
    ),
];

const USAGE: &str = "\
usage: cargo xtask <command>

  doctor     check the build environment and worktree hygiene
  fixtures   build the shared libraries the tests load
  test       build the fixtures, then run the workspace tests
  check      run the lint, no_std and feature checks
  ci         run check, then the workspace tests";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();

    let outcome = match command.as_deref() {
        Some("doctor") => doctor(),
        Some("fixtures") => fixtures(&rest),
        Some("test") => tests(&rest),
        Some("check") => check(),
        Some("ci") => ci(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives beside the workspace root")
        .to_path_buf()
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
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

fn run(mut command: Command, what: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("{what}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} exited with {status}"))
    }
}

fn doctor() -> Result<(), String> {
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

fn fixtures(args: &[String]) -> Result<(), String> {
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

fn tests(args: &[String]) -> Result<(), String> {
    // The test build compiles every workspace member that a test depends on,
    // and the backend and arbiter fixtures are dev-dependencies, so their
    // shared libraries come out of it. The demo's library is a `cdylib` only
    // and nothing depends on it; it has to be built here, and building it does
    // not invalidate the test build.
    let mut library = cargo();
    library.args(["build", "--locked", "-p", "cyberspine-demo-maze-dylib"]);
    run(library, "building the demo library")?;

    let has_nextest = cargo()
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());

    if has_nextest {
        println!("tests: nextest");
        let mut nextest = cargo();
        nextest.args(["nextest", "run", "--locked", "--workspace", "--all-targets"]);
        nextest.args(args);
        run(nextest, "workspace tests")?;
    } else {
        println!("tests: cargo test (install cargo-nextest for parallel binaries)");
        let mut test = cargo();
        test.args(["test", "--locked", "--workspace", "--all-targets"]);
        test.args(args);
        run(test, "workspace tests")?;
    }

    // nextest runs test binaries only; a doc example is still a test.
    let mut doctests = cargo();
    doctests.args(["test", "--locked", "--workspace", "--doc"]);
    run(doctests, "doctests")
}

fn cargo_step(args: &[&str]) -> Command {
    let mut command = cargo();
    command.arg(args[0]);
    if args[0] != "fmt" {
        command.arg("--locked");
    }
    command.args(&args[1..]);
    command
}

fn steps() -> Vec<&'static str> {
    let mut failed = Vec::new();
    for (name, args) in STEPS {
        let command = cargo_step(args);
        match run(command, name) {
            Ok(()) => println!("PASS {name}"),
            Err(error) => {
                eprintln!("FAIL {name}: {error}");
                failed.push(name);
            }
        }
    }
    failed
}

fn check() -> Result<(), String> {
    finish(steps())
}

fn ci() -> Result<(), String> {
    let mut failed = steps();
    match tests(&[]) {
        Ok(()) => println!("PASS tests"),
        Err(error) => {
            eprintln!("FAIL tests: {error}");
            failed.push("tests");
        }
    }
    finish(failed)
}

fn finish(failed: Vec<&'static str>) -> Result<(), String> {
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join(", "))
    }
}
