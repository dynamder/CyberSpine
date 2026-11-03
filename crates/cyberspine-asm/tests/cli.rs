//! End-to-end tests for the `cyinea` binary: a `.csa` on disk goes in, a `.csp`
//! comes out.
//!
//! The binary is exercised as a process (`CARGO_BIN_EXE_cyinea`) because that is
//! the only way to pin what a user actually gets: the exit code, the message on
//! stderr, and whether a file survived a failed run.
//!
//! `docs/asm.md` §3.2: a source is assembled *for a rig*, named either by its
//! `.rig` directive or by `--rig`, and either path is resolved against the
//! directory the `.csa` lives in.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Rig;
use cyberspine_testkit::scratch_dir;

const SOURCE: &str = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0

.rig \"greet.rig.json\"

.group brows = brow_l, brow_r

.text
    FMOVI   f1, 1.0
    FEXCITE brows, f1
    FLUSH
    FSENSE  f2, smile
    HALT
";

const MANIFEST: &str = r#"{
  "rig": { "name": "greet", "version": 1 },
  "muscles": ["brow_l", "brow_r"],
  "sensors_f": ["smile"],
  "sensors_i": []
}"#;

/// The same names, one identity version later: a different rig, same shape.
const MANIFEST_OTHER: &str = r#"{
  "rig": { "name": "greet", "version": 2 },
  "muscles": ["brow_l", "brow_r"],
  "sensors_f": ["smile"],
  "sensors_i": []
}"#;

fn scratch(name: &str) -> PathBuf {
    scratch_dir(&format!("cli/{name}"))
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

fn write_file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, text).expect("write the fixture file");
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

fn rig_of(text: &str) -> Rig {
    cyberspine_manifest::from_json(text).expect("the test manifest is valid")
}

/// The image the source is supposed to produce, straight from the library.
fn expected_image(manifest: &str) -> Image {
    let asm = parse(SOURCE).expect("the test source parses");
    generate_image(&asm, &rig_of(manifest)).expect("the test source generates an image")
}

#[test]
fn a_source_is_assembled_into_the_requested_file() {
    let dir = scratch("output");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);
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

    let expected = expected_image(MANIFEST);
    let written = fs::read(&target).expect("read the file");
    assert_eq!(
        VerifiedImage::from_bytes(&written)
            .expect("the loader accepts the file")
            .to_bytes(),
        expected.to_bytes(),
        "the file round-trips through the loader byte for byte"
    );
    assert_eq!(
        written,
        expected.to_bytes(),
        "the file is the canonical encoding"
    );
    assert_eq!(
        entries(&dir),
        vec!["greet.csa", "greet.csp", "greet.rig.json"],
        "no temporary file is left"
    );
}

#[test]
fn the_default_output_replaces_the_extension() {
    let dir = scratch("default-output");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(dir.join("greet.csp").exists());
    assert_eq!(
        entries(&dir),
        vec!["greet.csa", "greet.csp", "greet.rig.json"]
    );
}

#[test]
fn a_bare_output_name_lands_in_the_working_directory() {
    let dir = scratch("relative-output");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);

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
    assert_eq!(
        entries(&dir),
        vec!["greet.csa", "greet.rig.json", "out.csp"]
    );
}

// ---------------------------------------------------------------------------
// Where the rig comes from
// ---------------------------------------------------------------------------

/// A relative `.rig` belongs to the source, not to whoever runs the assembler.
#[test]
fn a_relative_manifest_is_resolved_against_the_source_not_the_cwd() {
    let dir = scratch("relative-manifest");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);
    let elsewhere = scratch("relative-manifest-cwd");

    let output = Command::new(env!("CARGO_BIN_EXE_cyinea"))
        .current_dir(&elsewhere)
        .args([OsStr::new("-a"), source.as_os_str()])
        .output()
        .expect("the cyinea binary should run");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(dir.join("greet.csp").exists(), "next to the `.csa`");
    assert_eq!(entries(&elsewhere), Vec::<String>::new());
}

#[test]
fn the_rig_flag_overrides_the_directive() {
    let dir = scratch("rig-override");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);
    let other = write_file(&dir, "other.rig.json", MANIFEST_OTHER);
    let target = dir.join("greet.csp");

    let output = run(vec![
        OsStr::new("-a"),
        source.as_os_str(),
        OsStr::new("-o"),
        target.as_os_str(),
        OsStr::new("--rig"),
        other.as_os_str(),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let written = VerifiedImage::from_bytes(&fs::read(&target).expect("read the file"))
        .expect("the loader accepts the file");
    assert_eq!(
        written.header().rig,
        rig_of(MANIFEST_OTHER).fingerprint(),
        "`--rig` wins over the source's `.rig`"
    );
    assert_ne!(
        written.header().rig,
        expected_image(MANIFEST).header().rig,
        "the directive was not used"
    );
}

#[test]
fn a_source_without_a_manifest_is_reported() {
    let dir = scratch("no-manifest");
    let source = write_file(
        &dir,
        "bare.csa",
        "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.text
    HALT
",
    );

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("no rig manifest"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(!dir.join("bare.csp").exists(), "nothing is written");
    assert_eq!(entries(&dir), vec!["bare.csa"]);
}

#[test]
fn a_missing_manifest_file_is_reported() {
    let dir = scratch("missing-manifest");
    let source = write_file(&dir, "greet.csa", SOURCE);

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot read the rig manifest"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(!dir.join("greet.csp").exists(), "nothing is written");
    assert_eq!(entries(&dir), vec!["greet.csa"]);
}

#[test]
fn a_broken_manifest_is_reported_against_its_own_path() {
    let dir = scratch("broken-manifest");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", "{\"rig\": {\"name\": \"g\"}}");

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("greet.rig.json"),
        "the manifest path is named, not the source: {}",
        stderr(&output)
    );
    assert!(!dir.join("greet.csp").exists(), "nothing is written");
    assert_eq!(entries(&dir), vec!["greet.csa", "greet.rig.json"]);
}

// ---------------------------------------------------------------------------
// Failures leave nothing behind
// ---------------------------------------------------------------------------

#[test]
fn a_parse_error_produces_no_file() {
    let dir = scratch("parse-error");
    // Only `.layer`: the other three fixed-header fields are missing.
    let source = write_file(&dir, "bad.csa", ".layer 10\n");

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
    // `FEXCITE` still pending when `HALT` is reached (`docs/asm.md` §6 第 6 条).
    let source = write_file(
        &dir,
        "pending.csa",
        "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.rig \"greet.rig.json\"
.text
    FMOVI   f1, 1.0
    FEXCITE brow_l, f1
    HALT
",
    );
    write_file(&dir, "greet.rig.json", MANIFEST);

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("FLUSH is missing"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(!dir.join("pending.csp").exists(), "nothing is written");
    assert_eq!(entries(&dir), vec!["greet.rig.json", "pending.csa"]);
}

/// `docs/asm.md` §6.1: the CLI prints `path:line:col: error: …` — for the
/// generation stage too, not just for parse errors.
#[test]
fn a_generation_error_is_rendered_at_its_line() {
    let dir = scratch("generation-location");
    let source = write_file(
        &dir,
        "jump.csa",
        "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.rig \"greet.rig.json\"
.text
    FMOVI   f1, 1.0
    JMP     nowhere
    HALT
",
    );
    write_file(&dir, "greet.rig.json", MANIFEST);

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("undefined label"), "stderr: {message}");
    assert!(
        message.contains("jump.csa:8:5"),
        "the error must carry path:line:col, got: {message}"
    );
    assert!(!dir.join("jump.csp").exists(), "nothing is written");
}

/// Warnings are reported but do not fail the build unless asked to.
#[test]
fn warnings_are_reported_and_denied_on_request() {
    let dir = scratch("warnings");
    let source = write_file(
        &dir,
        "warn.csa",
        "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.rig \"greet.rig.json\"
.text
never_used:
    HALT
",
    );
    write_file(&dir, "greet.rig.json", MANIFEST);

    let output = run(vec![OsStr::new("-a"), source.as_os_str()]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("never a jump target"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(dir.join("warn.csp").exists(), "a warning still assembles");

    let output = run(vec![
        OsStr::new("-a"),
        source.as_os_str(),
        OsStr::new("--deny-warnings"),
    ]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn an_unwritable_destination_is_reported() {
    let dir = scratch("unwritable");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);
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
        vec!["greet.csa", "greet.rig.json"],
        "no temporary file is left"
    );
}

#[test]
fn an_existing_output_is_replaced() {
    let dir = scratch("replace");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);
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
        expected_image(MANIFEST).to_bytes()
    );
}

#[test]
fn the_source_file_is_never_overwritten() {
    let dir = scratch("source-guard");
    let source = write_file(&dir, "greet.csa", SOURCE);
    write_file(&dir, "greet.rig.json", MANIFEST);
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
    assert_eq!(entries(&dir), vec!["greet.csa", "greet.rig.json"]);
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
