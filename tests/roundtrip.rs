//! Round-trip tests for the instruction-word codec: `Insn::encode` against
//! `Insn::decode`.
//!
//! `tests/bytecode.rs` pins the decoder to the spec with a hand-written table.
//! This file pins the encoder the same way, and then exercises the property the
//! loader is built on.
//!
//! **Why the round trip is the whole of `docs/binfmt.md` §9.3 I2.** `Insn::decode`
//! zeroes every operand field the opcode does not use, and `Insn::encode` writes
//! only the fields the opcode declares. So a word with bits in unused positions
//! decodes to the *same* canonical `Insn` as its clean form, and the only
//! surviving difference is the word itself. `encode(decode(w)) == w` therefore
//! decides canonicality exactly, and `!=` is the rejection rule.
//!
//! Normative references:
//!   * `docs/isa.md` §1.1 — 8/6/6/6/6/32 layout
//!   * `docs/isa.md` §1.2 — per-opcode `imm` interpretation
//!   * `docs/isa.md` §4    — which operands each opcode uses
//!   * `docs/binfmt.md` §8 — canonical form, §9.3 I2 — the round-trip equation

use cyber_spine::register::RegId;
use cyber_spine::vm::op::{ImmType, Insn, OpCode, RegName};

// ---------------------------------------------------------------------------
// Independent re-derivation of the layout (isa.md §1.1).
// ---------------------------------------------------------------------------

const OP_SHIFT: u32 = 56;
const RD_SHIFT: u32 = 50;
const RA_SHIFT: u32 = 44;
const RB_SHIFT: u32 = 38;
const RC_SHIFT: u32 = 32;
const REG_MASK: u64 = 0x3F;

const SHIFTS: [u32; 4] = [RD_SHIFT, RA_SHIFT, RB_SHIFT, RC_SHIFT];

const B_RD: u8 = 1;
const B_RA: u8 = 2;
const B_RB: u8 = 4;
const B_RC: u8 = 8;

/// `(opcode, register slots, uses imm)`, re-derived from `docs/isa.md` §4 without
/// consulting the generated table. If the encoder ever wrote a field this table
/// says is unused, `canonical_word` would build a word it does not reproduce and
/// the property tests below would fail.
const FIELDS: [(u8, u8, bool); 48] = [
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
];

/// The immediate values the property tests sweep. Chosen to be the ones a width
/// or signedness slip would break first: `0x3F80_0000` is `1.0f32`, a legal
/// positive jump offset, and a plain integer; `0x8000_0000` and `0xFFFF_FFFF`
/// are `i32::MIN` and `-1`; `0x7FC0_0001` is a NaN whose payload must survive.
const IMMS: [u32; 6] = [
    0x0000_0000,
    0x0000_0001,
    0x3F80_0000,
    0x7FC0_0001,
    0x8000_0000,
    0xFFFF_FFFF,
];

fn fields_of(op: u8) -> (u8, bool) {
    FIELDS
        .iter()
        .find(|&&(byte, _, _)| byte == op)
        .map(|&(_, slots, uses_imm)| (slots, uses_imm))
        .unwrap_or_else(|| panic!("opcode {op:#04x} is missing from FIELDS"))
}

/// Build the one canonical word for `op` with the given operands: used fields
/// hold what the caller asked for, unused fields are zero.
fn canonical_word(op: u8, regs: [u64; 4], imm: u32) -> u64 {
    let (slots, uses_imm) = fields_of(op);
    assert!(
        regs.iter().all(|&r| r <= REG_MASK),
        "canonical_word is for canonical operands only"
    );

    let mut word = (op as u64) << OP_SHIFT;
    for (slot, shift) in SHIFTS.iter().enumerate() {
        if slots & (1 << slot) != 0 {
            word |= regs[slot] << shift;
        }
    }
    if uses_imm {
        word |= imm as u64;
    }
    word
}

/// Every bit this opcode is permitted to occupy, derived from `FIELDS`.
fn used_mask(op: u8) -> u64 {
    let (slots, uses_imm) = fields_of(op);
    let mut mask = 0xFFu64 << OP_SHIFT;
    for (slot, shift) in SHIFTS.iter().enumerate() {
        if slots & (1 << slot) != 0 {
            mask |= REG_MASK << shift;
        }
    }
    if uses_imm {
        mask |= u32::MAX as u64;
    }
    mask
}

/// Build an `Insn` from independent fields. Unused fields are the caller's
/// business; `encode` ignores them.
fn insn_from(op: OpCode, regs: [u64; 4], imm: u32) -> Insn {
    Insn {
        op,
        rd: RegId::new(regs[0] as u8).expect("reg < 64"),
        ra: RegId::new(regs[1] as u8).expect("reg < 64"),
        rb: RegId::new(regs[2] as u8).expect("reg < 64"),
        rc: RegId::new(regs[3] as u8).expect("reg < 64"),
        imm,
    }
}

fn decode(word: u64) -> Insn {
    Insn::decode(word)
        .unwrap_or_else(|e| panic!("word {word:#018x} must decode, but was rejected: {e}"))
}

fn encode(insn: Insn) -> u64 {
    insn.encode()
}

// ---------------------------------------------------------------------------
// Encoder conformance: the 48-row table against the independent spec
// ---------------------------------------------------------------------------

#[test]
fn fields_table_covers_every_opcode() {
    assert_eq!(FIELDS.len(), 48, "isa.md §4.12 enumerates 48 instructions");
    let mut seen: Vec<u8> = FIELDS.iter().map(|&(byte, _, _)| byte).collect();
    seen.sort_unstable();
    let total = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), total, "FIELDS contains a duplicate opcode");
}

#[test]
fn generated_table_matches_the_independent_spec() {
    // The single source of truth is `isa_table!`; this pins it to a table written
    // out from §4 by hand, so a transposed opcode or a dropped operand is caught
    // even though encoder and decoder now share the generated table.
    for &(byte, slots, uses_imm) in FIELDS.iter() {
        let code = OpCode::from_byte(byte)
            .unwrap_or_else(|| panic!("opcode {byte:#04x} is missing from isa_table!"));

        assert_eq!(code.byte(), byte, "opcode {byte:#04x} maps back to the wrong byte");

        for (name, bit) in [
            (RegName::Rd, B_RD),
            (RegName::Ra, B_RA),
            (RegName::Rb, B_RB),
            (RegName::Rc, B_RC),
        ] {
            assert_eq!(
                code.uses(name),
                slots & bit != 0,
                "opcode {byte:#04x} field usage for {name:?}"
            );
        }

        assert_eq!(
            code.imm_type() != ImmType::None,
            uses_imm,
            "opcode {byte:#04x} imm usage"
        );
    }
}

#[test]
fn encode_writes_the_expected_word_for_every_opcode() {
    // Sentinels are distinct in every slot, so an encoder that put RA where RB
    // belongs cannot accidentally produce the expected word.
    const D: u64 = 1;
    const A: u64 = 2;
    const B: u64 = 3;
    const C: u64 = 4;

    for &(op, _, uses_imm) in FIELDS.iter() {
        let code = OpCode::from_byte(op).expect("defined opcode");
        let imm = if uses_imm { IMMS[2] } else { 0 };

        let expected = canonical_word(op, [D, A, B, C], imm);
        assert_eq!(
            encode(insn_from(code, [D, A, B, C], imm)),
            expected,
            "{code:?} (opcode {op:#04x}) must encode with its operands in the slots isa.md §1.1 assigns"
        );
    }
}

// ---------------------------------------------------------------------------
// The I2 property: encode ∘ decode is the canonicalisation of a word
// ---------------------------------------------------------------------------

#[test]
fn every_canonical_word_is_reproduced_verbatim() {
    // `encode(decode(w)) == w` for canonical `w` — the accepting half of I2.
    let operand_sets: [[u64; 4]; 6] = [
        [0, 0, 0, 0],
        [1, 2, 3, 4],
        [REG_MASK, REG_MASK, REG_MASK, REG_MASK],
        [REG_MASK, 0, REG_MASK, 0],
        [0, REG_MASK, 0, REG_MASK],
        [32, 33, 34, 35],
    ];

    for &(op, _, uses_imm) in FIELDS.iter() {
        for regs in operand_sets {
            for imm in IMMS {
                if !uses_imm && imm != 0 {
                    continue; // a non-zero imm would make the word non-canonical
                }
                let word = canonical_word(op, regs, imm);
                assert_eq!(
                    encode(decode(word)),
                    word,
                    "opcode {op:#04x} regs {regs:?} imm {imm:#010x} did not survive encode(decode(..))"
                );
            }
        }
    }
}

#[test]
fn every_register_value_in_every_slot_is_reproduced() {
    // The 6-bit field width is the register file length (isa.md §2.1), so all 64
    // values must survive in each slot the opcode actually uses.
    for &(op, slots, _) in FIELDS.iter() {
        for slot in 0..4 {
            if slots & (1 << slot) == 0 {
                continue;
            }
            for value in 0..=REG_MASK {
                let mut regs = [0u64; 4];
                regs[slot] = value;
                let word = canonical_word(op, regs, 0);
                assert_eq!(
                    encode(decode(word)),
                    word,
                    "opcode {op:#04x} slot {slot} = {value} did not survive the round trip"
                );
            }
        }
    }
}

#[test]
fn a_word_with_bits_in_unused_fields_is_not_reproduced() {
    // The rejecting half of I2 — exactly the check the loader runs, and what makes
    // the on-disk encoding canonical (`binfmt.md` §8).
    for &(op, _, _) in FIELDS.iter() {
        let canonical = canonical_word(op, [1, 2, 3, 4], 0x3F80_0000);
        let mask = used_mask(op);
        let mut dirty_bits = 0;

        for bit in 0..64 {
            if mask & (1u64 << bit) != 0 {
                continue;
            }
            let dirty = canonical | (1u64 << bit);

            // The opcode is still defined, so decoding succeeds; only the
            // re-encoded word reveals that the input was not canonical.
            let decoded = decode(dirty);
            assert_ne!(
                encode(decoded),
                dirty,
                "opcode {op:#04x}: bit {bit} is unused, so re-encoding must differ from the input"
            );
            assert_eq!(
                encode(decoded),
                canonical,
                "opcode {op:#04x}: bit {bit} is unused, so dropping it must yield the canonical word"
            );
            dirty_bits += 1;
        }

        assert!(
            dirty_bits > 0,
            "opcode {op:#04x} uses all 64 bits, so I2 could never reject a word for it"
        );
    }
}

#[test]
fn re_encoding_is_idempotent_over_the_whole_opcode_space() {
    // Deterministic xorshift64: no dependency, and a fixed seed keeps failures
    // reproducible.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut accepted = 0;
    for _ in 0..4096 {
        let word = next();

        let Ok(decoded) = Insn::decode(word) else {
            continue; // reserved opcode: I1's territory, not this test's
        };
        accepted += 1;

        let canonical = encode(decoded);
        assert_eq!(
            encode(decode(canonical)),
            canonical,
            "canonicalising {word:#018x} was not a fixed point"
        );
        assert!(
            canonical & !used_mask(decoded.op.byte()) == 0,
            "canonical form of {word:#018x} still carries bits the opcode does not use"
        );
    }

    assert!(accepted > 0, "the sweep never hit a defined opcode");
}

// ---------------------------------------------------------------------------
// `imm` interpretations and out-of-range operands
// ---------------------------------------------------------------------------

#[test]
fn imm_interpretations_round_trip_every_bit_pattern() {
    // f32 by bit pattern: every 32-bit pattern is legal, including NaN payloads
    // and both signed zeros (isa.md §1.2, binfmt.md §6.4).
    for bits in [0x0000_0000u32, 0x8000_0000, 0x7FC0_0001, 0xFFC0_0001, 0x0000_0001] {
        let insn = insn_from(OpCode::Fmovi, [0; 4], bits);
        assert_eq!(insn.imm_f32().to_bits(), bits, "FMOVI must embed {bits:#010x} verbatim");
        assert_eq!(insn.encode() & u32::MAX as u64, bits as u64);

        let insn = insn_from(OpCode::Wait, [0; 4], bits);
        assert_eq!(insn.imm_f32().to_bits(), bits, "WAIT must embed {bits:#010x} verbatim");
        assert_eq!(insn.encode() & u32::MAX as u64, bits as u64);
    }

    // i32 by two's complement, and jump offsets by the same i32 field.
    for bits in [0x0000_0000u32, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF] {
        let insn = insn_from(OpCode::Imovi, [0; 4], bits);
        assert_eq!(insn.imm_i32(), bits as i32, "IMOVI must embed {bits:#010x} as i32");
        assert_eq!(insn.encode() & u32::MAX as u64, bits as u64);

        let insn = insn_from(OpCode::Jmp, [0; 4], bits);
        assert_eq!(insn.imm_i32(), bits as i32, "JMP must embed {bits:#010x} as i32");
        assert_eq!(insn.encode() & u32::MAX as u64, bits as u64);
    }

    // Unsigned indices span the whole u32 range and zero-extend, never sign-extend.
    for value in [0u32, 1, u32::MAX] {
        for code in [
            OpCode::Fsense,
            OpCode::Isense,
            OpCode::Fvget,
            OpCode::Ivget,
            OpCode::Fvset,
            OpCode::Ivset,
            OpCode::Fexcite,
        ] {
            let insn = insn_from(code, [0; 4], value);
            assert_eq!(insn.imm_index(), value as usize, "{code:?} must zero-extend index {value}");
            assert_eq!(insn.encode() & u32::MAX as u64, value as u64);
        }
    }
}

#[test]
fn a_register_index_out_of_range_is_unrepresentable() {
    // The old typed enum took `usize` operands and had to refuse 64 at encode
    // time. `RegId` makes it a type-level property instead: there is no `Insn`
    // holding an out-of-range register, so `encode` has no refusal path to get
    // wrong — and no mask that could silently alias 64 onto register 0.
    assert!(RegId::new(64).is_none());
    assert!(RegId::new(u8::MAX).is_none());
    assert_eq!(RegId::new(63).expect("63 is a legal register").get(), 63);
}
