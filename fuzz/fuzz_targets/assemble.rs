#![no_main]

//! Fuzz oracle for the assembler (`docs/asm.md`).
//!
//! The contract being tested is the accept/reject boundary:
//!
//! * a source the assembler rejects is legal behaviour — the input was illegal;
//! * a source the assembler accepts **must** produce a legal image, and the
//!   loader is the authority on "legal" (`docs/binfmt.md` §9);
//! * a panic anywhere is a robustness failure, not a rejection.
//!
//! libFuzzer treats panics as crashes; the `Image::from_bytes` branch turns a
//! false accept into a crash as well. "Legal code rejected" is not observable
//! from arbitrary bytes, so it is covered by `tests/generate.rs` and the seed
//! corpus instead.

use cyberspine_asm::assembler::{generate_image, parse};
use libfuzzer_sys::fuzz_target;
use cyberspine_core::binfmt::load::VerifiedImage;

mod rig_for;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let Ok(asm) = parse(source) else {
        return;
    };
    let Ok(image) = generate_image(&asm, &rig_for::rig_for(&asm)) else {
        return;
    };

    // Accepted means "this is a legal program". The loader must agree.
    let bytes = image.to_bytes();
    let reloaded = match VerifiedImage::from_bytes(&bytes) {
        Ok(reloaded) => reloaded,
        Err(err) => panic!("assembler accepted a program the loader rejects: {err}"),
    };

    // Accepted images also have to be canonical: a second write must be byte
    // identical (`docs/binfmt.md` §8). Byte equality, not `Image` equality,
    // because `VarF::init` is an `f32` and NaN never equals itself.
    assert_eq!(
        reloaded.to_bytes(),
        bytes,
        "an accepted image is not in canonical form"
    );
});
