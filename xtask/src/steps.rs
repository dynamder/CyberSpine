//! The gate: what runs, in what order, and what counts as passing.
//!
//! `STEPS` is the list a pull request has to satisfy, and the CI workflow keeps
//! its own copy of the names on purpose: deleting a step here must not be able
//! to turn a red run green without someone noticing.

use std::time::Instant;

use crate::docs;
use crate::report::{Report, Step, run_logged, write_log};
use crate::workspace::{cargo, root};

pub enum Plan {
    Cargo(&'static [&'static str]),
    Docs,
}

pub const STEPS: [(&str, Plan); 9] = [
    ("fmt", Plan::Cargo(&["fmt", "--all", "--check"])),
    (
        "clippy",
        Plan::Cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--no-deps",
            "--",
            "-D",
            "warnings",
        ]),
    ),
    (
        "nostd",
        Plan::Cargo(&[
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
        ]),
    ),
    (
        "runtime-no-default",
        Plan::Cargo(&[
            "clippy",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--all-targets",
            "--no-deps",
            "--",
            "-D",
            "warnings",
        ]),
    ),
    (
        "runtime-dylib",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--features",
            "dylib",
        ]),
    ),
    (
        "runtime-rpc",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--features",
            "rpc",
        ]),
    ),
    (
        "backend-export",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-backend",
            "--no-default-features",
            "--features",
            "export",
        ]),
    ),
    (
        "arbiter-export",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-arbiter",
            "--no-default-features",
            "--features",
            "export",
        ]),
    ),
    ("docs", Plan::Docs),
];

pub fn check(json: bool) -> Result<(), String> {
    let mut report = Report::new(json);
    lint(&mut report);
    conclude(&report, json)
}

pub fn ci(json: bool) -> Result<(), String> {
    let mut report = Report::new(json);
    lint(&mut report);
    tests(&[], &mut report);
    conclude(&report, json)
}

pub fn test(args: &[String], json: bool) -> Result<(), String> {
    let mut report = Report::new(json);
    tests(args, &mut report);
    conclude(&report, json)
}

pub fn docs_only() -> Result<(), String> {
    let mut report = Report::new(false);
    run_docs("docs", &mut report);
    if report.ok() {
        Ok(())
    } else {
        Err(format!("{} step(s) failed", report.failed().len()))
    }
}

fn lint(report: &mut Report) {
    for (name, plan) in STEPS {
        match plan {
            Plan::Cargo(args) => {
                run_logged(name, cargo_step(args), report);
            }
            Plan::Docs => run_docs(name, report),
        }
    }
}

fn tests(args: &[String], report: &mut Report) {
    // The test build compiles every workspace member that a test depends on,
    // and the backend, arbiter and panic fixtures are dev-dependencies, so their
    // shared libraries come out of it. The demo's library is a `cdylib` only
    // and nothing depends on it; it has to be built here, and building it does
    // not invalidate the test build.
    let mut library = cargo();
    library.args(["build", "--locked", "-p", "cyberspine-demo-maze-dylib"]);
    if !run_logged("fixtures", library, report) {
        return;
    }

    let has_nextest = cargo()
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());

    let mut runner = cargo();
    if has_nextest {
        runner.args(["nextest", "run", "--locked", "--workspace", "--all-targets"]);
    } else {
        eprintln!("tests: cargo test (install cargo-nextest for parallel binaries)");
        runner.args(["test", "--locked", "--workspace", "--all-targets"]);
    }
    runner.args(args);
    if !run_logged("tests", runner, report) {
        return;
    }

    // nextest runs test binaries only; a doc example is still a test.
    let mut doctests = cargo();
    doctests.args(["test", "--locked", "--workspace", "--doc"]);
    run_logged("doctests", doctests, report);
}

fn cargo_step(args: &'static [&'static str]) -> std::process::Command {
    let mut command = cargo();
    command.arg(args[0]);
    if args[0] != "fmt" {
        command.arg("--locked");
    }
    command.args(&args[1..]);
    command
}

fn run_docs(name: &'static str, report: &mut Report) {
    let started = Instant::now();
    let root = root();
    let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
    let problems = docs::check(&root);
    let millis = started.elapsed().as_millis();

    let mut text = String::new();
    for problem in &problems {
        let line = problem.replace(&prefix, "");
        eprintln!("{line}");
        text.push_str(&line);
        text.push('\n');
    }

    report.record(Step {
        name,
        ok: problems.is_empty(),
        code: None,
        millis,
        log: write_log(name, &text).ok(),
    });
}

fn conclude(report: &Report, json: bool) -> Result<(), String> {
    report.write(&root())?;
    if json {
        println!("{}", report.json(&root()));
    }
    let failed = report.failed();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join(", "))
    }
}
