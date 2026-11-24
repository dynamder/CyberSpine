//! Generated inputs for the instruction codec.
//!
//! The table tests enumerate every opcode and every register value; these let
//! the whole 64-bit word space speak for itself. A word the decoder refuses is
//! fine; a word that decodes must re-encode to a fixed point, because that is
//! what makes the on-disk encoding canonical (`docs/binfmt.md` §8).

use cyberspine_core::isa::Insn;
use proptest::prelude::*;

proptest! {
    #[test]
    fn a_decoded_word_re_encodes_to_a_fixed_point(word in any::<u64>()) {
        if let Ok(insn) = Insn::decode(word) {
            let canonical = Insn::encode(insn);
            let decoded = Insn::decode(canonical).expect("a canonical word decodes");
            prop_assert_eq!(Insn::encode(decoded), canonical);
        }
    }
}
