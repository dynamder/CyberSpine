//! `VerifiedImage::verify` holds an in-memory image to the loader's rules
//! instead of reaching a producer assertion first.

use cyberspine_binfmt::{DebugError, DebugInfo, DebugLine, Image, LoadError, VerifiedImage};
use cyberspine_isa::{Insn, OpCode};

#[test]
fn a_short_debug_line_map_is_a_load_error_not_a_panic() {
    let mut image = Image::new(vec![Insn::new(OpCode::Nop), Insn::new(OpCode::Halt)]);
    image.set_debug(DebugInfo::new(
        "probe.csa",
        Vec::new(),
        vec![DebugLine { line: 1, column: 1 }],
    ));

    let error = VerifiedImage::verify(image).expect_err("the debug line map is short");

    assert!(
        matches!(error, LoadError::Debug(DebugError::LineMapLength { .. })),
        "{error}"
    );
}
