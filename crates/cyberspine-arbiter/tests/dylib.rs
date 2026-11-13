//! End to end through a real shared library.
//!
//! The fixture crate is built on demand: Cargo does not build another package's
//! `cdylib` for a test, so this test asks it to. The artifact path comes from
//! `--message-format=json`, which keeps the test independent of the target
//! directory, the profile and the platform's library suffix.

#![cfg(feature = "dylib")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use cyberspine_arbiter::{Arbiter, Completion, EventId, InterruptEvent, Verdict, dylib};

fn fixture() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();

    PATH.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "cyberspine-arbiter-fixture",
                "--message-format=json",
                "--offline",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");

        assert!(
            output.status.success(),
            "cargo could not build the fixture:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if record["reason"] != "compiler-artifact" {
                continue;
            }
            let package = record["package_id"].as_str().unwrap_or_default();
            if !package.contains("cyberspine-arbiter-fixture") {
                continue;
            }
            for file in record["filenames"].as_array().into_iter().flatten() {
                let path = PathBuf::from(file.as_str().unwrap_or_default());
                if path.extension().and_then(|ext| ext.to_str())
                    == Some(std::env::consts::DLL_EXTENSION)
                {
                    return path;
                }
            }
        }

        panic!("cargo did not report a shared library for the fixture");
    })
}

fn event() -> InterruptEvent {
    InterruptEvent {
        event: EventId::new(7),
        pc: 3,
        tick: 12,
        format: "x={f1}".into(),
        reason: "x=1.5".into(),
        action: "actions/idle.csp".into(),
        instances: Vec::new(),
        actions: Vec::new(),
        image_hash: [0; 32],
    }
}

#[test]
fn two_rounds_cross_the_library_boundary() {
    let mut arbiter = dylib::open(fixture()).expect("the fixture loads");
    assert_eq!(arbiter.build_id(), "cyberspine-arbiter-fixture/0.1.0");

    arbiter.deliver(event());

    let first = arbiter.take(0);
    assert!(first.failure.is_none(), "{:?}", first.failure);
    assert_eq!(first.decisions.len(), 1);
    assert_eq!(first.decisions[0].event, EventId::new(7));
    assert!(matches!(
        first.decisions[0].ruling.completion,
        Completion::Unfinished
    ));

    let second = arbiter.take(1);
    assert!(second.failure.is_none(), "{:?}", second.failure);
    assert_eq!(second.decisions.len(), 1);
    assert!(matches!(
        second.decisions[0].ruling.completion,
        Completion::Finished(Verdict::Continue)
    ));
}

#[test]
fn a_missing_library_names_the_path_it_could_not_load() {
    let missing = std::env::temp_dir().join("cyberspine-absent-arbiter.dll");

    let error = dylib::open(&missing).expect_err("the file does not exist");

    assert!(
        error.to_string().contains("cyberspine-absent-arbiter"),
        "{error}"
    );
}
