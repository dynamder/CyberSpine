//! The register file: all-zero at load, values kept, `f0`/`i0` hardwired
//! (`docs/isa.md` §2.1–§2.3).

use cyberspine_core::exec::registers::Registers;
use cyberspine_core::isa::register::RegId;

fn reg(number: u8) -> RegId {
    RegId::new(number).expect("number < RegId::COUNT")
}

#[test]
fn a_new_file_is_all_zero() {
    let regs = Registers::new();
    for number in 0..RegId::COUNT as u8 {
        assert_eq!(regs.get_f(reg(number)).to_bits(), 0);
        assert_eq!(regs.get_i(reg(number)), 0);
    }
}

#[test]
fn writes_are_visible_to_reads() {
    let mut regs = Registers::new();
    regs.set_f(reg(1), -2.5);
    regs.set_i(reg(63), -7);
    assert_eq!(regs.get_f(reg(1)).to_bits(), (-2.5f32).to_bits());
    assert_eq!(regs.get_i(reg(63)), -7);
}

#[test]
fn writes_to_the_hardwired_zeros_are_dropped() {
    let mut regs = Registers::new();
    regs.set_f(RegId::ZERO, 1.0);
    regs.set_i(RegId::ZERO, 1);
    assert_eq!(regs.get_f(RegId::ZERO).to_bits(), 0);
    assert_eq!(regs.get_i(RegId::ZERO), 0);
}

#[test]
fn f0_stays_zero_while_the_rest_of_the_file_changes() {
    let mut regs = Registers::new();
    for number in 1..RegId::COUNT as u8 {
        regs.set_f(reg(number), f32::from(number));
        regs.set_i(reg(number), i32::from(number));
    }
    assert_eq!(regs.get_f(RegId::ZERO).to_bits(), 0);
    assert_eq!(regs.get_i(RegId::ZERO), 0);
    assert_eq!(regs.get_f(reg(63)), 63.0);
    assert_eq!(regs.get_i(reg(63)), 63);
}
