//! Repository tasks that a single cargo invocation cannot express.
//!
//! This file is the surface: the subcommands, their one-line descriptions, and
//! the dispatch. Everything a subcommand does lives beside it — `doctor`,
//! `docs`, `fixtures`, `steps` — and the two shared pieces are `workspace`
//! (where the checkout is) and `report` (what a step's outcome looks like).

mod docs;
mod doctor;
mod fixtures;
mod report;
mod steps;
mod workspace;

use std::process::ExitCode;

use doctor::doctor;
use fixtures::fixtures;
use steps::{check, ci, docs_only, test};

const SUBCOMMANDS: [(&str, &str); 6] = [
    ("doctor", "check the build environment and worktree hygiene"),
    ("docs", "check the documents against the tree"),
    ("fixtures", "build the shared libraries the tests load"),
    ("test", "build the fixtures, then run the workspace tests"),
    ("check", "run the lint, no_std, feature and document checks"),
    ("ci", "run check, then the workspace tests"),
];

fn usage() -> String {
    let mut text = String::from("usage: cargo xtask <command>\n\n");
    for (name, what) in SUBCOMMANDS {
        text.push_str(&format!("  {name:<10} {what}\n"));
    }
    text.push_str("\n  --json     print the step report as one JSON object\n");
    text
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();

    let json = rest.iter().any(|argument| argument == "--json");
    let rest = rest
        .into_iter()
        .filter(|argument| argument != "--json")
        .collect::<Vec<_>>();

    let outcome = match command.as_deref() {
        Some("doctor") => doctor(),
        Some("docs") => docs_only(),
        Some("fixtures") => fixtures(&rest),
        Some("test") => test(&rest, json),
        Some("check") => check(json),
        Some("ci") => ci(json),
        _ => {
            eprint!("{}", usage());
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
