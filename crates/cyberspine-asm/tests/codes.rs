//! The assembler's rules are one namespace, and the table is exhaustive.
//!
//! The `codes!` macro generates `CODES` from the same arms as the `match` it
//! compiles, so a new variant with no identifier fails to build rather than
//! silently reporting an empty code.

use cyberspine_asm::assembler::lint::WarningKind;
use cyberspine_asm::assembler::{ImageGenError, ParseErrorKind};
use cyberspine_diag::table_is_sane;

#[test]
fn every_assembler_table_is_sane() {
    assert_eq!(table_is_sane(ParseErrorKind::CODES), Ok(()));
    assert_eq!(table_is_sane(ImageGenError::CODES), Ok(()));
    assert_eq!(table_is_sane(WarningKind::CODES), Ok(()));
}

#[test]
fn every_assembler_code_is_in_the_asm_namespace() {
    for code in ParseErrorKind::CODES
        .iter()
        .chain(ImageGenError::CODES)
        .chain(WarningKind::CODES)
    {
        assert_eq!(code.namespace(), "asm", "{code}");
    }
}
