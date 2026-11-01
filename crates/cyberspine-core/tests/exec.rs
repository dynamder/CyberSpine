//! The execution core against `docs/exec.md` §10: the 49 instruction bodies, fuel,
//! `WAIT`, `FLUSH`, suspension and the pure-function guarantee.

use cyberspine_core::binfmt::image::{Image, VarF, VarI};
use cyberspine_core::exec::{Fault, Instance, Program, Sensors, Status, Step};
use cyberspine_core::isa::register::RegId;
use cyberspine_core::isa::{Insn, OpCode, math};

fn reg(id: u8) -> RegId {
    RegId::new(id).expect("id < RegId::COUNT")
}

fn no_sensors() -> Sensors<'static> {
    Sensors::new(&[], &[])
}

fn halt() -> Insn {
    Insn::new(OpCode::Halt)
}

fn nop() -> Insn {
    Insn::new(OpCode::Nop)
}

fn fmovi(rd: u8, value: f32) -> Insn {
    Insn {
        rd: reg(rd),
        imm: value.to_bits(),
        ..Insn::new(OpCode::Fmovi)
    }
}

fn imovi(rd: u8, value: i32) -> Insn {
    Insn {
        rd: reg(rd),
        imm: value as u32,
        ..Insn::new(OpCode::Imovi)
    }
}

fn unary(op: OpCode, rd: u8, ra: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        ..Insn::new(op)
    }
}

fn binary(op: OpCode, rd: u8, ra: u8, rb: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        rb: reg(rb),
        ..Insn::new(op)
    }
}

fn ternary(op: OpCode, rd: u8, ra: u8, rb: u8, rc: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        rb: reg(rb),
        rc: reg(rc),
        ..Insn::new(op)
    }
}

fn compare(op: OpCode, ra: u8, rb: u8) -> Insn {
    Insn {
        ra: reg(ra),
        rb: reg(rb),
        ..Insn::new(op)
    }
}

fn jump(off: i32) -> Insn {
    Insn {
        imm: off as u32,
        ..Insn::new(OpCode::Jmp)
    }
}

fn wait(seconds: f32) -> Insn {
    Insn {
        imm: seconds.to_bits(),
        ..Insn::new(OpCode::Wait)
    }
}

fn image(code: Vec<Insn>) -> Image {
    Image::new(code)
}

fn fed(code: Vec<Insn>, fuel: u32) -> Image {
    let mut image = Image::new(code);
    image.header.fuel = fuel;
    image
}

fn clocked(code: Vec<Insn>, hz: u32) -> Image {
    let mut image = Image::new(code);
    image.header.hz = hz;
    image
}

// ---------------------------------------------------------------------------
// Lifecycle (`docs/isa.md` §8.3)
// ---------------------------------------------------------------------------

#[test]
fn halt_terminates_and_a_second_resume_is_dead() {
    let image = image(vec![halt()]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.status(), Status::Halted);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Dead);
    assert_eq!(instance.status(), Status::Halted, "Dead must not change the status");
}

#[test]
fn a_faulted_instance_stays_faulted() {
    let image = fed(vec![nop()], 1);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&no_sensors(), 0),
        Step::Faulted(Fault::FuelExhausted { pc: 1 })
    );
    assert_eq!(instance.status(), Status::Faulted);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Dead);
}

#[test]
fn yield_suspends_at_the_next_instruction_and_keeps_state() {
    let image = image(vec![imovi(1, 42), Insn::new(OpCode::Yield), halt()]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Yielded);
    assert_eq!(instance.status(), Status::Suspended);
    assert_eq!(instance.pc(), 2, "the resume point is the instruction after YIELD");
    assert_eq!(instance.registers().get_i(reg(1)), 42);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
}

#[test]
fn the_whole_register_file_survives_a_suspend() {
    let image = image(vec![
        fmovi(1, 1.5),
        imovi(2, -7),
        fmovi(63, 2.5),
        imovi(63, 2),
        Insn::new(OpCode::Yield),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Yielded);
    assert_eq!(instance.registers().get_f(reg(1)), 1.5);
    assert_eq!(instance.registers().get_i(reg(2)), -7);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(63)), 2.5);
    assert_eq!(instance.registers().get_i(reg(63)), 2);
}

#[test]
fn writes_to_f0_and_i0_are_dropped_by_every_instruction() {
    let image = image(vec![fmovi(0, 1.0), imovi(0, 1), halt()]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(RegId::ZERO).to_bits(), 0);
    assert_eq!(instance.registers().get_i(RegId::ZERO), 0);
}

// ---------------------------------------------------------------------------
// Data movement and arithmetic (`docs/isa.md` §4.1, §4.5)
// ---------------------------------------------------------------------------

#[test]
fn moves_and_immediates() {
    let image = image(vec![
        fmovi(1, 1.5),
        imovi(2, -3),
        unary(OpCode::Fmov, 3, 1),
        unary(OpCode::Imov, 4, 2),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(3)), 1.5);
    assert_eq!(instance.registers().get_i(reg(4)), -3);
}

#[test]
fn float_arithmetic_is_ieee_binary32() {
    let image = image(vec![
        fmovi(1, 4.0),
        fmovi(2, 2.0),
        binary(OpCode::Fadd, 3, 1, 2),
        binary(OpCode::Fsub, 4, 1, 2),
        binary(OpCode::Fmul, 5, 1, 2),
        binary(OpCode::Fdiv, 6, 1, 2),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(3)), 6.0);
    assert_eq!(instance.registers().get_f(reg(4)), 2.0);
    assert_eq!(instance.registers().get_f(reg(5)), 8.0);
    assert_eq!(instance.registers().get_f(reg(6)), 2.0);
}

#[test]
fn integer_arithmetic_wraps_instead_of_trapping() {
    let image = image(vec![
        imovi(1, i32::MAX),
        imovi(2, 1),
        binary(OpCode::Iadd, 3, 1, 2),
        binary(OpCode::Isub, 4, 1, 2),
        imovi(5, i32::MIN),
        binary(OpCode::Imul, 6, 5, 2),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_i(reg(3)), i32::MIN);
    assert_eq!(instance.registers().get_i(reg(4)), i32::MAX - 1);
    assert_eq!(instance.registers().get_i(reg(6)), i32::MIN);
}

#[test]
fn min_and_max_follow_ieee_754_2019_minimum_and_maximum() {
    let nan = f32::NAN;
    let cases: [(f32, f32, f32, f32); 4] = [
        (-0.0, 0.0, -0.0, 0.0),
        (1.0, 2.0, 1.0, 2.0),
        (nan, 1.0, nan, nan),
        (-1.0, -2.0, -2.0, -1.0),
    ];

    for (a, b, min, max) in cases {
        let image = image(vec![
            fmovi(2, a),
            fmovi(3, b),
            binary(OpCode::Fmin, 1, 2, 3),
            binary(OpCode::Fmax, 4, 2, 3),
            halt(),
        ]);
        let program = Program::new(&image);
        let mut instance = Instance::new(&program);

        assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
        assert_eq!(
            instance.registers().get_f(reg(1)).to_bits(),
            min.to_bits(),
            "FMIN({a}, {b})"
        );
        assert_eq!(
            instance.registers().get_f(reg(4)).to_bits(),
            max.to_bits(),
            "FMAX({a}, {b})"
        );
    }
}

#[test]
fn neg_and_abs_are_bit_operations() {
    let image = image(vec![
        fmovi(1, 2.5),
        unary(OpCode::Fneg, 2, 1),
        unary(OpCode::Fabs, 3, 2),
        fmovi(4, f32::NAN),
        unary(OpCode::Fneg, 5, 4),
        unary(OpCode::Fabs, 6, 5),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(2)), -2.5);
    assert_eq!(instance.registers().get_f(reg(3)), 2.5);
    assert!(instance.registers().get_f(reg(5)).is_nan());
    assert!(instance.registers().get_f(reg(6)).is_nan());
}

#[test]
fn sqrt_sin_cos_and_frac_use_the_frozen_kernel() {
    let image = image(vec![
        fmovi(1, 4.0),
        unary(OpCode::Fsqrt, 2, 1),
        fmovi(3, 0.0),
        unary(OpCode::Fsin, 4, 3),
        unary(OpCode::Fcos, 5, 3),
        fmovi(6, -0.25),
        unary(OpCode::Ffrac, 7, 6),
        fmovi(8, -1e-30),
        unary(OpCode::Ffrac, 9, 8),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(2)), 2.0);
    assert_eq!(instance.registers().get_f(reg(4)).to_bits(), 0);
    assert_eq!(instance.registers().get_f(reg(5)), 1.0);
    assert_eq!(instance.registers().get_f(reg(7)), 0.75);
    assert_eq!(
        instance.registers().get_f(reg(9)),
        math::ONE_MINUS_ULP,
        "the fractional part of a tiny negative rounds up to 1.0 and must be clamped"
    );
}

#[test]
fn madd_rounds_twice_and_is_never_fused() {
    let a = f32::from_bits(0x3F80_0001);
    let b = f32::from_bits(0x3F7F_FFFF);
    let fused = a.mul_add(b, -1.0);

    let image = image(vec![
        fmovi(1, a),
        fmovi(2, b),
        fmovi(3, -1.0),
        ternary(OpCode::Fmadd, 4, 1, 2, 3),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(
        instance.registers().get_f(reg(4)).to_bits(),
        0,
        "a*b rounds to 1.0 first, so the sum is zero"
    );
    assert_ne!(fused.to_bits(), 0, "this input must distinguish fused from unfused");
}

#[test]
fn lerp_follows_the_specified_order_and_clamp_is_a_composition() {
    let image = image(vec![
        fmovi(1, 1.0),
        fmovi(2, 2.0),
        fmovi(3, 0.5),
        ternary(OpCode::Flerp, 4, 1, 2, 3),
        fmovi(5, 5.0),
        fmovi(6, 1.0),
        fmovi(7, 0.0),
        ternary(OpCode::Fclamp, 8, 5, 6, 7),
        fmovi(9, 0.5),
        fmovi(10, 0.0),
        fmovi(11, 1.0),
        ternary(OpCode::Fclamp, 12, 9, 10, 11),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(4)), 1.5);
    assert_eq!(
        instance.registers().get_f(reg(8)),
        0.0,
        "lo > hi clamps to hi"
    );
    assert_eq!(instance.registers().get_f(reg(12)), 0.5);
}

#[test]
fn conversions_saturate_and_round_to_nearest_even() {
    let image = image(vec![
        fmovi(1, 2.7),
        unary(OpCode::Ftoi, 2, 1),
        fmovi(3, -2.7),
        unary(OpCode::Ftoi, 4, 3),
        fmovi(5, 1e30),
        unary(OpCode::Ftoi, 6, 5),
        fmovi(7, -1e30),
        unary(OpCode::Ftoi, 8, 7),
        fmovi(9, f32::NAN),
        unary(OpCode::Ftoi, 10, 9),
        imovi(11, 16_777_217),
        unary(OpCode::Itof, 12, 11),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_i(reg(2)), 2);
    assert_eq!(instance.registers().get_i(reg(4)), -2);
    assert_eq!(instance.registers().get_i(reg(6)), i32::MAX);
    assert_eq!(instance.registers().get_i(reg(8)), i32::MIN);
    assert_eq!(instance.registers().get_i(reg(10)), 0);
    assert_eq!(instance.registers().get_f(reg(12)), 16_777_216.0);
}

// ---------------------------------------------------------------------------
// Control flow (`docs/isa.md` §4.6, §5)
// ---------------------------------------------------------------------------

/// Run a program laid out so the condition at `p` picks between two markers:
///
/// ```text
/// p:     <condition, off +2>   taken -> p+3, not taken -> p+1
/// p + 1: IMOVI i20, 0
/// p + 2: JMP +1                -> p+4
/// p + 3: IMOVI i20, 1
/// p + 4: HALT
/// ```
fn condition_taken(setup: Vec<Insn>, condition: Insn) -> bool {
    let p = setup.len();
    let mut code = setup;
    code.push(Insn {
        imm: 2u32,
        ..condition
    });
    code.push(imovi(20, 0));
    code.push(jump(1));
    code.push(imovi(20, 1));
    code.push(halt());

    let image = image(code);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.pc(), p + 4);
    instance.registers().get_i(reg(20)) == 1
}

#[test]
fn integer_conditionals_branch_on_their_own_comparison() {
    let setup = vec![imovi(1, 3), imovi(2, 5)];

    assert!(condition_taken(setup.clone(), compare(OpCode::Ilt, 1, 2)));
    assert!(condition_taken(setup.clone(), compare(OpCode::Ile, 1, 2)));
    assert!(!condition_taken(setup.clone(), compare(OpCode::Igt, 1, 2)));
    assert!(!condition_taken(setup.clone(), compare(OpCode::Ige, 1, 2)));
    assert!(!condition_taken(setup.clone(), compare(OpCode::Ieq, 1, 2)));
    assert!(condition_taken(setup, compare(OpCode::Ine, 1, 2)));
}

#[test]
fn float_conditionals_are_false_when_an_operand_is_nan() {
    let setup = vec![fmovi(1, 1.0), fmovi(2, 2.0)];
    assert!(condition_taken(setup.clone(), compare(OpCode::Flt, 1, 2)));
    assert!(condition_taken(setup.clone(), compare(OpCode::Fle, 1, 2)));
    assert!(!condition_taken(setup.clone(), compare(OpCode::Fgt, 1, 2)));
    assert!(!condition_taken(setup, compare(OpCode::Fge, 1, 2)));

    let nan = vec![fmovi(1, f32::NAN), fmovi(2, 1.0)];
    for op in [OpCode::Flt, OpCode::Fle, OpCode::Fgt, OpCode::Fge] {
        assert!(
            !condition_taken(nan.clone(), compare(op, 1, 2)),
            "{op:?} must not branch when an operand is NaN"
        );
    }
}

#[test]
fn ijz_and_ijnz_test_the_whole_integer() {
    assert!(condition_taken(
        vec![imovi(1, 0)],
        Insn {
            ra: reg(1),
            ..Insn::new(OpCode::Ijz)
        }
    ));
    assert!(condition_taken(
        vec![imovi(1, -1)],
        Insn {
            ra: reg(1),
            ..Insn::new(OpCode::Ijnz)
        }
    ));
    assert!(!condition_taken(
        vec![imovi(1, 0)],
        Insn {
            ra: reg(1),
            ..Insn::new(OpCode::Ijnz)
        }
    ));
}

#[test]
fn a_backward_jump_drives_a_counted_loop() {
    let image = image(vec![
        imovi(1, 3),
        imovi(2, 1),
        binary(OpCode::Isub, 1, 1, 2),
        Insn {
            ra: reg(1),
            imm: (-2i32) as u32,
            ..Insn::new(OpCode::Ijnz)
        },
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_i(reg(1)), 0);
}

// ---------------------------------------------------------------------------
// Sensors, variables, muscles (`docs/isa.md` §G7, §G10, §8.2)
// ---------------------------------------------------------------------------

#[test]
fn senses_read_the_frozen_snapshot() {
    let floats = [1.5, -2.5];
    let ints = [7, -9];
    let image = image(vec![
        Insn {
            rd: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fsense)
        },
        Insn {
            rd: reg(2),
            imm: 1,
            ..Insn::new(OpCode::Fsense)
        },
        Insn {
            rd: reg(3),
            imm: 1,
            ..Insn::new(OpCode::Isense)
        },
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let sensors = Sensors::new(&floats, &ints);
    assert_eq!(instance.resume(&sensors, 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(1)), 1.5);
    assert_eq!(instance.registers().get_f(reg(2)), -2.5);
    assert_eq!(instance.registers().get_i(reg(3)), -9);
}

#[test]
fn an_instance_cannot_read_back_its_own_excitation() {
    let mut image = Image::new(vec![
        fmovi(1, 1.0),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn {
            rd: reg(2),
            imm: 0,
            ..Insn::new(OpCode::Fsense)
        },
        halt(),
    ]);
    image.push_muscle("m0");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let floats = [0.0];
    let sensors = Sensors::new(&floats, &[]);
    assert_eq!(instance.resume(&sensors, 0), Step::Halted);
    assert_eq!(
        instance.registers().get_f(reg(2)).to_bits(),
        0,
        "the snapshot was taken before this excitation, so the feedback is one tick late"
    );
}

#[test]
fn variables_take_their_initial_value_from_the_image() {
    let mut image = Image::new(vec![
        Insn {
            rd: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fvget)
        },
        Insn {
            rd: reg(2),
            imm: 0,
            ..Insn::new(OpCode::Ivget)
        },
        halt(),
    ]);
    image.vars_f.push(VarF {
        name: "height".into(),
        init: 2.5,
    });
    image.vars_i.push(VarI {
        name: "count".into(),
        init: -4,
    });

    let program = Program::new(&image);
    let mut instance = Instance::new(&program);
    assert_eq!(instance.vars_f(), [2.5]);
    assert_eq!(instance.vars_i(), [-4]);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(1)), 2.5);
    assert_eq!(instance.registers().get_i(reg(2)), -4);
}

#[test]
fn variables_survive_a_suspend() {
    let mut image = Image::new(vec![
        fmovi(1, 8.0),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fvset)
        },
        Insn::new(OpCode::Yield),
        Insn {
            rd: reg(2),
            imm: 0,
            ..Insn::new(OpCode::Fvget)
        },
        halt(),
    ]);
    image.vars_f.push(VarF {
        name: "state".into(),
        init: 0.0,
    });

    let program = Program::new(&image);
    let mut instance = Instance::new(&program);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Yielded);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
    assert_eq!(instance.vars_f(), [8.0]);
    assert_eq!(instance.registers().get_f(reg(2)), 8.0);
}

#[test]
fn excite_alone_publishes_nothing() {
    let mut image = Image::new(vec![
        fmovi(1, 0.25),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        halt(),
    ]);
    image.push_muscle("m0");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let zeros = [0.0];
    assert_eq!(instance.resume(&Sensors::new(&zeros, &[]), 0), Step::Halted);
    assert_eq!(instance.published(0), None);
    assert_eq!(instance.muscle_count(), 1);
}

#[test]
fn flush_publishes_only_what_was_pending_and_keeps_it_afterwards() {
    let mut image = Image::new(vec![
        fmovi(1, 0.5),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn::new(OpCode::Flush),
        fmovi(2, 0.9),
        Insn {
            ra: reg(2),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        halt(),
    ]);
    image.push_muscle("m0");
    image.push_muscle("m1");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let zeros = [0.0; 2];
    assert_eq!(instance.resume(&Sensors::new(&zeros, &[]), 0), Step::Halted);
    assert_eq!(
        instance.published(0),
        Some(0.5),
        "the 0.9 written after FLUSH is still pending"
    );
    assert_eq!(instance.published(1), None, "m1 was never flushed");
}

#[test]
fn a_published_value_stays_across_a_flush_of_another_muscle() {
    let mut image = Image::new(vec![
        fmovi(1, 0.5),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn::new(OpCode::Flush),
        Insn::new(OpCode::Yield),
        fmovi(2, 0.25),
        Insn {
            ra: reg(2),
            imm: 1,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn::new(OpCode::Flush),
        halt(),
    ]);
    image.push_muscle("m0");
    image.push_muscle("m1");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let zeros = [0.0; 2];
    let sensors = Sensors::new(&zeros, &[]);
    assert_eq!(instance.resume(&sensors, 0), Step::Yielded);
    assert_eq!(instance.published(0), Some(0.5));
    assert_eq!(instance.published(1), None);

    assert_eq!(instance.resume(&sensors, 0), Step::Halted);
    assert_eq!(instance.published(0), Some(0.5), "G10: a flushed value holds");
    assert_eq!(instance.published(1), Some(0.25));
}

// ---------------------------------------------------------------------------
// fuel (`docs/exec.md` §6)
// ---------------------------------------------------------------------------

#[test]
fn fuel_stops_exactly_before_the_next_instruction() {
    let image = fed(vec![nop(), nop(), nop()], 2);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&no_sensors(), 0),
        Step::Faulted(Fault::FuelExhausted { pc: 2 })
    );
    assert_eq!(instance.fault(), Some(Fault::FuelExhausted { pc: 2 }));
}

#[test]
fn zero_fuel_faults_at_the_entry_without_executing_anything() {
    let image = fed(vec![imovi(1, 1), halt()], 0);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&no_sensors(), 0),
        Step::Faulted(Fault::FuelExhausted { pc: 0 })
    );
    assert_eq!(instance.registers().get_i(reg(1)), 0, "no instruction ran");
}

#[test]
fn a_self_jump_burns_fuel_at_the_same_pc() {
    let image = fed(vec![jump(-1)], 3);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&no_sensors(), 0),
        Step::Faulted(Fault::FuelExhausted { pc: 0 })
    );
}

#[test]
fn every_executed_instruction_costs_one_including_jumps() {
    let image = fed(
        vec![
            imovi(2, 1),
            binary(OpCode::Iadd, 1, 1, 2),
            jump(-2),
            halt(),
        ],
        6,
    );
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&no_sensors(), 0),
        Step::Faulted(Fault::FuelExhausted { pc: 2 })
    );
    assert_eq!(instance.registers().get_i(reg(1)), 3);
}

#[test]
fn fuel_is_refilled_on_every_resume() {
    let image = fed(
        vec![nop(), Insn::new(OpCode::Yield), nop(), halt()],
        2,
    );
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&no_sensors(), 0), Step::Yielded);
    assert_eq!(instance.resume(&no_sensors(), 0), Step::Halted);
}

// ---------------------------------------------------------------------------
// WAIT (`docs/exec.md` §7)
// ---------------------------------------------------------------------------

fn waited_at(seconds: f32, hz: u32, now: u64) -> u64 {
    let image = clocked(vec![wait(seconds), halt()], hz);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    match instance.resume(&no_sensors(), now) {
        Step::Waited { until } => until,
        other => panic!("expected a wait, got {other:?}"),
    }
}

#[test]
fn wait_rounds_ticks_up_never_down() {
    assert_eq!(waited_at(0.0, 60, 100), 100);
    assert_eq!(waited_at(1.0 / 64.0, 64, 100), 101, "exactly one tick");
    assert_eq!(waited_at(1.0, 60, 100), 160);
    assert_eq!(waited_at(1e-9, 60, 100), 101, "any positive duration is one tick");
    assert_eq!(waited_at(-1.0, 60, 100), 100, "a negative duration is zero ticks");
    assert_eq!(waited_at(f32::NAN, 60, 100), 100);
    assert_eq!(waited_at(0.5, 30, 0), 15);
}

#[test]
fn wait_rounds_a_f32_a_whole_tick_up_because_ceil_amplifies_the_error() {
    // `f32(0.3)` is 0.300000012, so the product is 18.0000007 and the ceiling is
    // 19 — `docs/exec.md` §7.4's stated trade: never early, at most one tick
    // late. The same reasons make `f32(1/60)` wait two ticks.
    assert_eq!(waited_at(0.3, 60, 100), 119);
    assert_eq!(waited_at(1.0 / 60.0, 60, 100), 102);
}

#[test]
fn wait_saturates_instead_of_overflowing() {
    assert_eq!(waited_at(f32::MAX, 60, u64::MAX - 10), u64::MAX);
    assert_eq!(waited_at(1.0, 60, u64::MAX - 5), u64::MAX);
}

#[test]
fn a_waited_instance_runs_again_when_the_host_decides() {
    let image = image(vec![wait(0.25), fmovi(1, 1.0), halt()]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let Step::Waited { until } = instance.resume(&no_sensors(), 10) else {
        panic!("expected a wait");
    };
    assert_eq!(until, 25);
    assert_eq!(instance.wake_at(), 25);
    assert_eq!(instance.status(), Status::Suspended);
    assert_eq!(instance.pc(), 1, "resume point is the instruction after WAIT");

    assert_eq!(instance.resume(&no_sensors(), 25), Step::Halted);
    assert_eq!(instance.registers().get_f(reg(1)), 1.0);
}

// ---------------------------------------------------------------------------
// The pure-function guarantee (`docs/exec.md` §1, §10)
// ---------------------------------------------------------------------------

#[test]
fn the_same_inputs_produce_bit_identical_commands() {
    let mut image = Image::new(vec![
        fmovi(1, 0.25),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn::new(OpCode::Flush),
        Insn::new(OpCode::Yield),
        fmovi(2, 0.5),
        Insn {
            ra: reg(2),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn::new(OpCode::Flush),
        halt(),
    ]);
    image.push_muscle("m0");

    let zeros = [0.0];
    let sensors = Sensors::new(&zeros, &[]);
    let trace = |image: &Image| {
        let program = Program::new(image);
        let mut instance = Instance::new(&program);
        let mut steps = Vec::new();
        let mut published = Vec::new();
        loop {
            let step = instance.resume(&sensors, 0);
            published.push(instance.published(0).map(f32::to_bits));
            steps.push(step);
            if !matches!(step, Step::Yielded) {
                break;
            }
        }
        (steps, published)
    };

    assert_eq!(trace(&image), trace(&image));
    assert_eq!(
        trace(&image).1,
        vec![Some(0.25f32.to_bits()), Some(0.5f32.to_bits())]
    );
}

#[test]
#[should_panic(expected = "host contract")]
fn a_short_float_snapshot_is_a_host_violation() {
    let mut image = Image::new(vec![halt()]);
    image.push_muscle("m0");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let _ = instance.resume(&no_sensors(), 0);
}

#[test]
#[should_panic(expected = "host contract")]
fn a_short_integer_snapshot_is_a_host_violation() {
    let mut image = Image::new(vec![halt()]);
    image.sensors_i.push(cyberspine_core::binfmt::image::Sensor {
        name: "tick".into(),
        source: cyberspine_core::binfmt::image::SensorSource::BuiltinTick,
    });
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let _ = instance.resume(&no_sensors(), 0);
}
