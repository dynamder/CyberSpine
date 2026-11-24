//! Generated source text for the parser and the image generator.
//!
//! The hand-written cases cover the grammar; these cover the inputs nobody
//! thought to write — an unbalanced brace, a lone backslash, a multi-byte
//! character where a mnemonic should be. A parser may reject any of them, but
//! it may not panic.

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::rig::Rig;
use proptest::prelude::*;

const MANIFEST: &str = r#"{
  "rig": { "name": "probe", "version": 1 },
  "muscles": ["m0"],
  "sensors_f": [],
  "sensors_i": []
}"#;

fn rig() -> Rig {
    cyberspine_manifest::from_json(MANIFEST).expect("the fixture manifest parses")
}

proptest! {
    #[test]
    fn any_source_text_parses_or_is_refused(text in any::<String>()) {
        let _ = parse(&text);
    }

    #[test]
    fn a_parsed_source_generates_or_is_refused(text in any::<String>()) {
        if let Ok(assembly) = parse(&text) {
            let _ = generate_image(&assembly, &rig());
        }
    }
}
