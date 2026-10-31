//! End-to-end diagnostics: aggregation across sections and rendering through
//! `annotate-snippets` (`docs/asm.md` §6.1).
//!
//! `header()` and `text()` are also tested in isolation; this file pins the
//! property that `parse()` coordinates them: instruction-section line numbers
//! are shifted into file coordinates, and errors from both sections survive.

use std::path::Path;

use cyberspine_asm::assembler::{Location, ParseErrorKind, parse};

const TWO_ERRORS: &str = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.wobble 1
.text
FADDD f1, f2, f3
HALT
";

#[test]
fn parse_collects_header_and_instruction_errors_in_file_coordinates() {
    let errors = parse(TWO_ERRORS)
        .expect_err("the source contains two independent mistakes")
        .errors;

    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        matches!(&errors[0].kind, ParseErrorKind::UnknownDirective(name) if name == "wobble"),
        "{errors:?}"
    );
    assert_eq!(errors[0].location, Location::new(5, 1));
    assert!(
        matches!(&errors[1].kind, ParseErrorKind::UnknownMnemonic(name) if name == "FADDD"),
        "{errors:?}"
    );
    assert_eq!(errors[1].location, Location::new(7, 1));
}

#[test]
fn text_errors_without_a_text_marker_keep_file_coordinates() {
    let src = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
FADDD f1, f2, f3
";
    let errors = parse(src)
        .expect_err("the instruction line has an unknown mnemonic")
        .errors;

    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].location, Location::new(5, 1));
}

#[test]
fn every_missing_fixed_field_is_reported_once() {
    let errors = parse("").expect_err("the fixed header is empty").errors;

    assert_eq!(errors.len(), 4, "{errors:?}");
    for (index, field) in ["layer", "clock_hz", "fuel", "max_wait"]
        .into_iter()
        .enumerate()
    {
        assert!(
            matches!(&errors[index].kind, ParseErrorKind::MissingField(name) if *name == field),
            "for {field}: {errors:?}"
        );
        assert_eq!(errors[index].location, Location::new(1, 1));
    }
}

#[test]
fn render_points_at_the_offending_lines() {
    let errors = parse(TWO_ERRORS).expect_err("the source contains two mistakes");
    let rendered = errors.render(Path::new("bad.csa"), TWO_ERRORS);

    assert!(
        rendered.contains("error: unknown directive `.wobble`"),
        "{rendered}"
    );
    assert!(
        rendered.contains("error: unknown mnemonic `FADDD`"),
        "{rendered}"
    );
    assert!(rendered.contains("bad.csa:5:1"), "{rendered}");
    assert!(rendered.contains("bad.csa:7:1"), "{rendered}");
    assert!(rendered.contains(".wobble 1"), "{rendered}");
    assert!(rendered.contains("FADDD f1, f2, f3"), "{rendered}");
    assert!(rendered.contains("2 errors emitted"), "{rendered}");
}
