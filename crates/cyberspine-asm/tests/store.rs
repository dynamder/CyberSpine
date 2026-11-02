//! The write path: atomic replace and its platform semantics.
//!
//! These tests run on Linux, macOS and Windows in CI. Anything that cannot
//! behave the same way everywhere is gated (`unix` / `windows`) rather than
//! silently skipped, so each platform's own rules are pinned by a test.

use std::fs;
use std::path::{Path, PathBuf};

use cyberspine_asm::store::{WriteError, write_atomic, write_image};
use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::isa::{Insn, OpCode};

/// A scratch directory owned by one test. `CARGO_TARGET_TMPDIR` is the crate's
/// target directory, so nothing is ever written outside the build tree.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("store")
        .join(name);
    // A failed previous run may have left files behind; each test owns its dir.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read the scratch directory")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn a_new_image_is_written_whole() {
    let dir = scratch("new");
    let path = dir.join("halt.csp");
    let image = Image::halt_only();

    write_image(&image, &path).expect("write the image");

    let bytes = fs::read(&path).expect("read the image back");
    assert_eq!(
        bytes,
        image.to_bytes(),
        "the file is the canonical encoding"
    );
    assert_eq!(
        VerifiedImage::from_bytes(&bytes)
            .expect("the loader accepts what we wrote")
            .image(),
        &image
    );
    assert_eq!(entries(&dir), vec!["halt.csp"], "no temporary file is left");
}

#[test]
fn an_existing_image_is_replaced() {
    let dir = scratch("replace");
    let path = dir.join("halt.csp");
    fs::write(&path, b"stale bytes").expect("seed the old file");

    let image = Image::halt_only();
    write_image(&image, &path).expect("replace the image");

    assert_eq!(fs::read(&path).expect("read back"), image.to_bytes());
    assert_eq!(entries(&dir), vec!["halt.csp"]);
}

#[test]
fn a_missing_parent_directory_is_reported_and_leaves_nothing() {
    let dir = scratch("missing-parent");
    let path = dir.join("no-such-dir").join("halt.csp");

    let error = write_atomic(&path, b"bytes").expect_err("the parent does not exist");

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(entries(&dir), Vec::<String>::new());
}

#[test]
fn a_directory_destination_is_refused_without_leftovers() {
    let dir = scratch("directory-destination");
    let path = dir.join("halt.csp");
    fs::create_dir(&path).expect("make the destination a directory");

    // The error kind differs per platform (EISDIR on Unix, ERROR_ACCESS_DENIED
    // on Windows, where the retry runs first), so only "it failed" is pinned.
    write_atomic(&path, b"bytes").expect_err("a directory cannot be replaced by a file");

    assert!(path.is_dir(), "the directory is untouched");
    assert_eq!(entries(&dir), vec!["halt.csp"], "no temporary file is left");
}

#[test]
fn an_image_the_loader_rejects_is_never_written() {
    let dir = scratch("not-canonical");
    let path = dir.join("halt.csp");
    // No trailing HALT: `docs/binfmt.md` §9.3 I8 rejects this image.
    let image = Image::new(vec![Insn::new(OpCode::Nop)]);

    let error = write_image(&image, &path).expect_err("the loader would reject this image");

    assert!(matches!(error, WriteError::NotCanonical { .. }), "{error}");
    assert!(!path.exists(), "a rejected image must never reach the disk");
    assert_eq!(entries(&dir), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn the_mode_of_an_existing_image_survives() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("mode");
    let path = dir.join("halt.csp");
    fs::write(&path, b"stale").expect("seed the old file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("chmod the old file");

    write_image(&Image::halt_only(), &path).expect("replace the image");

    let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o640, "the temp file's 0o600 default must not leak");
}

#[cfg(unix)]
#[test]
fn a_read_only_image_can_be_replaced() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("read-only");
    let path = dir.join("halt.csp");
    fs::write(&path, b"stale").expect("seed the old file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).expect("chmod the old file");

    // `fs::write` would fail here; replacing the directory entry only needs
    // write permission on the directory, which is what `mv -f` and Cargo do.
    let image = Image::halt_only();
    write_image(&image, &path).expect("replace the read-only image");

    assert_eq!(fs::read(&path).expect("read back"), image.to_bytes());
    let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o444, "read-only-ness is carried over");
}

#[cfg(unix)]
#[test]
fn a_symlink_is_followed() {
    use std::os::unix::fs::symlink;

    let dir = scratch("symlink");
    let target = dir.join("real.csp");
    fs::write(&target, b"stale").expect("seed the target");
    let link = dir.join("link.csp");
    symlink(&target, &link).expect("make the symlink");

    let image = Image::halt_only();
    write_image(&image, &link).expect("write through the symlink");

    assert!(
        fs::symlink_metadata(&link)
            .expect("stat the link")
            .file_type()
            .is_symlink(),
        "the link itself must survive"
    );
    assert_eq!(
        fs::read(&target).expect("read the target"),
        image.to_bytes()
    );
    assert_eq!(entries(&dir), vec!["link.csp", "real.csp"]);
}

#[cfg(windows)]
#[test]
fn an_image_locked_by_another_process_is_retried_then_reported() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = scratch("locked");
    let path = dir.join("halt.csp");
    fs::write(&path, b"stale").expect("seed the old file");

    {
        // `share_mode(0)` denies even the rename's `DELETE` access, which is the
        // shape of "another process has the image open".
        let _lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("lock the file");

        write_atomic(&path, b"new").expect_err("the destination is locked");
    }

    assert_eq!(
        fs::read(&path).expect("read back"),
        b"stale",
        "old file intact"
    );
    assert_eq!(entries(&dir), vec!["halt.csp"], "no temporary file is left");
}

#[cfg(windows)]
#[test]
fn a_read_only_image_can_be_replaced() {
    let dir = scratch("read-only-windows");
    let path = dir.join("halt.csp");
    fs::write(&path, b"stale").expect("seed the old file");
    let mut permissions = fs::metadata(&path).expect("stat").permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).expect("mark the file read-only");

    // A plain `fs::rename` cannot replace a read-only destination on Windows.
    // `write_atomic` clears the attribute for the rename, and the new file
    // inherits it from the old one.
    let image = Image::halt_only();
    write_image(&image, &path).expect("replace the read-only image");

    assert_eq!(fs::read(&path).expect("read back"), image.to_bytes());
    assert!(
        fs::metadata(&path).expect("stat").permissions().readonly(),
        "the read-only attribute is carried over"
    );
}
