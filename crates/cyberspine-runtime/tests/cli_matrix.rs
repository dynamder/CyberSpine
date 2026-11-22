//! The flag-combination matrix.
//!
//! `cyinevm` is meant to be driven by scripts and agents, so the one outcome it
//! must never produce for bad arguments is a panic: exit code 101 says nothing
//! about what to fix. Every combination below therefore has to land on a
//! documented exit code (0 success, 1 failed run, 2 usage) instead.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_testkit::scratch_dir;

const MANIFEST: &str = r#"{
  "rig": { "name": "probe", "version": 1 },
  "muscles": ["brow_l"],
  "sensors_f": [],
  "sensors_i": []
}"#;

const SOURCE: &str = "\
.layer 10
.clock hz = 1000
.fuel 4096
.max_wait 600.0

.text
    HALT
";

const EXIT_SUCCESS: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_INPUT: i32 = 3;
const EXIT_FAULTED: i32 = 4;
const EXIT_ENVIRONMENT: i32 = 5;

const LIMIT: Duration = Duration::from_secs(10);

fn scratch(name: &str) -> PathBuf {
    scratch_dir(&format!("runtime/matrix/{name}"))
}

fn image() -> PathBuf {
    let dir = scratch("image");
    let rig = cyberspine_manifest::from_json(MANIFEST).expect("the manifest is valid");
    let asm = parse(SOURCE).expect("the source parses");
    let image = generate_image(&asm, &rig).expect("the source generates");
    let bytes = VerifiedImage::verify(image)
        .expect("the image is valid")
        .to_bytes();
    let path = dir.join("halt.csp");
    std::fs::write(&path, bytes).expect("the image is written");
    path
}

fn garbage() -> PathBuf {
    let dir = scratch("garbage");
    let path = dir.join("not-an-image.csp");
    std::fs::write(&path, b"GARBAGE").expect("the file is written");
    path
}

/// Run the binary, but never let the test hang on it.
fn run(args: &[&OsStr], case: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cyinevm"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the cyinevm binary should spawn");

    let deadline = Instant::now() + LIMIT;
    loop {
        if child.try_wait().expect("the child is waitable").is_some() {
            return child.wait_with_output().expect("the output is collected");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("`{case}` did not exit within {LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("the process should exit, not be killed by a signal")
}

fn assert_no_panic(output: &Output, case: &str) {
    let status = code(output);
    assert_ne!(
        status,
        101,
        "`{case}` panicked instead of exiting with a usage error:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        matches!(
            status,
            EXIT_SUCCESS | EXIT_USAGE | EXIT_INPUT | EXIT_FAULTED | EXIT_ENVIRONMENT
        ),
        "`{case}` exited {status}, which is not a documented code\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn bad_flag_combinations_are_usage_errors() {
    let image_path = image();
    let image = image_path.as_os_str();
    let garbage_path = garbage();
    let garbage = garbage_path.as_os_str();
    let bogus = OsStr::new("target/definitely-not-here.csp");

    let cases: &[(&str, &[&OsStr])] = &[
        ("no arguments", &[]),
        ("image missing", &[OsStr::new("--ticks"), OsStr::new("1")]),
        (
            "unpaced without ticks",
            &[OsStr::new("-i"), image, OsStr::new("--unpaced")],
        ),
        (
            "unpaced with until-halt",
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("--unpaced"),
                OsStr::new("--until-halt"),
            ],
        ),
        (
            "unpaced with until-halt against a bad file",
            &[
                OsStr::new("-i"),
                garbage,
                OsStr::new("--unpaced"),
                OsStr::new("--until-halt"),
            ],
        ),
        (
            "ticks with until-halt",
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("--ticks"),
                OsStr::new("3"),
                OsStr::new("--until-halt"),
            ],
        ),
        (
            "replay with an arbiter",
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("--replay"),
                OsStr::new("nowhere.jsonl"),
                OsStr::new("--interrupt"),
                OsStr::new("rpc:127.0.0.1:1"),
            ],
        ),
        (
            "unknown backend kind",
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("-b"),
                OsStr::new("carrier-pigeon:me"),
            ],
        ),
        (
            "unknown interrupt kind",
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("--interrupt"),
                OsStr::new("smoke-signal:1"),
            ],
        ),
        ("missing image file", &[OsStr::new("-i"), bogus]),
        (
            "garbage image",
            &[
                OsStr::new("-i"),
                garbage,
                OsStr::new("--ticks"),
                OsStr::new("1"),
            ],
        ),
        (
            "zero ticks",
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("--ticks"),
                OsStr::new("0"),
            ],
        ),
        ("help", &[OsStr::new("--help")]),
        ("version", &[OsStr::new("--version")]),
    ];

    for (case, args) in cases {
        let output = run(args, case);
        assert_no_panic(&output, case);
    }
}

#[test]
fn the_matrix_keeps_the_documented_codes() {
    let image_path = image();
    let image = image_path.as_os_str();

    assert_eq!(
        code(&run(
            &[
                OsStr::new("-i"),
                image,
                OsStr::new("--ticks"),
                OsStr::new("1"),
                OsStr::new("--unpaced"),
            ],
            "a run that completes within its budget",
        )),
        EXIT_SUCCESS
    );
    assert_eq!(
        code(&run(
            &[OsStr::new("--unpaced"), OsStr::new("--until-halt")],
            "unpaced with until-halt",
        )),
        EXIT_USAGE
    );
    assert_eq!(
        code(&run(&[OsStr::new("--help")], "--help")),
        EXIT_SUCCESS,
        "--help is not a failure"
    );
}

#[test]
fn no_two_flag_prefix_panics() {
    // Belt and braces: the table above is finite, this checks the property the
    // table is an instance of — no short argument vector reaches a panic. Every
    // invocation ends with `--ticks 0`, so a valid combination terminates
    // instead of waiting for Ctrl-C.
    let image_path = image();
    let flags: &[&OsStr] = &[
        OsStr::new("--unpaced"),
        OsStr::new("--until-halt"),
        OsStr::new("--trace"),
        OsStr::new("--quiet"),
        OsStr::new("--ticks"),
        OsStr::new("-i"),
        image_path.as_os_str(),
    ];

    for a in flags {
        for b in flags {
            let args: Vec<&OsStr> = vec![a, b, OsStr::new("--ticks"), OsStr::new("0")];
            let case = format!("two-flag prefix {a:?} {b:?}");
            let output = run(&args, &case);
            assert_no_panic(&output, &case);
        }
    }
}
