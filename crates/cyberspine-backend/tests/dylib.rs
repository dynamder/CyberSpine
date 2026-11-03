//! End to end through a real shared library.
//!
//! The fixture crate is built on demand: Cargo does not build another package's
//! `cdylib` for a test, so this test asks it to. The artifact path comes from
//! `--message-format=json`, which keeps the test independent of the target
//! directory, the profile and the platform's library suffix.

#![cfg(feature = "dylib")]

#[path = "../../cyberspine-backend-fixture/src/body.rs"]
mod body;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use cyberspine_backend::dylib;
use cyberspine_rig::BoundFingerprint;
use cyberspine_testkit as testkit;
use cyberspine_vm::{Backend, Host, Shape};

fn fixture() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();

    PATH.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "cyberspine-backend-fixture",
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
            if !package.contains("cyberspine-backend-fixture") {
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

/// Four ticks of a program that publishes both muscles every tick.
fn trace<B: Backend>(host: Host<B>) -> Vec<Vec<Option<f32>>> {
    let image = testkit::verified(testkit::action(
        vec![
            testkit::sense(1, 2),
            testkit::excite(1, 1),
            testkit::fmovi(2, 0.5),
            testkit::excite(0, 2),
            testkit::flush(),
            testkit::yield_(),
            testkit::jump(-7),
        ],
        &body::rig(),
    ));

    let (mut session, _) = host.load(image).expect("the image fits the body");
    session.set_tracing(true);
    for _ in 0..4 {
        session.tick().expect("the body answers");
    }
    session.trace().map(|tick| tick.commands.to_vec()).collect()
}

#[test]
fn the_fixture_manifest_and_body_declare_the_same_rig() {
    let manifest =
        cyberspine_manifest::from_json(body::RIG_JSON).expect("the fixture manifest parses");

    assert_eq!(manifest, body::rig());
}

#[test]
fn a_library_declares_the_same_body_as_the_linked_type() {
    let loaded = dylib::open(fixture()).expect("the fixture loads");

    assert_eq!(
        loaded.fingerprint(),
        BoundFingerprint::of(&body::rig()),
        "the manifest the library embeds is the rig the linked body hashes"
    );
    assert_eq!(
        loaded.shape(),
        Shape {
            muscles: 2,
            sensors_f: 1,
            sensors_i: 1
        }
    );
    assert_eq!(loaded.build_id(), "cyberspine-backend-fixture/0.1.0");
}

#[test]
fn a_library_produces_the_same_trace_as_the_linked_body() {
    let linked = trace(Host::new(body::Fixture::new()));
    let loaded = trace(Host::new(
        dylib::open(fixture()).expect("the fixture loads"),
    ));

    assert_eq!(loaded, linked, "one body, two packagings, one trace");
    assert_eq!(
        linked[1],
        vec![Some(0.5), Some(1.0)],
        "the sensor crossed the boundary and drove the second muscle"
    );
}

#[test]
fn a_missing_library_names_the_path_it_could_not_load() {
    let missing = std::env::temp_dir().join("cyberspine-absent-body.dll");

    let error = dylib::open(&missing).expect_err("the file does not exist");

    assert!(
        error.to_string().contains("cyberspine-absent-body"),
        "{error}"
    );
}
