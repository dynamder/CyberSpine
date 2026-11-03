//! `INTR` in the assembler: the quoted reason operand, template rewriting, and
//! the new suspend point (`docs/asm.md` §4.4, `docs/binfmt.md` §6.6).

use cyberspine_asm::assembler::{ImageGenError, generate_image, parse};
use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::binfmt::reason::TemplateError;
use cyberspine_core::isa::OpCode;
use cyberspine_core::rig::Rig;
use cyberspine_testkit::name as n;

const HEADER: &str = ".layer 10\n.clock hz = 60\n.fuel 4096\n.max_wait 600.0\n";

fn rig() -> Rig {
    Rig::new("test", 1).muscle(n("m0"))
}

fn source(decls: &str, body: &str) -> String {
    format!("{HEADER}{decls}\n.text\n{body}")
}

fn generate(text: &str) -> Result<Image, ImageGenError> {
    let asm = parse(text).expect("the test source parses");
    generate_image(&asm, &rig())
}

#[test]
fn a_reason_is_canonicalized_and_loaded() {
    let image = generate(&source(
        ".var_f phase = 0.0\n.var_i state = 1",
        "    INTR \"x={f1:.2} p={vf:phase} s={vi:state} {{lit}}\"\n    HALT\n",
    ))
    .expect("the template is valid");

    assert_eq!(image.code()[0].op, OpCode::Intr);
    assert_eq!(image.code()[0].imm_index(), 0, "the first reason row");
    assert_eq!(
        image.reason(0),
        "x={f1:.2} p={vf:0} s={vi:0} {{lit}}",
        "names become indices and literal braces stay escaped"
    );

    VerifiedImage::verify(image).expect("the generated image passes the loader");
}

#[test]
fn the_same_template_is_stored_once() {
    let image = generate(&source(
        "",
        "    INTR \"same\"\n    INTR \"same\"\n    HALT\n",
    ))
    .expect("both sites are legal");

    assert_eq!(image.reasons().len(), 1);
    assert_eq!(image.code()[0].imm_index(), 0);
    assert_eq!(image.code()[1].imm_index(), 0);
}

#[test]
fn an_unknown_variable_is_a_generation_error() {
    let error = generate(&source("", "    INTR \"{vf:missing}\"\n    HALT\n"))
        .expect_err("missing is not declared");

    match error {
        ImageGenError::InvalidReason { source, .. } => {
            assert_eq!(
                source,
                TemplateError::UnknownVar {
                    name: "missing".to_string()
                }
            );
        }
        other => panic!("the template error was not reported: {other:?}"),
    }
}

#[test]
fn a_broken_template_is_a_generation_error() {
    let error = generate(&source("", "    INTR \"x={f64}\"\n    HALT\n"))
        .expect_err("f64 is not a register");
    assert!(matches!(
        error,
        ImageGenError::InvalidReason {
            source: TemplateError::RegisterOutOfRange { .. },
            ..
        }
    ));

    let error = generate(&source("", "    INTR \"x={\"\n    HALT\n"))
        .expect_err("an unclosed placeholder is broken");
    assert!(matches!(
        error,
        ImageGenError::InvalidReason {
            source: TemplateError::UnclosedBrace { .. },
            ..
        }
    ));
}

#[test]
fn an_intr_after_an_unflushed_excite_is_refused() {
    let error = generate(&source(
        "",
        "    FMOVI f1, 0.5\n    FEXCITE m0, f1\n    INTR \"pending\"\n    HALT\n",
    ))
    .expect_err("INTR is a suspend point");

    assert!(matches!(error, ImageGenError::PendingNotFlushed { .. }));
}

#[test]
fn an_intr_without_a_reason_does_not_parse() {
    assert!(
        parse(&source("", "    INTR\n    HALT\n")).is_err(),
        "the operand is mandatory"
    );
}
