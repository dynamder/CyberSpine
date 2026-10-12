//! Tests for the instruction-word decoder, `Insn::decode`.
//!
//! Kept outside the crate sources on purpose: they drive `Insn` only through its
//! public API and re-derive the bit layout independently, so they check the
//! decoder against the written spec rather than against itself.
//!
//! Scope: **decoding only**. `Insn::decode` and its typed accessors. Nothing
//! here validates an instruction or does jump-target arithmetic — those layers
//! sit above this one.
//!
//! Normative references:
//!   * `docs/isa.md` §1.1 — 8/6/6/6/6/32 layout, 49 opcodes
//!   * `docs/isa.md` §1.2 — per-opcode `imm` interpretation
//!   * `docs/isa.md` §4    — opcode table

use cyberspine_vm::register::RegId;
use cyberspine_vm::vm::op::{DecodeError, Insn, OpCode};

// ---------------------------------------------------------------------------
// Independent re-derivation of the encoding (isa.md §1.1).
// ---------------------------------------------------------------------------

const OP_SHIFT: u32 = 56;
const RD_SHIFT: u32 = 50;
const RA_SHIFT: u32 = 44;
const RB_SHIFT: u32 = 38;
const RC_SHIFT: u32 = 32;
const REG_MASK: u64 = 0x3F;

/// Sentinel operands. All four register fields are distinct so a decoder reading
/// the wrong field cannot accidentally produce the expected value.
const S_RD: u64 = 1;
const S_RA: u64 = 2;
const S_RB: u64 = 3;
const S_RC: u64 = 4;

/// `0x3F80_0000` is simultaneously `1.0f32`, `1065353216u32` and a legal
/// positive jump offset, so one sentinel exercises all three `imm` readings.
const IMM: u32 = 0x3F80_0000;

/// Register-slot presence bits.
const B_RD: u8 = 1;
const B_RA: u8 = 2;
const B_RB: u8 = 4;
const B_RC: u8 = 8;

/// `(opcode, register slots, uses imm)` — re-derived from `isa.md` §4, without
/// consulting the generated `isa_table!`. This is the independent spec that
/// makes the assertions below non-circular.
const FIELDS: [(u8, u8, bool); 49] = [
    (0x00, B_RD, true),           // FMOVI  rd, imm(f32 bits)
    (0x01, B_RD, true),           // IMOVI  rd, imm(i32)
    (0x02, B_RD | B_RA, false),   // FMOV   rd, ra
    (0x03, B_RD | B_RA, false),   // IMOV   rd, ra
    (0x10, B_RD | B_RA | B_RB, false), // FADD
    (0x11, B_RD | B_RA | B_RB, false), // FSUB
    (0x12, B_RD | B_RA | B_RB, false), // FMUL
    (0x13, B_RD | B_RA | B_RB, false), // FDIV
    (0x14, B_RD | B_RA | B_RB, false), // FMIN
    (0x15, B_RD | B_RA | B_RB, false), // FMAX
    (0x20, B_RD | B_RA, false),   // FNEG
    (0x21, B_RD | B_RA, false),   // FABS
    (0x22, B_RD | B_RA, false),   // FSQRT
    (0x23, B_RD | B_RA, false),   // FSIN
    (0x24, B_RD | B_RA, false),   // FCOS
    (0x25, B_RD | B_RA, false),   // FFRAC
    (0x30, B_RD | B_RA | B_RB | B_RC, false), // FMADD
    (0x31, B_RD | B_RA | B_RB | B_RC, false), // FLERP
    (0x32, B_RD | B_RA | B_RB | B_RC, false), // FCLAMP
    (0x40, B_RD | B_RA | B_RB, false), // IADD
    (0x41, B_RD | B_RA | B_RB, false), // ISUB
    (0x42, B_RD | B_RA | B_RB, false), // IMUL
    (0x43, B_RD | B_RA, false),   // ITOF
    (0x44, B_RD | B_RA, false),   // FTOI
    (0x50, 0, true),              // JMP   imm
    (0x51, B_RA, true),           // IJZ   ra, imm
    (0x52, B_RA, true),           // IJNZ  ra, imm
    (0x53, B_RA | B_RB, true),    // FLT
    (0x54, B_RA | B_RB, true),    // FLE
    (0x55, B_RA | B_RB, true),    // FGT
    (0x56, B_RA | B_RB, true),    // FGE
    (0x57, B_RA | B_RB, true),    // ILT
    (0x58, B_RA | B_RB, true),    // ILE
    (0x59, B_RA | B_RB, true),    // IGT
    (0x5A, B_RA | B_RB, true),    // IGE
    (0x5B, B_RA | B_RB, true),    // IEQ
    (0x5C, B_RA | B_RB, true),    // INE
    (0x60, B_RD, true),           // FSENSE rd, imm(sensor id)
    (0x61, B_RD, true),           // ISENSE rd, imm(sensor id)
    (0x70, 0, false),             // YIELD
    (0x71, 0, true),              // WAIT  imm(f32 bits)
    (0x80, B_RA, true),           // FEXCITE imm(muscle id), ra
    (0x90, B_RD, true),           // FVGET rd, imm(var id)
    (0x91, B_RA, true),           // FVSET imm(var id), ra
    (0x92, B_RD, true),           // IVGET rd, imm(var id)
    (0x93, B_RA, true),           // IVSET imm(var id), ra
    (0xA0, 0, false),             // HALT
    (0xA1, 0, false),             // NOP
    (0xA2, 0, false),             // FLUSH published <- pending
];

const JUMP_OPCODES: [u8; 13] = [
    0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C,
];

fn word(op: u8, rd: u64, ra: u64, rb: u64, rc: u64, imm: u32) -> u64 {
    debug_assert!(rd <= REG_MASK && ra <= REG_MASK && rb <= REG_MASK && rc <= REG_MASK);
    ((op as u64) << OP_SHIFT)
        | (rd << RD_SHIFT)
        | (ra << RA_SHIFT)
        | (rb << RB_SHIFT)
        | (rc << RC_SHIFT)
        | (imm as u64)
}

/// `Insn::decode` for words that are known to be legal. A refusal here is a
/// broken fixture, not a result worth asserting on, so it panics with the cause.
fn decode(op: u8, rd: u64, ra: u64, rb: u64, rc: u64, imm: u32) -> Insn {
    Insn::decode(word(op, rd, ra, rb, rc, imm))
        .unwrap_or_else(|e| panic!("opcode {op:#04x} must decode, but was rejected: {e}"))
}

// ---------------------------------------------------------------------------
// Word layout
// ---------------------------------------------------------------------------

#[test]
fn bit_fields_do_not_bleed_into_each_other() {
    // Only the opcode set: every operand field must read back as zero.
    let op_only = decode(0xA1, 0, 0, 0, 0, 0);
    assert_eq!(op_only.op, OpCode::Nop);
    assert_eq!(
        (op_only.rd, op_only.ra, op_only.rb, op_only.rc),
        (RegId::ZERO, RegId::ZERO, RegId::ZERO, RegId::ZERO),
        "opcode bit(s) leaked into the register fields"
    );
    assert_eq!(op_only.imm, 0);

    // Only `imm` set, on an opcode that uses it.
    let imm_only = decode(0x01, 0, 0, 0, 0, u32::MAX);
    assert_eq!(imm_only.op, OpCode::Imovi);
    assert_eq!(imm_only.rd, RegId::ZERO);
    assert_eq!(imm_only.imm_i32(), -1);

    // Each register field independently, on an opcode that uses all four.
    for (insn, expected) in [
        (decode(0x30, REG_MASK, 0, 0, 0, 0), (REG_MASK as u8, 0, 0, 0)),
        (decode(0x30, 0, REG_MASK, 0, 0, 0), (0, REG_MASK as u8, 0, 0)),
        (decode(0x30, 0, 0, REG_MASK, 0, 0), (0, 0, REG_MASK as u8, 0)),
        (decode(0x30, 0, 0, 0, REG_MASK, 0), (0, 0, 0, REG_MASK as u8)),
    ] {
        assert_eq!(
            (insn.rd.get(), insn.ra.get(), insn.rb.get(), insn.rc.get()),
            expected
        );
        assert_eq!(insn.op, OpCode::Fmadd, "register field overflowed into the opcode");
        assert_eq!(insn.imm, 0);
    }
}

#[test]
fn every_register_index_from_0_to_63_round_trips() {
    // Exactly 64 registers, indexed by a 6-bit field (isa.md §2.1). The boundary
    // values 0 and 63 are the ones a width bug would break first.
    for r in 0..=REG_MASK {
        let insn = decode(0x30, r, r, r, r, 0);
        assert_eq!(insn.rd.get() as u64, r, "rd = {r}");
        assert_eq!(insn.ra.get() as u64, r, "ra = {r}");
        assert_eq!(insn.rb.get() as u64, r, "rb = {r}");
        assert_eq!(insn.rc.get() as u64, r, "rc = {r}");
    }
}

// ---------------------------------------------------------------------------
// Opcode space
// ---------------------------------------------------------------------------

#[test]
fn isa_defines_exactly_49_distinct_opcodes() {
    assert_eq!(FIELDS.len(), 49, "isa.md §4.12 enumerates 49 instructions");
    let mut seen: Vec<u8> = FIELDS.iter().map(|&(byte, _, _)| byte).collect();
    seen.sort_unstable();
    let total = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), total, "the opcode table contains a duplicate byte");
}

#[test]
fn every_defined_opcode_decodes_to_its_expected_operands() {
    for &(byte, slots, uses_imm) in FIELDS.iter() {
        let insn = decode(byte, S_RD, S_RA, S_RB, S_RC, IMM);

        let slot = |present: u8, sentinel: u64| {
            if slots & present != 0 {
                RegId::new(sentinel as u8).expect("sentinel < 64")
            } else {
                RegId::ZERO
            }
        };

        assert_eq!(insn.op, OpCode::from_byte(byte).expect("defined opcode"), "opcode {byte:#04x}");
        assert_eq!(insn.rd, slot(B_RD, S_RD), "opcode {byte:#04x} rd");
        assert_eq!(insn.ra, slot(B_RA, S_RA), "opcode {byte:#04x} ra");
        assert_eq!(insn.rb, slot(B_RB, S_RB), "opcode {byte:#04x} rb");
        assert_eq!(insn.rc, slot(B_RC, S_RC), "opcode {byte:#04x} rc");
        assert_eq!(
            insn.imm,
            if uses_imm { IMM } else { 0 },
            "opcode {byte:#04x} imm"
        );
    }
}

#[test]
fn opcode_space_is_exactly_defined_plus_reserved() {
    // `decode` reports a reserved opcode as `Err`, so the whole 8-bit space can
    // be scanned without catching unwinds or muting the panic hook.
    let decoded: Vec<u8> = (0u8..=255)
        .filter(|&op| Insn::decode(word(op, 0, 0, 0, 0, 0)).is_ok())
        .collect();

    let mut expected: Vec<u8> = FIELDS.iter().map(|&(byte, _, _)| byte).collect();
    expected.sort_unstable();
    assert_eq!(
        decoded, expected,
        "the decoder must accept exactly the 49 defined opcodes and reject the other 207"
    );
}

#[test]
fn reserved_opcode_inside_a_group_is_rejected() {
    // 0x04 is the first hole after FMOVI / IMOVI / FMOV / IMOV.
    assert!(matches!(
        Insn::decode(word(0x04, 0, 0, 0, 0, 0)),
        Err(DecodeError::InvalidOpCode(0x04))
    ));
}

#[test]
fn reserved_high_opcode_is_rejected() {
    assert!(matches!(
        Insn::decode(word(0xFF, 0, 0, 0, 0, 0)),
        Err(DecodeError::InvalidOpCode(0xFF))
    ));
}

#[test]
fn reserved_opcode_with_populated_fields_is_rejected() {
    // The opcode byte alone decides: a reserved opcode is refused even when every
    // other field looks like a plausible operand. 0xA3 is the first hole after
    // HALT / NOP / FLUSH.
    assert!(matches!(
        Insn::decode(word(0xA3, S_RD, S_RA, S_RB, S_RC, u32::MAX)),
        Err(DecodeError::InvalidOpCode(0xA3))
    ));
}

// ---------------------------------------------------------------------------
// `imm` interpretation (isa.md §1.2)
// ---------------------------------------------------------------------------

#[test]
fn fmovi_embeds_a_bit_pattern_not_a_number() {
    let cases: [(u32, f32); 6] = [
        (0x3F80_0000, 1.0),
        (0xBF80_0000, -1.0),
        (0x0000_0000, 0.0),
        (0x8000_0000, -0.0),
        (0x7F80_0000, f32::INFINITY),
        (0xFF80_0000, f32::NEG_INFINITY),
    ];
    for (bits, expected) in cases {
        let insn = decode(0x00, S_RD, 0, 0, 0, bits);
        assert_eq!(insn.op, OpCode::Fmovi);
        assert_eq!(insn.rd.get() as u64, S_RD);
        assert_eq!(insn.imm_f32().to_bits(), bits, "FMOVI must embed {bits:#010x} verbatim");
        assert_eq!(insn.imm_f32(), expected);
    }
}

#[test]
fn fmovi_regression_numeric_cast_would_be_wrong() {
    // Guards the bug in the first draft (`imm as f32`): a numeric cast turns the
    // *integer* 0x3F80_0000 into 1.06535322e9 rather than 1.0, and turns 1 into
    // 1.0 rather than the smallest positive subnormal.
    let insn = decode(0x00, 0, 0, 0, 0, 0x3F80_0000);
    assert_eq!(insn.imm_f32(), 1.0, "0x3F80_0000 is the bit pattern of 1.0f32");

    let insn = decode(0x00, 0, 0, 0, 0, 1);
    assert_eq!(insn.imm_f32().to_bits(), 1, "imm = 1 must be a subnormal, not 1.0");
    assert_ne!(insn.imm_f32(), 1.0);
    assert!(insn.imm_f32() > 0.0 && insn.imm_f32().is_subnormal());
}

#[test]
fn fmovi_preserves_nan_payloads() {
    const QUIET_NAN_WITH_PAYLOAD: u32 = 0x7FC0_0001;
    let insn = decode(0x00, 0, 0, 0, 0, QUIET_NAN_WITH_PAYLOAD);
    assert!(insn.imm_f32().is_nan());
    assert_eq!(
        insn.imm_f32().to_bits(),
        QUIET_NAN_WITH_PAYLOAD,
        "NaN payload was not preserved"
    );
}

#[test]
fn wait_imm_is_a_bit_exact_f32() {
    for bits in [0x3F80_0000u32, 0x0000_0000, 0x8000_0000, 0x7FC0_0001] {
        let insn = decode(0x71, 0, 0, 0, 0, bits);
        assert_eq!(insn.op, OpCode::Wait);
        assert_eq!(insn.imm_f32().to_bits(), bits, "WAIT must embed {bits:#010x} verbatim");
    }
}

#[test]
fn imovi_imm_is_a_signed_i32() {
    let cases: [(u32, i32); 5] = [
        (0x0000_0000, 0),
        (0x0000_0001, 1),
        (0x7FFF_FFFF, i32::MAX),
        (0x8000_0000, i32::MIN),
        (0xFFFF_FFFF, -1),
    ];
    for (bits, expected) in cases {
        let insn = decode(0x01, S_RD, 0, 0, 0, bits);
        assert_eq!(insn.op, OpCode::Imovi);
        assert_eq!(insn.rd.get() as u64, S_RD);
        assert_eq!(insn.imm_i32(), expected, "IMOVI imm {bits:#010x}");
    }
}

#[test]
fn jump_offsets_are_sign_extended() {
    let cases: [(u32, i32); 3] = [
        (0xFFFF_FFFF, -1),
        (0x8000_0000, i32::MIN),
        (0x7FFF_FFFF, i32::MAX),
    ];
    for op in JUMP_OPCODES {
        for (bits, expected) in cases {
            let insn = decode(op, 0, S_RA, S_RB, 0, bits);
            assert_eq!(insn.op, OpCode::from_byte(op).expect("defined opcode"));
            assert_eq!(
                insn.imm_i32(),
                expected,
                "opcode {op:#04x} imm {bits:#010x} must sign-extend"
            );
        }
    }
}

#[test]
fn index_immediates_are_zero_extended_not_sign_extended() {
    // sensor / var / muscle ids are unsigned indices; 0xFFFF_FFFF is a huge
    // index (that validation must reject later), never -1.
    let max = u32::MAX as usize;
    assert_eq!(decode(0x60, S_RD, 0, 0, 0, u32::MAX).imm_index(), max);
    assert_eq!(decode(0x61, S_RD, 0, 0, 0, u32::MAX).imm_index(), max);
    assert_eq!(decode(0x90, S_RD, 0, 0, 0, u32::MAX).imm_index(), max);
    assert_eq!(decode(0x91, 0, S_RA, 0, 0, u32::MAX).imm_index(), max);
    assert_eq!(decode(0x92, S_RD, 0, 0, 0, u32::MAX).imm_index(), max);
    assert_eq!(decode(0x93, 0, S_RA, 0, 0, u32::MAX).imm_index(), max);
    assert_eq!(decode(0x80, 0, S_RA, 0, 0, u32::MAX).imm_index(), max);
}
