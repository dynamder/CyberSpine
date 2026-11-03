//! The spec string is the one spelling the CLI and a production config share, so
//! its grammar is pinned here rather than in the CLI.

use cyberspine_backend::{BackendSpec, SpecError};

#[test]
fn a_spec_is_a_kind_and_a_target() {
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
fn the_dummy_body_is_not_a_spec() {
    assert!(matches!(
        BackendSpec::parse("dummy:rigs/maze.rig.json"),
        Err(SpecError::BuiltinDummy)
    ));
}

#[cfg(not(feature = "dylib"))]
#[test]
fn a_dylib_spec_without_the_feature_says_so() {
    let error = BackendSpec::Dylib("bodies/maze.dll".into())
        .open()
        .map(|_| ())
        .expect_err("the feature is off");

    assert!(error.to_string().contains("dylib"), "{error}");
}

#[cfg(not(feature = "rpc"))]
#[test]
fn an_rpc_spec_without_the_feature_says_so() {
    let error = BackendSpec::Rpc("127.0.0.1:7777".into())
        .open()
        .map(|_| ())
        .expect_err("the feature is off");

    assert!(error.to_string().contains("rpc"), "{error}");
}
