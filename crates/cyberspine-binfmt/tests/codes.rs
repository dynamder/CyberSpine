//! Every loader refusal names a rule from `docs/binfmt.md` §9.
//!
//! The table is generated from the same `match` the compiler checks for
//! exhaustiveness, so this test is about the identifiers themselves: one
//! namespace, and no code that is not `namespace.name`.

use cyberspine_binfmt::LoadError;
use cyberspine_diag::table_is_sane;

#[test]
fn the_loader_code_table_is_sane() {
    assert_eq!(table_is_sane(LoadError::CODES), Ok(()));
}

#[test]
fn every_loader_code_is_in_the_binfmt_namespace() {
    for code in LoadError::CODES {
        assert_eq!(code.namespace(), "binfmt", "{code}");
    }
}
