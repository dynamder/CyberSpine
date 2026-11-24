//! The human diagnostic shape, as bytes.
//!
//! The JSON field names are pinned in `render.rs` and the run records in
//! `cyberspine-run-report`; this snapshot covers the one rendering that is not
//! asserted field by field: the source frame.

use cyberspine_diag::{Anchor, Code, Diagnostic};
use cyberspine_report::{Format, Renderer};

#[test]
fn the_human_diagnostic_renders_the_same_artifact() {
    let text = "\
.layer 10
    FADD f1, f2, f3
    FADDD f1, f2, f3
";
    let diagnostic = Diagnostic::error(
        Code::new("asm.unknown-mnemonic"),
        "unknown mnemonic `FADDD`",
    )
    .at(Anchor::at_source("probe.csa", 3, 5))
    .label(
        Anchor::at_source("probe.csa", 2, 5),
        "the previous instruction used a real mnemonic",
    )
    .note("the mnemonic table is generated from `OpCode::ALL`")
    .with_help("did you mean `FADD`?");

    let mut renderer = Renderer::new(Format::Human);
    renderer.remember("probe.csa", text);
    let mut out = Vec::new();
    renderer
        .diagnostic(&diagnostic, &mut out)
        .expect("human renders");

    insta::assert_snapshot!(
        "diagnostic_human",
        String::from_utf8(out).expect("the renderer writes UTF-8")
    );
}
