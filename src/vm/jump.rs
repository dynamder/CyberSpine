//! The one place jump offsets are computed (`docs/isa.md` §1.3, §5.1, §5.2).
//!
//! `target = pc + 1 + off`, unit is **instructions**, `off` is an `i32`.
//!
//! The `pc + 1` is invisible in source, and §1.3 spells out the consequence: a
//! slip here makes *every* jump in a program land one instruction off. So the
//! arithmetic lives in exactly one shared place — this module — and the
//! assembler, the disassembler and the loader all call it. Nothing else may
//! re-derive `pc + 1 + off`.
//!
//! §5.1's discipline is the reason the two directions sit side by side: offsets
//! are a **derived** product. Labels are the editable representation; an
//! assembled image is frozen and is never byte-patched in place.

/// The instruction a jump at `pc` with offset `off` transfers control to.
///
/// `target = pc + 1 + off` — measured from the instruction *after* the jump, so
/// `off = 0` falls through to whatever follows the jump, and `off = -1` targets
/// the jump itself. A self-jump is a legal encoding of an infinite loop, not a
/// special case this function should reject.
///
/// Returns `None` when the arithmetic leaves the addressable space, i.e. when it
/// would land below index 0. The *upper* bound is deliberately not checked here:
/// it depends on `code_len`, which only the caller knows (`docs/binfmt.md` §9.3
/// I6 checks `target < code_len`).
///
/// This is the disassembler/loader direction; [`jump_offset`] is its inverse.
pub fn jump_target(pc: usize, off: i32) -> Option<usize> {
    let next = pc.checked_add(1)?;
    if off >= 0 {
        next.checked_add(off as usize)
    } else {
        next.checked_sub(off.unsigned_abs() as usize)
    }
}

/// The offset that makes a jump at `pc` transfer control to `target` — the
/// assembler's direction (`docs/isa.md` §5.2`: `off = labels[t] - (pc + 1)`).
///
/// Returns `None` when the distance does not fit an `i32`, which needs a unit of
/// at least 2³¹ instructions (`docs/isa.md` §5.3 rule 3).
///
/// This is the assembler direction; [`jump_target`] is its inverse. The two are
/// exact inverses wherever both are defined, and `tests/jump.rs` pins that as a
/// property rather than as a handful of examples.
pub fn jump_offset(pc: usize, target: usize) -> Option<i32> {
    let next = pc.checked_add(1)?;
    let distance = i64::try_from(target).ok()? - i64::try_from(next).ok()?;
    i32::try_from(distance).ok()
}
