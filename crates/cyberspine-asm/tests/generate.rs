//! End-to-end tests for the binary-generation stage: `parse` then
//! `generate_image` (`docs/asm.md` §5 / §6, `docs/binfmt.md` §3 / §6).
//!
//! Every positive test that builds an `Image` also runs it through
//! `Image::from_bytes`: an image the loader rejects is not a finished image.

use cyberspine_asm::assembler::{ImageGenError, generate_image, parse};
use cyberspine_vm::binfmt::image::{Image, SensorSource};
use cyberspine_vm::vm::jump::jump_target;
use cyberspine_vm::vm::op::{ImmType, Insn, OpCode};

/// The scalar header is currently mandatory (see `tests/header.rs`).
const HEADER: &str = ".layer 10\n.clock hz = 60\n.fuel 4096\n.max_wait 600.0\n";

fn source(body: &str) -> String {
    format!("{HEADER}{body}")
}

fn generate(src: &str) -> Result<Image, ImageGenError> {
    let asm = parse(src).expect("the source should parse");
    generate_image(&asm)
}

fn assemble(body: &str) -> Image {
    generate(&source(body)).expect("the source should generate an image")
}

fn generation_error(body: &str) -> ImageGenError {
    generate(&source(body)).expect_err("generation should fail")
}

// ---------------------------------------------------------------------------
// 1. Header, implicit HALT, loader round trip
// ---------------------------------------------------------------------------

#[test]
fn a_minimal_program_gets_the_implicit_halt_and_the_header() {
    let image = assemble(".text\nNOP\n");

    assert_eq!(image.header.layer, 10);
    assert_eq!(image.header.hz, 60);
    assert_eq!(image.header.fuel, 4096);
    assert_eq!(image.header.max_wait, 600.0);
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
    let image = assemble(
        "\
.muscle brow_l
.muscle brow_r
.group brows = brow_l, brow_r
.sensor_f smile
.sensor_i gaze_lock
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

    // The `imm` of each instruction is the logical index, in the order above.
    let imm: Vec<u32> = image.code.iter().map(|insn| insn.imm).collect();
    assert_eq!(imm, vec![0, 2, 3, 0, 1, 0, 0, 1, 0, 1, 0, 0]);

    let reloaded = Image::from_bytes(&image.to_bytes()).expect("the loader must accept it");
    assert_eq!(reloaded, image);
}

// ---------------------------------------------------------------------------
// 3. group expansion
// ---------------------------------------------------------------------------

#[test]
fn a_group_expands_to_one_excite_per_member() {
    let image = assemble(
        "\
.muscle a
.muscle b
.group g = a, b
.text
FEXCITE g, f1
FLUSH
HALT
",
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

.muscle brow_l
.muscle brow_r
.muscle mouth_corner_l
.muscle mouth_corner_r
.group brows = brow_l, brow_r
.group corners = mouth_corner_l, mouth_corner_r

.sensor_f smile
.sensor_f brow_raise
.sensor_i gaze_lock

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

    let asm = parse(src).expect("the documented example parses");
    let image = generate_image(&asm).expect("the documented example assembles");

    assert_eq!(image.code.len(), 28);
    assert_eq!(image.muscles.len(), 4);
    assert_eq!(image.sensors_f.len(), 7);
    assert_eq!(image.sensors_i.len(), 2);
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
    assert!(matches!(
        generation_error(".text\nFSENSE f1, nope\nHALT\n"),
        ImageGenError::UnknownSensorF(_)
    ));
    assert!(matches!(
        generation_error(".text\nISENSE i1, nope\nHALT\n"),
        ImageGenError::UnknownSensorI(_)
    ));
    assert!(matches!(
        generation_error(".text\nFVGET f1, nope\nHALT\n"),
        ImageGenError::UnknownVarF(_)
    ));
    assert!(matches!(
        generation_error(".text\nIVGET i1, nope\nHALT\n"),
        ImageGenError::UnknownVarI(_)
    ));
    assert!(matches!(
        generation_error(".text\nFEXCITE nope, f1\nHALT\n"),
        ImageGenError::UnknownExciteTarget(_)
    ));
}

#[test]
fn groups_are_only_legal_at_fexcite() {
    let err = generation_error(
        "\
.muscle a
.group g = a
.text
FSENSE f1, g
HALT
",
    );
    assert!(
        matches!(err, ImageGenError::GroupOutsideExcite(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn group_members_must_be_declared_muscles() {
    let err = generation_error(".group g = missing\n.text\nHALT\n");
    assert!(
        matches!(err, ImageGenError::UnknownGroupMember { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_group_name_cannot_collide_with_a_muscle() {
    let err = generation_error(".muscle a\n.group a = a\n.text\nHALT\n");
    assert!(
        matches!(err, ImageGenError::GroupNameCollision(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn an_explicit_sensor_cannot_shadow_a_muscle_readback() {
    let err = generation_error(".muscle a\n.sensor_f a\n.text\nHALT\n");
    assert!(
        matches!(err, ImageGenError::MuscleReadbackCollision(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn duplicate_declarations_are_rejected() {
    for body in [
        ".muscle a\n.muscle a\n.text\nHALT\n",
        ".sensor_f s\n.sensor_f s\n.text\nHALT\n",
        ".sensor_i s\n.sensor_i s\n.text\nHALT\n",
        ".var_f v = 0.0\n.var_f v = 1.0\n.text\nHALT\n",
        ".var_i v = 0\n.var_i v = 1\n.text\nHALT\n",
        ".muscle a\n.group g = a\n.group g = a\n.text\nHALT\n",
    ] {
        let err = generation_error(body);
        assert!(
            matches!(err, ImageGenError::DuplicateName(_)),
            "for {body:?}: unexpected error: {err}"
        );
    }
}

#[test]
fn reserved_names_are_rejected_in_every_table() {
    for body in [
        ".muscle dt\n.text\nHALT\n",
        ".muscle tick\n.text\nHALT\n",
        ".muscle f0\n.text\nHALT\n",
        ".muscle i63\n.text\nHALT\n",
        ".sensor_f dt\n.text\nHALT\n",
        ".sensor_i tick\n.text\nHALT\n",
        ".var_f dt = 0.0\n.text\nHALT\n",
        ".var_i tick = 0\n.text\nHALT\n",
    ] {
        let err = generation_error(body);
        assert!(
            matches!(err, ImageGenError::ReservedName(_)),
            "for {body:?}: unexpected error: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Pending-write dataflow (`docs/asm.md` §4.3 / §6.6)
// ---------------------------------------------------------------------------

#[test]
fn an_unflushed_excite_before_a_suspend_point_is_rejected() {
    let err = generation_error(
        "\
.muscle m
.text
FEXCITE m, f1
YIELD
HALT
",
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
    let err = generation_error(
        "\
.muscle m
.text
FEXCITE m, f1
HALT
",
    );
    assert!(matches!(err, ImageGenError::PendingNotFlushed { .. }));
}

#[test]
fn a_flush_on_only_one_branch_still_leaks() {
    let err = generation_error(
        "\
.muscle m
.text
FEXCITE m, f1
IJZ i1, L
FLUSH
L:
YIELD
HALT
",
    );
    assert!(
        matches!(err, ImageGenError::PendingNotFlushed { suspend_pc: 3, .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn a_flush_on_every_branch_is_accepted() {
    let image = assemble(
        "\
.muscle m
.text
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
    assert!(matches!(generate(zero_hz), Err(ImageGenError::ZeroHz)));

    let bad_max_wait = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait -1.0
.text
HALT
";
    assert!(matches!(
        generate(bad_max_wait),
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
