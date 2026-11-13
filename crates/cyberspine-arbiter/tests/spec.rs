//! The spec string is the one spelling the CLI and a production config share, so
//! its grammar is pinned here rather than in the CLI.

use cyberspine_arbiter::{ArbiterSpec, SpecError};

#[test]
fn a_spec_is_a_kind_and_a_target() {
    assert_eq!(
        ArbiterSpec::parse("dylib:arbiters/fallback.dll").expect("a library"),
        ArbiterSpec::Dylib("arbiters/fallback.dll".into())
    );
    assert_eq!(
        ArbiterSpec::parse("rpc:127.0.0.1:7788").expect("an address"),
        ArbiterSpec::Rpc("127.0.0.1:7788".into())
    );
    assert_eq!(
        ArbiterSpec::parse("rpc://127.0.0.1:7788").expect("a URL-shaped address"),
        ArbiterSpec::Rpc("127.0.0.1:7788".into())
    );
}

#[test]
fn a_spec_that_names_nothing_is_refused() {
    assert!(matches!(ArbiterSpec::parse("rpc"), Err(SpecError::Shape)));
    assert!(matches!(
        ArbiterSpec::parse("rpc:"),
        Err(SpecError::Empty { .. })
    ));
    assert!(matches!(
        ArbiterSpec::parse("bluetooth:x"),
        Err(SpecError::Kind(kind)) if kind == "bluetooth"
    ));
}

#[test]
fn the_builtin_arbiter_is_not_a_spec() {
    assert!(matches!(
        ArbiterSpec::parse("builtin:stop"),
        Err(SpecError::Builtin)
    ));
}
