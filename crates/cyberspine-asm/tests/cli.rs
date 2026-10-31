//! End-to-end tests for the `cyinea` binary: a `.csa` on disk goes in, a `.csp`
//! comes out.
//!
//! The binary is exercised as a process (`CARGO_BIN_EXE_cyinea`) because that is
//! the only way to pin what a user actually gets: the exit code, the message on
//! stderr, and whether a file survived a failed run.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::binfmt::image::Image;

const SOURCE: &str = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0

.muscle brow_l
.group brows = brow_l
.sensor_f smile

.text
entry:
    FMOVI   f1, 1.0
    FEXCITE brows, f1
    FLUSH
    FSENSE  f2, smile
    HALT
";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(name);
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

fn write_source(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("write the source file");
    path
}

fn run(args: Vec<&OsStr>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cyinea"))
        .args(args)
        .output()
        .expect("the cyinea binary should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The image the source is supposed to produce, straight from the library.
fn expected_image() -> Image {
    let asm = parse(SOURCE).expect("the test source parses");
    generate_image(&asm).expect("the test source generates an image")
}

#[test]
fn a_source_is_assembled_into_the_requested_file() {
    let dir = scratch("output");
    let source = write_source(&dir, "greet.csa", SOURCE);
    let target = dir.join("greet.csp");

    let output = run(vec![
        OsStr::new("-a"),
        source.as_os_str(),
        OsStr::new("-o"),
        target.as_os_str(),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(output.stdout.is_empty(), "success is silent");
    assert!(output.stderr.is_empty(), "success is silent");

    let expected = expected_image();
    let written = fs::read(&target).expect("read the file");
    assert_eq!(
        Image::from_bytes(&written).expect("the loader accepts the file"),
        expected
    );
    assert_eq!(
        written,
        expected.to_bytes(),
        "the file is the canonical encoding"
    );
    assert_eq!(
        entries(&dir),
        vec!["greet.csa", "greet.csp"],
        "no temporary file is left"
    );
}

#[test]
fn the_default_output_replaces_the_extension() {
    let dir = scratch("default-output");
    let source = write_source(&dir, "greet.csa", SOURCE);

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(dir.join("greet.csp").exists());
    assert_eq!(entries(&dir), vec!["greet.csa", "greet.csp"]);
}

#[test]
fn a_bare_output_name_lands_in_the_working_directory() {
    let dir = scratch("relative-output");
    let source = write_source(&dir, "greet.csa", SOURCE);

    // `-o out.csp` has no parent directory at all, which is the one path where
    // the temporary file's parent has to default to `.`.
    let output = Command::new(env!("CARGO_BIN_EXE_cyinea"))
        .current_dir(&dir)
        .args([
            OsStr::new("-a"),
            source.as_os_str(),
            OsStr::new("-o"),
            OsStr::new("out.csp"),
        ])
        .output()
        .expect("the cyinea binary should run");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(dir.join("out.csp").exists());
    assert_eq!(entries(&dir), vec!["greet.csa", "out.csp"]);
}

#[test]
fn a_parse_error_produces_no_file() {
    let dir = scratch("parse-error");
    // Only `.layer`: the other three fixed-header fields are missing.
    let source = write_source(&dir, "bad.csa", ".layer 10\n");

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("error:"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(!dir.join("bad.csp").exists(), "nothing is written");
    assert_eq!(entries(&dir), vec!["bad.csa"]);
}

#[test]
fn a_generation_error_produces_no_file() {
    let dir = scratch("generation-error");
    // `FEXCITE` still pending when `HALT` is reached (`docs/asm.md` §6.6).
    let source = write_source(
        &dir,
        "pending.csa",
        "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.muscle brow_l
.text
    FMOVI   f1, 1.0
    FEXCITE brow_l, f1
    HALT
",
    );

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("FLUSH is missing"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(!dir.join("pending.csp").exists(), "nothing is written");
    assert_eq!(entries(&dir), vec!["pending.csa"]);
}

#[test]
fn an_unwritable_destination_is_reported() {
    let dir = scratch("unwritable");
    let source = write_source(&dir, "greet.csa", SOURCE);
    let target = dir.join("no-such-dir").join("greet.csp");

    let output = run(vec![
        OsStr::new("-a"),
        source.as_os_str(),
        OsStr::new("-o"),
        target.as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot write"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(!target.exists());
    assert_eq!(
        entries(&dir),
        vec!["greet.csa"],
        "no temporary file is left"
    );
}

#[test]
fn an_existing_output_is_replaced() {
    let dir = scratch("replace");
    let source = write_source(&dir, "greet.csa", SOURCE);
    let target = dir.join("greet.csp");
    fs::write(&target, b"stale bytes").expect("seed the old output");

    let output = run(vec![
        OsStr::new("-a"),
        source.as_os_str(),
        OsStr::new("-o"),
        target.as_os_str(),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        fs::read(&target).expect("read the file"),
        expected_image().to_bytes()
    );
}

#[test]
fn the_source_file_is_never_overwritten() {
    let dir = scratch("source-guard");
    let source = write_source(&dir, "greet.csa", SOURCE);
    let before = fs::read(&source).expect("read the source");

    let output = run(vec![
        OsStr::new("-a"),
        source.as_os_str(),
        OsStr::new("-o"),
        source.as_os_str(),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("refusing to overwrite"),
        "stderr: {}",
        stderr(&output)
    );
    assert_eq!(fs::read(&source).expect("read the source"), before);
    assert_eq!(entries(&dir), vec!["greet.csa"]);
}

#[test]
fn a_missing_source_file_is_reported() {
    let dir = scratch("missing-source");
    let missing = dir.join("nope.csa");

    let output = run(vec![OsStr::new("-a"), missing.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot read"),
        "stderr: {}",
        stderr(&output)
    );
    assert_eq!(entries(&dir), Vec::<String>::new());
}

#[test]
fn a_missing_argument_is_a_usage_error() {
    // clap's usage errors are code 2; the assembler's own failures are code 1.
    // Pinned so the two never collapse into one.
    let output = run(Vec::new());

    assert_eq!(output.status.code(), Some(2));
}
