//! The spec string is the one spelling the CLI and a production config share, so
//! its grammar is pinned here rather than in the CLI.

use cyberspine_backend::{BackendSpec, SpecError};
use cyberspine_vm::Shape;

const RIG: &str = r#"{"rig":{"name":"probe","version":1},"muscles":["left","right"],"sensors_f":["load"],"sensors_i":["mode"]}"#;

#[test]
fn a_spec_is_a_kind_and_a_target() {
    assert_eq!(
        BackendSpec::parse("dummy:rigs/maze.rig.json").expect("a manifest"),
        BackendSpec::Dummy("rigs/maze.rig.json".into())
    );
    assert_eq!(
        BackendSpec::parse("dylib:bodies/maze.dll").expect("a library"),
        BackendSpec::Dylib("bodies/maze.dll".into())
    );
    assert_eq!(
        BackendSpec::parse("rpc:127.0.0.1:7777").expect("an address"),
        BackendSpec::Rpc("127.0.0.1:7777".into())
    );
    assert_eq!(
        BackendSpec::parse("rpc://127.0.0.1:7777").expect("a URL-shaped address"),
        BackendSpec::Rpc("127.0.0.1:7777".into())
    );
}

#[test]
fn a_spec_that_names_nothing_is_refused() {
    assert!(matches!(BackendSpec::parse("rpc"), Err(SpecError::Shape)));
    assert!(matches!(
        BackendSpec::parse("rpc:"),
        Err(SpecError::Empty { .. })
    ));
    assert!(matches!(
        BackendSpec::parse("bluetooth:x"),
        Err(SpecError::Kind(kind)) if kind == "bluetooth"
    ));
}

#[test]
fn a_dummy_body_reads_its_manifest_from_disk() {
    let dir = cyberspine_testkit::scratch_dir("backend-spec");
    let path = dir.join("probe.rig.json");
    std::fs::write(&path, RIG).expect("the fixture writes");

    let body = BackendSpec::Dummy(path).open().expect("a valid manifest");

    assert_eq!(
        body.shape(),
        Shape {
            muscles: 2,
            sensors_f: 1,
            sensors_i: 1
        }
    );
}

#[test]
fn a_missing_manifest_names_the_path_it_could_not_read() {
    let dir = cyberspine_testkit::scratch_dir("backend-spec-missing");
    let path = dir.join("absent.rig.json");

    let error = BackendSpec::Dummy(path.clone())
        .open()
        .map(|_| ())
        .expect_err("the file does not exist");

    assert!(error.to_string().contains("absent.rig.json"), "{error}");
}
