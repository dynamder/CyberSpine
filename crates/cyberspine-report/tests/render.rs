//! What a diagnostic looks like in each of the two shapes.
//!
//! These pin the contract an agent reads: the JSON field names, and the
//! human header. Prose is allowed to change; `code`, `kind`, `line` are not.

use cyberspine_diag::{Anchor, Code, Diagnostic, Site};
use cyberspine_report::{Format, Renderer};

const TEXT: &str = "\
.layer 10
    FADD f1, f2, f3
    FADDD f1, f2, f3
";

fn sample() -> Diagnostic {
    Diagnostic::error(
        Code::new("asm.unknown-mnemonic"),
        "unknown mnemonic `FADDD`",
    )
    .at(Anchor::at_source("probe.csa", 3, 5))
    .label(
        Anchor::at_source("probe.csa", 2, 5),
        "the previous instruction used a real mnemonic",
    )
    .note("the mnemonic table is generated from `OpCode::ALL`")
    .with_help("did you mean `FADD`?")
}

#[test]
fn json_is_one_object_per_line_with_the_documented_fields() {
    let mut renderer = Renderer::new(Format::Json);
    let mut out = Vec::new();
    renderer
        .diagnostic(&sample(), &mut out)
        .expect("json renders");

    let text = String::from_utf8(out).expect("json is utf-8");
    assert_eq!(text.matches('\n').count(), 1, "exactly one line");

    let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("the line is json");
    assert_eq!(value["schema"], cyberspine_diag::SCHEMA);
    assert_eq!(value["kind"], "diagnostic");
    assert_eq!(value["level"], "error");
    assert_eq!(value["code"], "asm.unknown-mnemonic");
    assert_eq!(value["primary"]["artifact"], "probe.csa");
    assert_eq!(value["primary"]["kind"], "source");
    assert_eq!(value["primary"]["line"], 3);
    assert_eq!(value["primary"]["column"], 5);
    assert_eq!(value["labels"][0]["kind"], "source");
    assert_eq!(value["labels"][0]["line"], 2);
    assert_eq!(
        value["notes"][0].as_str().unwrap(),
        "the mnemonic table is generated from `OpCode::ALL`"
    );
    assert_eq!(value["help"], "did you mean `FADD`?");
}

#[test]
fn human_draws_the_source_frame_and_the_code() {
    let mut renderer = Renderer::new(Format::Human);
    renderer.remember("probe.csa", TEXT);
    let mut out = Vec::new();
    renderer
        .diagnostic(&sample(), &mut out)
        .expect("human renders");

    let text = String::from_utf8(out).expect("human output is utf-8");
    assert!(text.contains("error[asm.unknown-mnemonic]"), "{text}");
    assert!(
        text.contains("FADDD"),
        "the offending line is quoted:\n{text}"
    );
    assert!(text.contains("help: did you mean `FADD`?"), "{text}");
}

#[test]
fn human_names_the_artifact_for_a_place_it_cannot_draw() {
    let mut renderer = Renderer::new(Format::Human);
    let diagnostic = Diagnostic::error(
        Code::new("exec.fuel-exhausted"),
        "fuel exhausted at instruction 7",
    )
    .at(Anchor::new("action.csp", Site::Instruction { pc: 7 }));
    let mut out = Vec::new();
    renderer
        .diagnostic(&diagnostic, &mut out)
        .expect("human renders");

    let text = String::from_utf8(out).expect("human output is utf-8");
    assert!(text.contains("error[exec.fuel-exhausted]"), "{text}");
    assert!(text.contains("action.csp:instruction 7"), "{text}");
}

#[test]
fn an_unreadable_source_degrades_to_a_plain_location() {
    // No text was handed in and none exists on disk: rendering must not fail,
    // it just stops drawing the frame.
    let mut renderer = Renderer::new(Format::Human);
    let mut out = Vec::new();
    renderer
        .diagnostic(&sample(), &mut out)
        .expect("rendering never fails on a missing source");
    let text = String::from_utf8(out).expect("human output is utf-8");
    assert!(text.contains("probe.csa:3:5"), "{text}");
}

#[test]
fn a_truncated_batch_says_how_many_are_missing_in_both_shapes() {
    let diagnostics: Vec<_> = (0..3)
        .map(|index| {
            Diagnostic::error(
                Code::new("exec.fuel-exhausted"),
                format!("fault at instruction {index}"),
            )
        })
        .collect();

    let mut human = Renderer::new(Format::Human);
    let mut out = Vec::new();
    human
        .diagnostics(&diagnostics, 7, &mut out)
        .expect("human renders");
    let text = String::from_utf8(out).expect("human output is utf-8");
    assert!(text.contains("3 errors emitted"), "{text}");
    assert!(
        text.contains("note: 7 more diagnostic(s) were not shown"),
        "{text}"
    );

    let mut json = Renderer::new(Format::Json);
    let mut out = Vec::new();
    json.diagnostics(&diagnostics, 7, &mut out)
        .expect("json renders");
    let text = String::from_utf8(out).expect("json output is utf-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "three diagnostics and one truncation record"
    );
    let last: serde_json::Value =
        serde_json::from_str(lines[3]).expect("the truncation record is json");
    assert_eq!(last["schema"], cyberspine_diag::SCHEMA);
    assert_eq!(last["kind"], "truncated");
    assert_eq!(last["dropped"], 7);

    let mut out = Vec::new();
    Renderer::new(Format::Json)
        .diagnostics(&diagnostics, 0, &mut out)
        .expect("json renders");
    let text = String::from_utf8(out).expect("json output is utf-8");
    assert!(
        !text.contains("truncated"),
        "a complete list has no truncation record:\n{text}"
    );
}
