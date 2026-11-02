//! The disk entry point: the one thing the platform layer adds to the loader.
//!
//! Everything here needs a real filesystem, which is exactly why it lives in
//! `cyberspine-vm` and not in `cyberspine-core`.

use std::fs;
use std::path::PathBuf;

use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::LoadError;
use cyberspine_core::isa::{Insn, OpCode};
use cyberspine_core::rig::Name;
use cyberspine_vm::io::{LoadFileError, MAX_FILE_LEN, read_image};

fn n(text: &str) -> Name {
    Name::new(text).expect("test fixture name")
}

fn tmp_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn tmp_path(name: &str) -> PathBuf {
    let dir = tmp_dir();
    fs::create_dir_all(&dir).expect("create the test dir");
    dir.join(name)
}

#[test]
fn an_image_survives_a_trip_through_the_filesystem() {
    let path = tmp_path("round_trip.csp");

    let mut image = Image::new(vec![Insn::new(OpCode::Yield), Insn::new(OpCode::Halt)]);
    image.push_muscle(n("jaw"));
    fs::write(&path, image.to_bytes()).expect("write the image");

    let loaded = read_image(&path).expect("read the image back");
    assert_eq!(loaded.image(), &image);
}

#[test]
fn a_missing_file_is_an_io_error() {
    let path = tmp_path("does_not_exist.csp");
    let _ = fs::remove_file(&path);

    assert!(matches!(read_image(&path), Err(LoadFileError::Io { .. })));
}

#[test]
fn an_oversized_file_is_refused_by_the_metadata_gate() {
    // A sparse file: the length is real to the filesystem, the bytes are not.
    // §9.1 C5 cannot help here, because the allocation would happen in the read
    // itself — this gate is what stops it.
    let path = tmp_path("oversized.csp");
    let file = fs::File::create(&path).expect("create");
    file.set_len(MAX_FILE_LEN as u64 + 1)
        .expect("size the sparse file");
    drop(file);

    assert!(matches!(
        read_image(&path),
        Err(LoadFileError::FileTooLarge {
            max: MAX_FILE_LEN,
            ..
        })
    ));

    let _ = fs::remove_file(&path);
}

#[test]
fn a_truncated_file_on_disk_is_refused() {
    let path = tmp_path("truncated.csp");
    fs::write(&path, [0u8; 100]).expect("write");

    // The container rule (C1), not a disk rule: the file was read fine, it just
    // is not an image. The error stays a `LoadError` so the failing rule is still
    // nameable by a test or a user.
    assert!(matches!(
        read_image(&path),
        Err(LoadFileError::Image(LoadError::TooShort { len: 100 }))
    ));
}
