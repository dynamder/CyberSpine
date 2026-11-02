//! Runs the execution core on a bare-metal Cortex-M4 and reports through
//! semihosting. The images are hand-built in RAM; one golden `.csp` fixture goes
//! through the real loader so blake3 and the byte decoder are covered too.

#![no_std]
#![no_main]

extern crate alloc;

mod allocator;

use alloc::vec;
use core::panic::PanicInfo;

use cortex_m_rt::entry;
use cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, ExitStatus, exit};
use cortex_m_semihosting::hprintln;

use cyberspine_core::binfmt::image::Image;
use cyberspine_core::exec::{Fault, Instance, Program, Sensors, Step};
use cyberspine_core::isa::register::RegId;
use cyberspine_core::isa::{Insn, OpCode};
use cyberspine_core::rig::Name;

fn muscle_name(text: &str) -> Name {
    Name::new(text).expect("the firmware's hand-built images use literal names")
}

const CHECKS: [(&str, fn()); 6] = [
    ("loader", check_loader),
    ("arithmetic", check_arithmetic),
    ("control", check_control),
    ("commands", check_commands),
    ("suspend", check_suspend),
    ("fuel", check_fuel),
];

#[entry]
fn main() -> ! {
    for (name, run) in CHECKS {
        run();
        hprintln!("ok: {}", name);
    }
    hprintln!("embedded: {} checks passed", CHECKS.len());
    finish(EXIT_SUCCESS)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    hprintln!("embedded: panic at {:?}", info.location());
    finish(EXIT_FAILURE)
}

fn finish(status: ExitStatus) -> ! {
    exit(status);
    loop {}
}

fn reg(id: u8) -> RegId {
    RegId::new(id).expect("register below RegId::COUNT")
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

fn wait(seconds: f32) -> Insn {
    Insn {
        imm: seconds.to_bits(),
        ..Insn::new(OpCode::Wait)
    }
}

fn halt() -> Insn {
    Insn::new(OpCode::Halt)
}

fn check_loader() {
    let bytes = include_bytes!("../../crates/cyberspine-core/tests/fixtures/valid_all.csp");
    let image = Image::from_bytes(bytes).expect("the golden fixture must load");
    assert!(!image.code.is_empty());
}

fn check_arithmetic() {
    let image = Image::new(vec![
        fmovi(1, 1.5),
        fmovi(2, 4.0),
        binary(OpCode::Fadd, 3, 1, 2),
        binary(OpCode::Fmul, 4, 3, 2),
        unary(OpCode::Fsqrt, 5, 2),
        unary(OpCode::Ftoi, 6, 3),
        fmovi(7, 2.0),
        unary(OpCode::Fsqrt, 8, 7),
        fmovi(10, -1e-30),
        unary(OpCode::Ffrac, 11, 10),
        unary(OpCode::Fsin, 9, 1),
        halt(),
    ]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);
    assert_eq!(instance.resume(&program, &Sensors::new(&[], &[]), 0), Step::Halted);

    let regs = instance.registers();
    assert_eq!(regs.get_f(reg(3)), 5.5);
    assert_eq!(regs.get_f(reg(4)), 22.0);
    assert_eq!(regs.get_f(reg(5)), 2.0);
    assert_eq!(regs.get_i(reg(6)), 5);
    assert_eq!(
        regs.get_f(reg(8)).to_bits(),
        0x3FB5_04F3,
        "sqrt(2) must be correctly rounded on every target"
    );
    assert_eq!(
        regs.get_f(reg(11)).to_bits(),
        0x3F7F_FFFF,
        "FFRAC clamps to next_down(1.0)"
    );
    let sine = regs.get_f(reg(9));
    assert!(sine > -1.0 && sine < 1.0);
}

fn check_control() {
    let image = Image::new(vec![
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
    assert_eq!(instance.resume(&program, &Sensors::new(&[], &[]), 0), Step::Halted);
    assert_eq!(instance.registers().get_i(reg(1)), 0);
}

fn check_commands() {
    let mut image = Image::new(vec![
        fmovi(1, 0.5),
        Insn {
            ra: reg(1),
            imm: 0,
            ..Insn::new(OpCode::Fexcite)
        },
        Insn::new(OpCode::Flush),
        halt(),
    ]);
    image.push_muscle(muscle_name("m0"));
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    let sensed = [0.0];
    assert_eq!(instance.resume(&program, &Sensors::new(&sensed, &[]), 0), Step::Halted);
    assert_eq!(instance.muscle_count(), 1);
    assert_eq!(instance.published(0), Some(0.5));
}

fn check_suspend() {
    let image = Image::new(vec![imovi(1, 9), Insn::new(OpCode::Yield), wait(0.3), halt()]);
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(instance.resume(&program, &Sensors::new(&[], &[]), 100), Step::Yielded);
    assert_eq!(
        instance.registers().get_i(reg(1)),
        9,
        "registers survive a suspend"
    );

    match instance.resume(&program, &Sensors::new(&[], &[]), 100) {
        Step::Waited { until } => assert_eq!(until, 119, "f32(0.3) at 60 Hz is 19 ticks"),
        other => panic!("expected a wait, got {other:?}"),
    }

    assert_eq!(instance.resume(&program, &Sensors::new(&[], &[]), 119), Step::Halted);
}

fn check_fuel() {
    let mut image = Image::new(vec![
        Insn::new(OpCode::Nop),
        Insn::new(OpCode::Nop),
        Insn::new(OpCode::Nop),
    ]);
    image.header.fuel = 2;
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&program, &Sensors::new(&[], &[]), 0),
        Step::Faulted(Fault::FuelExhausted { pc: 2 })
    );
}
