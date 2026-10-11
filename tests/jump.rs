//! Tests for the shared jump-offset arithmetic (`docs/isa.md` §1.3).
//!
//! §1.3 requires `pc + 1 + off` to be derived in exactly one place, so this file
//! is the contract every other layer inherits: the assembler, the disassembler
//! and the loader. Get the convention wrong once and *every* jump in *every*
//! program lands one instruction off, with nothing to show for it at the source
//! level.

use cyber_spine::vm::jump::{jump_offset, jump_target};
use cyber_spine::vm::op::{Insn, OpCode};

/// Program counters worth exercising: the origin, small ones, and the region
/// around 2³¹ where `i32` offsets run out of room in both directions.
const PCS: [usize; 6] = [0, 1, 2, 999, 1 << 20, (1 << 31) - 1];

// ---------------------------------------------------------------------------
// The convention, pinned by worked examples
// ---------------------------------------------------------------------------

#[test]
fn off_zero_falls_through_to_the_next_instruction() {
    // The one case where "relative to the next instruction" and "relative to
    // this instruction" are indistinguishable, so it proves nothing on its own.
    // It is here to stop the other cases from being read as byte offsets.
    for pc in PCS {
        assert_eq!(jump_target(pc, 0), Some(pc + 1), "pc {pc}");
    }
}

#[test]
fn off_minus_one_targets_the_jump_itself() {
    // `JMP -1` is a legal encoding of an infinite loop. Nothing here treats a
    // self-jump as a special case, and nothing should: fuel (isa.md §7) is the
    // runtime bound, not a control-flow restriction.
    for pc in PCS {
        assert_eq!(jump_target(pc, -1), Some(pc), "pc {pc}");
    }
}

#[test]
fn a_forward_branch_skipping_one_instruction_is_offset_one() {
    //   pc 0: IJZ ra, +1      -> pc 2, skipping the instruction at pc 1
    //   pc 1: <not executed>
    //   pc 2: <branch target>
    assert_eq!(jump_offset(0, 2), Some(1));
    assert_eq!(jump_target(0, 1), Some(2));
}

#[test]
fn a_backward_jump_to_the_loop_head_is_offset_minus_the_body_length() {
    // The canonical counted loop, and the measurement humans get wrong:
    //
    //   pc 0: IADD  ...          <- `L:` labels this instruction
    //   pc 1: IGT   ra, rb, +1   -> forward branch, skips pc 2 when done
    //   pc 2: JMP   L            -> off = 0 - (2 + 1) = -3
    //   pc 3: HALT
    //
    // `off` is measured from the instruction *after* the jump, so coming back
    // from pc 2 to pc 0 costs three, not two.
    assert_eq!(jump_offset(2, 0), Some(-3));
    assert_eq!(jump_target(2, -3), Some(0));

    // The bug this pins: reading the offset as `-(body length)` lands on pc 1,
    // one instruction late — past the loop head and into the middle of the body.
    assert_eq!(
        jump_target(2, -2),
        Some(1),
        "the naive `-body_length` reading must land one instruction late, or this test is not testing the convention"
    );
}

// ---------------------------------------------------------------------------
// The two directions are exact inverses
// ---------------------------------------------------------------------------

#[test]
fn jump_offset_inverts_jump_target() {
    let offsets = (-1000..=1000).chain([i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX]);

    for pc in PCS {
        for off in offsets.clone() {
            // Only defined when the target lands at or above index 0.
            if let Some(target) = jump_target(pc, off) {
                assert_eq!(jump_offset(pc, target), Some(off), "pc {pc}, off {off}");
            }
        }
    }
}

#[test]
fn jump_target_inverts_jump_offset() {
    for pc in PCS {
        for target in [
            0,
            1,
            pc.saturating_sub(1),
            pc,
            pc + 1,
            pc + 5,
            pc.saturating_add(1 << 20),
        ] {
            if let Some(off) = jump_offset(pc, target) {
                assert_eq!(jump_target(pc, off), Some(target), "pc {pc}, target {target}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Edges
// ---------------------------------------------------------------------------

#[test]
fn a_target_below_index_zero_is_unrepresentable() {
    // The boundary is `off <= -(pc + 2)`: one step past `pc + 1 + off == 0`.
    // `off = -(pc + 1)` is still legal and lands exactly on index 0, so the
    // self-jump case (pc 0, off -1) sits on the valid side of the line.
    assert_eq!(jump_target(0, -1), Some(0), "off -1 from pc 0 is the self-jump, not an underflow");
    assert_eq!(jump_target(0, -2), None, "pc 0 with off -2 would land on index -1");
    assert_eq!(jump_target(0, i32::MIN), None);

    assert_eq!(jump_target(5, -6), Some(0), "off -(pc + 1) lands exactly on index 0");
    assert_eq!(jump_target(5, -7), None);
    assert_eq!(jump_target(5, i32::MIN), None);

    assert_eq!(jump_offset(0, 0), Some(-1));
    assert_eq!(jump_offset(5, 0), Some(-6));
}

#[test]
fn a_program_counter_that_cannot_be_advanced_yields_nothing() {
    assert_eq!(jump_target(usize::MAX, 0), None);
    assert_eq!(jump_target(usize::MAX, 1), None);
    assert_eq!(jump_offset(usize::MAX, 0), None);
}

#[test]
fn the_extremes_of_i32_are_addressable_from_a_large_enough_pc() {
    // The widest backward jump: from just below 2³¹ back to the origin.
    let far = (1usize << 31) - 1;
    assert_eq!(jump_target(far, i32::MIN), Some(0));
    assert_eq!(jump_offset(far, 0), Some(i32::MIN));

    // The widest forward jump: from the origin to 2³¹.
    assert_eq!(jump_target(0, i32::MAX), Some(1usize << 31));
    assert_eq!(jump_offset(0, 1usize << 31), Some(i32::MAX));
}

#[test]
fn a_distance_that_does_not_fit_i32_is_refused() {
    // off = target - (pc + 1), so at pc 0 the farthest representable target is 2³¹.
    assert_eq!(jump_offset(0, 1usize << 31), Some(i32::MAX));
    assert_eq!(jump_offset(0, (1usize << 31) + 1), None);

    // And symmetrically on the backward side.
    assert_eq!(jump_offset((1usize << 31) - 1, 0), Some(i32::MIN));
    assert_eq!(jump_offset(1usize << 31, 0), None);
}

// ---------------------------------------------------------------------------
// Both directions chained through the real codec
// ---------------------------------------------------------------------------

#[test]
fn an_offset_survives_the_assembler_and_loader_paths() {
    // The two ends the field format will actually wire together:
    //   assembler: off = jump_offset(pc, label_pc); encode JMP off
    //   loader:    decode word; target = jump_target(pc, off)
    // Chained through the real encoder and decoder, the round trip must land on
    // the label. This is the one test here that would catch the offset being
    // stored or recovered with the wrong width or signedness.
    for (pc, label_pc) in [(2usize, 0usize), (0, 5), (7, 7), (3, 4), (100, 0), (0, 0)] {
        let off = jump_offset(pc, label_pc).expect("this distance fits an i32");
        let word = Insn { imm: off as u32, ..Insn::new(OpCode::Jmp) }.encode();

        let decoded = Insn::decode(word).expect("a JMP word always decodes");
        assert_eq!(decoded.op, OpCode::Jmp, "JMP must decode back to JMP");

        assert_eq!(
            jump_target(pc, decoded.imm_i32()),
            Some(label_pc),
            "JMP at pc {pc} to the label at {label_pc} (off {off}) landed elsewhere"
        );
    }
}
