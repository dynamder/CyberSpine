//! End-to-end tests for the binary-generation stage: `parse` then
//! `generate_image` (`docs/asm.md` §5 / §6, `docs/binfmt.md` §3 / §6).
//!
//! Every positive test that builds an `Image` also runs it through
//! `Image::from_bytes`: an image the loader rejects is not a finished image.
//!
//! `docs/asm.md` §3.2 makes the rig the only source of muscles and observables,
//! so a test that names one has to hand in a `Rig` that contains it. The helpers
//! below make that explicit: `rig` builds the smallest rig a test can get away
//! with, and the `_with` forms take it.

use cyberspine_asm::assembler::{ImageGenError, generate_image, parse};
use cyberspine_core::binfmt::image::{Image, SensorSource};
use cyberspine_core::isa::jump::jump_target;
use cyberspine_core::isa::{ImmType, Insn, OpCode};
use cyberspine_core::rig::Rig;

/// The scalar header is currently mandatory (see `tests/header.rs`).
const HEADER: &str = ".layer 10\n.clock hz = 60\n.fuel 4096\n.max_wait 600.0\n";

fn source(body: &str) -> String {
    format!("{HEADER}{body}")
}

fn rig(muscles: &[&str], sensors_f: &[&str], sensors_i: &[&str]) -> Rig {
    let mut rig = Rig::new("test", 1);
    for name in muscles {
        rig = rig.muscle(*name);
    }
    for name in sensors_f {
        rig = rig.sensor_f(*name);
    }
    for name in sensors_i {
        rig = rig.sensor_i(*name);
    }
    rig
}

/// A rig with no names at all, for the tests that never mention one.
fn empty_rig() -> Rig {
    rig(&[], &[], &[])
}

/// Assemble a whole source against `rig`.
fn generate_with(src: &str, rig: &Rig) -> Result<Image, ImageGenError> {
    let asm = parse(src).expect("the source should parse");
    generate_image(&asm, rig)
}

/// Assemble a body under the standard header against `rig`.
fn body_with(body: &str, rig: &Rig) -> Result<Image, ImageGenError> {
    generate_with(&source(body), rig)
}

fn assemble_with(body: &str, rig: &Rig) -> Image {
    body_with(body, rig).unwrap_or_else(|error| {
        panic!("the source should generate an image: {error}\n--- body ---\n{body}")
    })
}

fn error_with(body: &str, rig: &Rig) -> ImageGenError {
    body_with(body, rig).expect_err("generation should fail")
}

fn assemble(body: &str) -> Image {
    assemble_with(body, &empty_rig())
}

fn generation_error(body: &str) -> ImageGenError {
    error_with(body, &empty_rig())
}

// ---------------------------------------------------------------------------
// 1. Header, implicit HALT, loader round trip
// ---------------------------------------------------------------------------

#[test]
fn a_minimal_program_gets_the_implicit_halt_and_the_header() {
    let rig = empty_rig();
    let image = assemble_with(".text\nNOP\n", &rig);

    assert_eq!(image.header.layer, 10);
    assert_eq!(image.header.hz, 60);
    assert_eq!(image.header.fuel, 4096);
    assert_eq!(image.header.max_wait, 600.0);
    assert_eq!(
        image.header.rig,
        rig.fingerprint(),
        "the image is bound to the rig it was assembled for"
    );
    assert_eq!(image.code.len(), 2);
    assert_eq!(image.code[0].op, OpCode::Nop);
    assert_eq!(image.code[1].op, OpCode::Halt);

    let reloaded = Image::from_bytes(&image.to_bytes()).expect("the loader must accept it");
    assert_eq!(reloaded, image);
}

#[test]
fn an_empty_instruction_section_is_exactly_one_halt() {
    let image = assemble(".text\n");
    assert_eq!(image.code, vec![Insn::new(OpCode::Halt)]);
}

// ---------------------------------------------------------------------------
// 2. Name resolution and table order
// ---------------------------------------------------------------------------

#[test]
fn names_resolve_to_the_spec_table_order() {
    let rig = rig(&["brow_l", "brow_r"], &["smile"], &["gaze_lock"]);
    let image = assemble_with(
        "\
.group brows = brow_l, brow_r
.var_f phase = 0.5
.var_i state = -2
.text
FSENSE f1, brow_l
FSENSE f2, smile
FSENSE f3, dt
ISENSE i1, gaze_lock
ISENSE i2, tick
FVGET f4, phase
IVGET i3, state
FEXCITE brow_r, f1
FEXCITE brows, f1
FLUSH
HALT
",
        &rig,
    );

    assert_eq!(image.muscles.len(), 2);
    assert_eq!(&*image.muscles[0].name, "brow_l");
    assert_eq!(&*image.muscles[1].name, "brow_r");

    assert_eq!(image.sensors_f.len(), 4);
    assert_eq!(&*image.sensors_f[0].name, "brow_l");
    assert_eq!(image.sensors_f[0].source, SensorSource::Muscle);
    assert_eq!(&*image.sensors_f[1].name, "brow_r");
    assert_eq!(&*image.sensors_f[2].name, "smile");
    assert_eq!(image.sensors_f[2].source, SensorSource::Backend);
    assert_eq!(&*image.sensors_f[3].name, "dt");
    assert_eq!(image.sensors_f[3].source, SensorSource::BuiltinDt);

    assert_eq!(image.sensors_i.len(), 2);
    assert_eq!(&*image.sensors_i[0].name, "gaze_lock");
    assert_eq!(&*image.sensors_i[1].name, "tick");
    assert_eq!(image.sensors_i[1].source, SensorSource::BuiltinTick);

    assert_eq!(image.vars_f.len(), 1);
    assert_eq!(image.vars_f[0].init, 0.5);
    assert_eq!(image.vars_i.len(), 1);
    assert_eq!(image.vars_i[0].init, -2);

    // The `imm` of each instruction is the physical index, in the order above.
    let imm: Vec<u32> = image.code.iter().map(|insn| insn.imm).collect();
    assert_eq!(imm, vec![0, 2, 3, 0, 1, 0, 0, 1, 0, 1, 0, 0]);

    let reloaded = Image::from_bytes(&image.to_bytes()).expect("the loader must accept it");
    assert_eq!(reloaded, image);
}

/// The table order is the rig's order, not the source's: swapping two names in
/// the manifest is another rig, and the image follows the manifest.
#[test]
fn the_rig_order_decides_the_indices() {
    let rig = rig(&["b_second", "a_first"], &[], &[]);
    let image = assemble_with(
        ".text\nFEXCITE b_second, f1\nFLUSH\nFEXCITE a_first, f1\nFLUSH\n",
        &rig,
    );

    assert_eq!(&*image.muscles[0].name, "b_second");
    assert_eq!(&*image.muscles[1].name, "a_first");
    assert_eq!(image.code[0].imm, 0);
    assert_eq!(image.code[2].imm, 1);
}

// ---------------------------------------------------------------------------
// 3. group expansion
// ---------------------------------------------------------------------------

#[test]
fn a_group_expands_to_one_excite_per_member() {
    let rig = rig(&["a", "b"], &[], &[]);
    let image = assemble_with(
        "\
.group g = a, b
.text
FEXCITE g, f1
FLUSH
HALT
",
        &rig,
    );

    assert_eq!(image.code.len(), 4);
    assert_eq!(image.code[0].op, OpCode::Fexcite);
    assert_eq!(image.code[0].imm, 0);
    assert_eq!(image.code[0].ra.get(), 1);
    assert_eq!(image.code[1].op, OpCode::Fexcite);
    assert_eq!(image.code[1].imm, 1);
    assert_eq!(image.code[1].ra.get(), 1);
    assert_eq!(image.code[2].op, OpCode::Flush);
    assert_eq!(image.code[3].op, OpCode::Halt);
}

// ---------------------------------------------------------------------------
// 4. Labels and offsets
// ---------------------------------------------------------------------------

#[test]
fn labels_become_relative_offsets_through_jump_offset() {
    let image = assemble(
        "\
.text
entry:
    JMP end
    NOP
end:
    HALT
",
    );

    assert_eq!(image.code.len(), 3);
    assert_eq!(jump_target(0, image.code[0].imm as i32), Some(2));
    assert_eq!(image.code[0].imm as i32, 1);
    assert_eq!(image.code[1].op, OpCode::Nop);
    assert_eq!(image.code[2].op, OpCode::Halt);
}

#[test]
fn a_label_at_the_end_lands_on_the_implicit_halt() {
    let image = assemble(
        "\
.text
entry:
    JMP end
end:
",
    );

    assert_eq!(image.code.len(), 2);
    assert_eq!(image.code[0].imm as i32, 0);
    assert_eq!(jump_target(0, 0), Some(1));
    assert_eq!(image.code[1].op, OpCode::Halt);
}

#[test]
fn a_label_after_an_explicit_halt_is_out_of_range() {
    let err = generation_error(".text\nHALT\nend:\n");
    assert!(
        matches!(err, ImageGenError::LabelOutOfRange { .. }),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// 5. The documented example (`docs/asm.md` §7)
// ---------------------------------------------------------------------------

#[test]
fn the_documented_example_assembles_and_reloads() {
    let src = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0

.rig \"rigs/live2d-standard.rig\"

.group brows = brow_l, brow_r
.group corners = mouth_corner_l, mouth_corner_r

.text
entry:
    ISENSE  i1, gaze_lock
    IJNZ    i1, L_done

    FMOVI   f1, 1.0
    FMOVI   f2, 0.0
    FMOVI   f3, 0.85
    FMOVI   f4, 0.90

    FEXCITE brows, f1
    FLUSH
wait_brow:
    FSENSE  f5, brow_raise
    FGE     f5, f4, L_brow_ok
    YIELD
    JMP     wait_brow
L_brow_ok:

    FEXCITE corners, f1
    FLUSH
wait_smile:
    FSENSE  f5, smile
    FGE     f5, f3, L_smile_ok
    YIELD
    JMP     wait_smile
L_smile_ok:

    WAIT    2.0

    FEXCITE brows, f2
    FEXCITE corners, f2
    FLUSH
    WAIT    0.5
L_done:
    HALT
";

    let rig = rig(
        &["brow_l", "brow_r", "mouth_corner_l", "mouth_corner_r"],
        &["smile", "brow_raise"],
        &["gaze_lock"],
    );

    let asm = parse(src).expect("the documented example parses");
    let image = generate_image(&asm, &rig).expect("the documented example assembles");

    assert_eq!(image.code.len(), 28);
    assert_eq!(image.muscles.len(), 4);
    assert_eq!(image.sensors_f.len(), 7);
    assert_eq!(image.sensors_i.len(), 2);
    assert_eq!(image.header.rig, rig.fingerprint());
    assert_eq!(image.code.last().expect("non-empty").op, OpCode::Halt);

    assert_eq!(jump_target(1, image.code[1].imm as i32), Some(27));
    assert_eq!(jump_target(10, image.code[10].imm as i32), Some(13));
    assert_eq!(jump_target(12, image.code[12].imm as i32), Some(9));
    assert_eq!(jump_target(17, image.code[17].imm as i32), Some(20));
    assert_eq!(jump_target(19, image.code[19].imm as i32), Some(16));

    let reloaded = Image::from_bytes(&image.to_bytes()).expect("the loader must accept it");
    assert_eq!(reloaded, image);
}

// ---------------------------------------------------------------------------
// 6. Static rejections
// ---------------------------------------------------------------------------

#[test]
fn duplicate_and_undefined_labels_are_rejected() {
    assert!(matches!(
        generation_error(".text\na:\na:\nHALT\n"),
        ImageGenError::DuplicateLabel(_)
    ));
    assert!(matches!(
        generation_error(".text\nJMP nowhere\nHALT\n"),
        ImageGenError::UndefinedLabel(_)
    ));
}

#[test]
fn unknown_names_are_rejected_per_namespace() {
    let rig = rig(&["brow_l"], &["smile"], &["gaze_lock"]);

    assert!(matches!(
        error_with(".text\nFSENSE f1, nope\nHALT\n", &rig),
        ImageGenError::UnknownSensorF(_)
    ));
    assert!(matches!(
        error_with(".text\nISENSE i1, nope\nHALT\n", &rig),
        ImageGenError::UnknownSensorI(_)
    ));
    assert!(matches!(
        error_with(".text\nFVGET f1, nope\nHALT\n", &rig),
        ImageGenError::UnknownVarF(_)
    ));
    assert!(matches!(
        error_with(".text\nIVGET i1, nope\nHALT\n", &rig),
        ImageGenError::UnknownVarI(_)
    ));
    assert!(matches!(
        error_with(".text\nFEXCITE nope, f1\nHALT\n", &rig),
        ImageGenError::UnknownExciteTarget(_)
    ));
}

#[test]
fn groups_are_only_legal_at_fexcite() {
    let rig = rig(&["a"], &[], &[]);
    let err = error_with(
        "\
.group g = a
.text
FSENSE f1, g
HALT
",
        &rig,
    );
    assert!(
        matches!(err, ImageGenError::GroupOutsideExcite(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn group_members_must_be_rig_muscles() {
    let err = generation_error(".group g = missing\n.text\nHALT\n");
    assert!(
        matches!(err, ImageGenError::UnknownGroupMember { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_group_name_cannot_collide_with_a_muscle() {
    let rig = rig(&["a"], &[], &[]);
    let err = error_with(".group a = a\n.text\nHALT\n", &rig);
    assert!(
        matches!(err, ImageGenError::GroupNameCollision(_)),
        "unexpected error: {err}"
    );
}

/// A `Rig` built in code never passes through the manifest parser, but
/// `generate_image` still owes the loader a legal image
/// (`docs/binfmt.md` §9.2 M1 / M2 / M5).
#[test]
fn a_rig_that_would_produce_an_illegal_image_is_refused() {
    let reserved = Rig::new("r", 1).muscle("dt");
    assert!(matches!(
        body_with(".text\nHALT\n", &reserved),
        Err(ImageGenError::ReservedName(_))
    ));

    let register_shape = Rig::new("r", 1).sensor_f("f3");
    assert!(matches!(
        body_with(".text\nHALT\n", &register_shape),
        Err(ImageGenError::ReservedName(_))
    ));

    let not_an_identifier = Rig::new("r", 1).muscle("9bad");
    assert!(matches!(
        body_with(".text\nHALT\n", &not_an_identifier),
        Err(ImageGenError::NotAnIdentifier(_))
    ));

    let repeated = Rig::new("r", 1).muscle("m").sensor_f("m");
    assert!(matches!(
        body_with(".text\nHALT\n", &repeated),
        Err(ImageGenError::DuplicateName(_))
    ));
}

#[test]
fn duplicate_declarations_are_rejected() {
    let rig = rig(&["a"], &[], &[]);

    for body in [
        ".var_f v = 0.0\n.var_f v = 1.0\n.text\nHALT\n",
        ".var_i v = 0\n.var_i v = 1\n.text\nHALT\n",
        ".group g = a\n.group g = a\n.text\nHALT\n",
    ] {
        let err = error_with(body, &rig);
        assert!(
            matches!(err, ImageGenError::DuplicateName(_)),
            "for {body:?}: unexpected error: {err}"
        );
    }
}

#[test]
fn reserved_names_are_rejected_in_the_tables_the_source_still_owns() {
    let rig = rig(&["a"], &[], &[]);

    for body in [
        ".var_f dt = 0.0\n.text\nHALT\n",
        ".var_i tick = 0\n.text\nHALT\n",
        ".group dt = a\n.text\nHALT\n",
    ] {
        let err = error_with(body, &rig);
        assert!(
            matches!(err, ImageGenError::ReservedName(_)),
            "for {body:?}: unexpected error: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Pending-write dataflow (`docs/asm.md` §4.3 / §6 第 6 条)
// ---------------------------------------------------------------------------

#[test]
fn an_unflushed_excite_before_a_suspend_point_is_rejected() {
    let rig = rig(&["m"], &[], &[]);
    let err = error_with(
        "\
FEXCITE m, f1
YIELD
HALT
",
        &rig,
    );
    assert!(
        matches!(
            err,
            ImageGenError::PendingNotFlushed {
                excite_pc: 0,
                suspend_pc: 1
            }
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn an_unflushed_excite_before_halt_is_rejected() {
    let rig = rig(&["m"], &[], &[]);
    let err = error_with(
        "\
FEXCITE m, f1
HALT
",
        &rig,
    );
    assert!(matches!(err, ImageGenError::PendingNotFlushed { .. }));
}

#[test]
fn a_flush_on_only_one_branch_still_leaks() {
    let rig = rig(&["m"], &[], &[]);
    let err = error_with(
        "\
FEXCITE m, f1
IJZ i1, L
FLUSH
L:
YIELD
HALT
",
        &rig,
    );
    assert!(
        matches!(err, ImageGenError::PendingNotFlushed { suspend_pc: 3, .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_flush_on_every_branch_is_accepted() {
    let rig = rig(&["m"], &[], &[]);
    let image = assemble_with(
        "\
FEXCITE m, f1
IJZ i1, L
FLUSH
JMP done
L:
FLUSH
done:
YIELD
HALT
",
        &rig,
    );
    assert_eq!(image.code.last().expect("non-empty").op, OpCode::Halt);
}

// ---------------------------------------------------------------------------
// 8. Header and WAIT bounds
// ---------------------------------------------------------------------------

#[test]
fn wait_must_be_non_negative_finite_and_within_max_wait() {
    assert_eq!(
        assemble(".text\nWAIT 600.0\nHALT\n").code[0].op,
        OpCode::Wait
    );

    for body in [
        ".text\nWAIT 600.5\nHALT\n",
        ".text\nWAIT -1.0\nHALT\n",
        ".text\nWAIT inf\nHALT\n",
    ] {
        let err = generation_error(body);
        assert!(
            matches!(err, ImageGenError::InvalidWait { .. }),
            "for {body:?}: unexpected error: {err}"
        );
    }
}

#[test]
fn hz_must_be_positive_and_max_wait_finite() {
    let zero_hz = "\
.layer 10
.clock hz = 0
.fuel 4096
.max_wait 600.0
.text
HALT
";
    assert!(matches!(
        generate_with(zero_hz, &empty_rig()),
        Err(ImageGenError::ZeroHz)
    ));

    let bad_max_wait = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait -1.0
.text
HALT
";
    assert!(matches!(
        generate_with(bad_max_wait, &empty_rig()),
        Err(ImageGenError::InvalidMaxWait)
    ));
}

// ---------------------------------------------------------------------------
// 9. Offset sanity (the single shared arithmetic)
// ---------------------------------------------------------------------------

#[test]
fn every_emitted_jump_targets_a_real_instruction() {
    let image = assemble(
        "\
.text
top:
    NOP
    IJNZ i1, top
    HALT
",
    );
    for (pc, insn) in image.code.iter().enumerate() {
        if insn.op.imm_type() == ImmType::JumpOffset {
            let target = jump_target(pc, insn.imm as i32).expect("target must exist");
            assert!(target < image.code.len(), "jump at {pc} -> {target}");
        }
    }
}
