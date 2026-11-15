//! The debug segment seen from the outside: the flag, the layout, and the hash.
//!
//! The segment's own rules live in `crates/cyberspine-binfmt/src/debug.rs`; these
//! tests only claim what `docs/binfmt.md` §6.7 says about the *file* — that a
//! debug build is the same image plus a tail, that the tail is covered by the
//! hash, and that a flag without a segment is refused.

use cyberspine_core::binfmt::DebugError;
use cyberspine_core::binfmt::debug::{DebugInfo, DebugLabel, DebugLine};
use cyberspine_core::binfmt::image::{HASH_LEN, HEADER_LEN, INSN_LEN, Image};
use cyberspine_core::binfmt::load::{LoadError, VerifiedImage};
use cyberspine_core::isa::{Insn, OpCode};

const FLAGS: usize = 10;

fn debug_build() -> Image {
    let mut image = Image::new(vec![Insn::new(OpCode::Halt)]);
    image.set_debug(DebugInfo::new(
        "maze.csa",
        vec![DebugLabel {
            name: "start".into(),
            pc: 0,
        }],
        vec![DebugLine { line: 7, column: 5 }],
    ));
    image
}

fn release_build() -> Image {
    Image::new(vec![Insn::new(OpCode::Halt)])
}

/// The segment starts right after the code of a metadata-free image.
fn segment_start(code_len: usize) -> usize {
    HEADER_LEN + INSN_LEN * code_len
}

fn repair_hash(bytes: &mut [u8]) {
    let hash = Image::content_hash(bytes).expect("a file has a hash slot");
    let at = bytes.len() - HASH_LEN;
    bytes[at..].copy_from_slice(hash.as_bytes());
}

#[test]
fn a_debug_build_sets_the_flag_and_round_trips() {
    let image = debug_build();
    let bytes = image.to_bytes();

    assert_eq!(
        u16::from_le_bytes([bytes[FLAGS], bytes[FLAGS + 1]]) & 1,
        1,
        "HAS_DEBUG is `flags` bit 0"
    );
    assert_eq!(
        bytes.len(),
        HEADER_LEN + INSN_LEN + image.debug().expect("debug").encode(1).len() + HASH_LEN,
        "the file is header, code, segment, hash — nothing else"
    );

    let loaded = VerifiedImage::from_bytes(&bytes).expect("the loader accepts a debug build");
    let debug = loaded.debug().expect("the segment survives the round trip");
    assert_eq!(debug.source(), "maze.csa");
    assert_eq!(debug.line(0), Some(DebugLine { line: 7, column: 5 }));
    assert_eq!(debug.label_at(0), Some("start"));
}

#[test]
fn the_release_build_carries_no_segment() {
    let bytes = release_build().to_bytes();
    assert_eq!(
        u16::from_le_bytes([bytes[FLAGS], bytes[FLAGS + 1]]),
        0,
        "a release build writes no flag"
    );
    let loaded = VerifiedImage::from_bytes(&bytes).expect("the loader accepts it");
    assert!(loaded.debug().is_none());
}

#[test]
fn the_debug_build_is_a_different_artifact() {
    let debug = VerifiedImage::from_bytes(&debug_build().to_bytes()).expect("loads");
    let release = VerifiedImage::from_bytes(&release_build().to_bytes()).expect("loads");

    assert_ne!(
        debug.content_hash(),
        release.content_hash(),
        "the hash covers the segment, so the two builds are distinct (§7)"
    );
}

#[test]
fn the_hash_covers_the_segment() {
    let mut bytes = debug_build().to_bytes();
    let at = segment_start(1);
    bytes[at] ^= 0xff;

    assert!(matches!(
        VerifiedImage::from_bytes(&bytes),
        Err(LoadError::ContentHashMismatch { .. })
    ));
}

#[test]
fn a_flag_without_a_segment_is_refused() {
    let mut bytes = release_build().to_bytes();
    bytes[FLAGS] = 1;
    repair_hash(&mut bytes);

    assert!(
        matches!(
            VerifiedImage::from_bytes(&bytes),
            Err(LoadError::Debug(DebugError::Truncated))
        ),
        "HAS_DEBUG with nothing behind it is a truncated segment, not an empty one"
    );
}

#[test]
fn an_undefined_flag_bit_is_refused() {
    let mut bytes = release_build().to_bytes();
    bytes[FLAGS] = 0b10;
    repair_hash(&mut bytes);

    assert!(matches!(
        VerifiedImage::from_bytes(&bytes),
        Err(LoadError::ReservedNotZero { field: "flags" })
    ));
}
