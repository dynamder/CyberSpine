//! End-to-end tests for the `cyinevm` binary: an image on disk, a body, and the
//! process a user actually runs.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::process::{Command, Output};
#[cfg(unix)]
use std::time::Duration;

use cyberspine_asm::assembler::{generate_image, generate_image_debug, parse};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Rig;
use cyberspine_testkit::scratch_dir;

/// The input was understood and rejected (`docs/diagnostics.md` §6).
#[cfg(feature = "rpc")]
const REJECTED: i32 = 3;
/// An instance faulted.
const FAULTED: i32 = 4;
/// The environment failed: a file, a body or an arbiter. Only the tests that
/// build without a backend feature use it.
#[cfg(any(not(feature = "dylib"), not(feature = "rpc")))]
const ENVIRONMENT: i32 = 5;

const MANIFEST: &str = r#"{
  "rig": { "name": "probe", "version": 1 },
  "muscles": ["brow_l", "brow_r"],
  "sensors_f": ["smile"],
  "sensors_i": ["mode"]
}"#;

#[cfg(feature = "rpc")]
const MANIFEST_OTHER: &str = r#"{
  "rig": { "name": "probe", "version": 2 },
  "muscles": ["brow_l", "brow_r"],
  "sensors_f": ["smile"],
  "sensors_i": ["mode"]
}"#;

const SOURCE: &str = "\
.layer 10
.clock hz = 1000
.fuel 4096
.max_wait 600.0

.text
    FMOVI   f1, 0.5
    FEXCITE brow_l, f1
    FLUSH
    YIELD
    HALT
";

const HALT: &str = "\
.layer 10
.clock hz = 1000
.fuel 4096
.max_wait 600.0

.text
    HALT
";

const LOOP: &str = "\
.layer 10
.clock hz = 1000
.fuel 8
.max_wait 600.0

.text
forever:
    JMP     forever
";

/// A loop that never halts on its own, paced slowly enough for a test to
/// interrupt it: 50 ticks is one second.
#[cfg(unix)]
const SPIN: &str = "\
.layer 10
.clock hz = 50
.fuel 4096
.max_wait 600.0

.text
forever:
    JMP     forever
";

fn scratch(name: &str) -> PathBuf {
    scratch_dir(&format!("runtime/{name}"))
}

fn rig_of(text: &str) -> Rig {
    cyberspine_manifest::from_json(text).expect("the test manifest is valid")
}

fn image(source: &str, manifest: &str) -> Vec<u8> {
    let asm = parse(source).expect("the test source parses");
    let image = generate_image(&asm, &rig_of(manifest)).expect("the test source generates");
    VerifiedImage::verify(image)
        .expect("the image is valid")
        .to_bytes()
}

/// A debug build: the same image plus a `pc → source position` table naming
/// `probe.csa` (`docs/asm.md` §8).
fn debug_image(source: &str, manifest: &str) -> Vec<u8> {
    let asm = parse(source).expect("the test source parses");
    let image = generate_image_debug(&asm, &rig_of(manifest), "probe.csa")
        .expect("the test source generates");
    VerifiedImage::verify(image)
        .expect("the image is valid")
        .to_bytes()
}

fn write_image(dir: &Path, name: &str, source: &str, manifest: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, image(source, manifest)).expect("the image writes");
    path
}

fn write_debug_image(dir: &Path, name: &str, source: &str, manifest: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, debug_image(source, manifest)).expect("the image writes");
    path
}

fn run(args: Vec<&OsStr>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cyinevm"))
        .args(args)
        .output()
        .expect("the cyinevm binary should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// In `--json` mode every stderr line is one object; find the one with `code`.
fn json_record_with_code(text: &str, code: &str) -> serde_json::Value {
    let mut found = None;
    for line in text.lines() {
        let value: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("non-json stderr line: {line}"));
        if value["code"] == code {
            found = Some(value);
        }
    }
    found.unwrap_or_else(|| panic!("no `{code}` record in:\n{text}"))
}

#[test]
fn no_backend_uses_the_builtin_dummy_and_warns() {
    let dir = scratch("dummy");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("3"),
        OsStr::new("--unpaced"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("built-in dummy"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(output.stdout.is_empty(), "stdout: {}", stdout(&output));
}

#[test]
fn trace_prints_one_line_per_tick() {
    let dir = scratch("trace");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("3"),
        OsStr::new("--unpaced"),
        OsStr::new("--trace"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let mut lines = stdout(&output)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines.retain(|line| !line.is_empty());
    assert_eq!(
        lines,
        vec![
            "tick\tbrow_l\tbrow_r".to_owned(),
            "0\t0.5\t-".to_owned(),
            "1\t0.5\t-".to_owned(),
            "2\t-\t-".to_owned(),
        ]
    );
}

#[test]
fn a_fault_is_reported_and_fails_the_run() {
    let dir = scratch("fault");
    let image = write_image(&dir, "loop.csp", LOOP, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
    ]);

    assert_eq!(output.status.code(), Some(FAULTED));
    assert!(
        stderr(&output).contains("fuel exhausted"),
        "stderr: {}",
        stderr(&output)
    );
}

/// `docs/runtime.md` §1: `--trace` writes one line per tick, and the tick that
/// faulted is the one a reader most needs to see.
#[test]
fn the_faulting_tick_is_in_the_trace() {
    let dir = scratch("trace-fault");
    let image = write_image(&dir, "loop.csp", LOOP, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("3"),
        OsStr::new("--unpaced"),
        OsStr::new("--trace"),
        OsStr::new("-q"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(FAULTED),
        "stderr: {}",
        stderr(&output)
    );
    let trace = stdout(&output);
    assert!(
        trace.lines().skip(1).any(|line| line.starts_with("0\t")),
        "the faulted tick is missing from the trace:\n{trace}"
    );
}

#[test]
fn a_fault_of_a_debug_build_names_its_source_line() {
    let dir = scratch("fault-debug");
    let image = write_debug_image(&dir, "loop.csp", LOOP, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
    ]);

    let message = stderr(&output);
    assert_eq!(output.status.code(), Some(FAULTED));
    assert!(
        message.contains("fuel exhausted at instruction 0"),
        "the fault names the instruction: {message}"
    );
    assert!(
        message.contains("probe.csa:8:5"),
        "a debug build names the resolved source line: {message}"
    );
}

#[test]
fn until_halt_stops_when_the_instance_is_gone() {
    let dir = scratch("until-halt");
    let image = write_image(&dir, "halt.csp", HALT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--until-halt"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
}

/// `docs/runtime.md` §1: an image that cannot be read is a rejected input, and
/// the report is still written — a terminal record for every exit code.
#[test]
fn a_missing_image_is_a_rejected_input_with_a_report() {
    let dir = scratch("missing-image");
    let missing = dir.join("absent.csp");

    let output = run(vec![
        OsStr::new("-i"),
        missing.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("0"),
        OsStr::new("--report"),
        OsStr::new("json"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(REJECTED),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("could not read"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("\"outcome\":\"rejected\""),
        "the failed run still reports its outcome: {}",
        stdout(&output)
    );
}

/// `docs/runtime.md` §1: an output path that is also an input is refused before
/// anything is opened, the way `cyinea` refuses to write over its source.
#[test]
fn a_decision_log_over_an_input_is_refused() {
    let dir = scratch("decisions-over-input");
    let image = write_image(&dir, "victim.csp", HALT, MANIFEST);
    let before = fs::read(&image).expect("the image is readable");
    let replay = dir.join("run.jsonl");
    fs::write(&replay, "").expect("an empty log is a valid recording");

    for input in [&image, &replay] {
        let mut args = vec![
            OsStr::new("-i"),
            image.as_os_str(),
            OsStr::new("--ticks"),
            OsStr::new("2"),
            OsStr::new("--unpaced"),
            OsStr::new("--decisions"),
            input.as_os_str(),
        ];
        if *input == replay {
            args.extend([OsStr::new("--replay"), replay.as_os_str()]);
        }

        let output = run(args);

        assert_eq!(
            output.status.code(),
            Some(REJECTED),
            "stderr: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("rt.output-overwrite"),
            "stderr: {}",
            stderr(&output)
        );
        assert_eq!(
            fs::read(&image).expect("the image is still readable"),
            before,
            "a refused run leaves the image untouched"
        );
    }
}

/// `docs/runtime.md` §1: `--ticks 0` loads and validates every input, so a path
/// that would fail later still fails now.
#[test]
fn zero_ticks_still_validates_every_input() {
    let dir = scratch("zero-ticks");
    let image = write_image(&dir, "halt.csp", HALT, MANIFEST);
    let missing = dir.join("absent.jsonl");

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("0"),
        OsStr::new("--replay"),
        missing.as_os_str(),
    ]);
    assert_ne!(
        output.status.code(),
        Some(0),
        "a missing recording is not a successful run: stderr: {}",
        stderr(&output)
    );

    let log = dir.join("decisions.jsonl");
    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("0"),
        OsStr::new("--decisions"),
        log.as_os_str(),
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        log.exists(),
        "the decision log is created even when nothing ticks"
    );
}

#[test]
fn the_dummy_spec_is_rejected() {
    let dir = scratch("dummy-spec");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("-b"),
        OsStr::new("dummy:rig.json"),
        OsStr::new("--ticks"),
        OsStr::new("0"),
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("built in"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn a_missing_argument_is_a_usage_error() {
    let output = run(Vec::new());
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn unpaced_requires_a_tick_bound() {
    let dir = scratch("unpaced");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--unpaced"),
    ]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn ticks_and_until_halt_are_exclusive() {
    let dir = scratch("exclusive");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--until-halt"),
    ]);

    assert_eq!(output.status.code(), Some(2));
}

#[cfg(feature = "rpc")]
#[test]
fn a_remote_body_runs_under_strict_identity() {
    use cyberspine_backend::rpc;
    use cyberspine_vm::DummyBackend;

    let dir = scratch("rpc");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);
    let (address, _server) = rpc::spawn(
        "127.0.0.1:0",
        DummyBackend::new(rig_of(MANIFEST)),
        MANIFEST.to_owned(),
        "test".to_owned(),
    )
    .expect("the server binds");

    let spec = format!("rpc:{address}");
    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("-b"),
        OsStr::new(spec.as_str()),
        OsStr::new("--ticks"),
        OsStr::new("3"),
        OsStr::new("--unpaced"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stderr(&output).contains("built-in dummy"),
        "stderr: {}",
        stderr(&output)
    );
}

#[cfg(feature = "rpc")]
#[test]
fn a_remote_body_with_the_wrong_rig_is_refused() {
    use cyberspine_backend::rpc;
    use cyberspine_vm::DummyBackend;

    let dir = scratch("rpc-rig");
    let image = write_image(&dir, "other.csp", SOURCE, MANIFEST_OTHER);
    let (address, _server) = rpc::spawn(
        "127.0.0.1:0",
        DummyBackend::new(rig_of(MANIFEST)),
        MANIFEST.to_owned(),
        "test".to_owned(),
    )
    .expect("the server binds");

    let spec = format!("rpc:{address}");
    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("-b"),
        OsStr::new(spec.as_str()),
        OsStr::new("--ticks"),
        OsStr::new("0"),
    ]);

    assert_eq!(output.status.code(), Some(REJECTED));
    assert!(
        stderr(&output).contains("assembled for rig"),
        "stderr: {}",
        stderr(&output)
    );
}

#[cfg(not(feature = "dylib"))]
#[test]
fn a_dylib_backend_without_the_feature_says_so() {
    let dir = scratch("no-dylib");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("-b"),
        OsStr::new("dylib:bodies/robot.dll"),
        OsStr::new("--ticks"),
        OsStr::new("0"),
    ]);

    assert_eq!(output.status.code(), Some(ENVIRONMENT));
    let message = stderr(&output);
    assert!(message.contains("without the `dylib` feature"), "{message}");
    assert!(message.contains("--features dylib"), "{message}");
}

#[cfg(not(feature = "rpc"))]
#[test]
fn an_rpc_backend_without_the_feature_says_so() {
    let dir = scratch("no-rpc");
    let image = write_image(&dir, "action.csp", SOURCE, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("-b"),
        OsStr::new("rpc:127.0.0.1:1"),
        OsStr::new("--ticks"),
        OsStr::new("0"),
    ]);

    assert_eq!(output.status.code(), Some(ENVIRONMENT));
    let message = stderr(&output);
    assert!(message.contains("without the `rpc` feature"), "{message}");
    assert!(message.contains("--features rpc"), "{message}");
}

// ---------------------------------------------------------------------------
// Several images, and interrupts
// ---------------------------------------------------------------------------

#[test]
fn two_images_load_into_one_session() {
    let dir = scratch("two-images");
    let first = write_image(&dir, "first.csp", HALT, MANIFEST);
    let second = write_image(&dir, "second.csp", HALT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        first.as_os_str(),
        OsStr::new("-i"),
        second.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("instance 0:"), "{message}");
    assert!(message.contains("instance 1:"), "{message}");
    assert!(message.contains("first.csp"), "{message}");
    assert!(message.contains("second.csp"), "{message}");
}

const INTERRUPT: &str = "\
.layer 10
.clock hz = 1000
.fuel 4096
.max_wait 600.0

.text
    FMOVI   f1, 0.5
    INTR    \"why {f1:.1}\"
    HALT
";

#[test]
fn an_interrupt_is_reported_and_the_default_policy_stops_it() {
    let dir = scratch("interrupt-stop");
    let image = write_image(&dir, "interrupt.csp", INTERRUPT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
        OsStr::new("--quiet"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("interrupt: instance 0"),
        "the interrupt must be reported: {message}"
    );
    assert!(message.contains("why 0.5"), "{message}");
    assert!(
        !message.contains("did not apply"),
        "the built-in policy must resolve the ticket: {message}"
    );
}

#[test]
fn an_interrupt_of_a_debug_build_names_its_source_line() {
    let dir = scratch("interrupt-debug");
    let image = write_debug_image(&dir, "interrupt.csp", INTERRUPT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
        OsStr::new("--quiet"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("probe.csa:8:5"),
        "a debug build names the resolved line of the INTR: {message}"
    );
}

#[test]
fn continue_lets_the_instance_halt_on_the_next_tick() {
    let dir = scratch("interrupt-continue");
    let image = write_image(&dir, "interrupt.csp", INTERRUPT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--on-interrupt"),
        OsStr::new("continue"),
        OsStr::new("--until-halt"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("why 0.5"), "{}", stderr(&output));
}

#[test]
fn a_decision_log_replays_the_same_run() {
    let dir = scratch("replay");
    let image = write_image(&dir, "interrupt.csp", INTERRUPT, MANIFEST);
    let log = dir.join("decisions.jsonl");

    let first = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("3"),
        OsStr::new("--unpaced"),
        OsStr::new("--quiet"),
        OsStr::new("--trace"),
        OsStr::new("--on-interrupt"),
        OsStr::new("continue"),
        OsStr::new("--decisions"),
        log.as_os_str(),
    ]);
    assert!(first.status.success(), "stderr: {}", stderr(&first));

    let text = fs::read_to_string(&log).expect("the log is written");
    assert!(text.contains("\"finished\":\"continue\""), "log: {text}");
    assert_eq!(text.lines().count(), 1, "one interrupt, one round: {text}");

    let replay = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("3"),
        OsStr::new("--unpaced"),
        OsStr::new("--quiet"),
        OsStr::new("--trace"),
        OsStr::new("--replay"),
        log.as_os_str(),
    ]);
    assert!(replay.status.success(), "stderr: {}", stderr(&replay));
    assert_eq!(
        stdout(&replay),
        stdout(&first),
        "the replayed trace is the recorded one"
    );
}

#[cfg(feature = "dylib")]
fn arbiter_fixture() -> &'static PathBuf {
    use std::sync::OnceLock;

    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "cyberspine-arbiter-fixture",
                "--message-format=json",
                "--offline",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");
        assert!(
            output.status.success(),
            "cargo could not build the fixture:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if record["reason"] != "compiler-artifact" {
                continue;
            }
            if !record["package_id"]
                .as_str()
                .unwrap_or_default()
                .contains("cyberspine-arbiter-fixture")
            {
                continue;
            }
            for file in record["filenames"].as_array().into_iter().flatten() {
                let path = PathBuf::from(file.as_str().unwrap_or_default());
                if path.extension().and_then(|ext| ext.to_str())
                    == Some(std::env::consts::DLL_EXTENSION)
                {
                    return path;
                }
            }
        }
        panic!("cargo did not report a shared library for the fixture");
    })
}

#[cfg(feature = "dylib")]
#[test]
fn a_dylib_arbiter_rules_in_two_rounds() {
    let dir = scratch("dylib-arbiter");
    let image = write_image(&dir, "interrupt.csp", INTERRUPT, MANIFEST);
    let spec = format!("dylib:{}", arbiter_fixture().display());

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--interrupt"),
        OsStr::new(&spec),
        OsStr::new("--ticks"),
        OsStr::new("4"),
        OsStr::new("--unpaced"),
        OsStr::new("--quiet"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("fixture: first pass"),
        "the first round crossed the boundary: {}",
        stderr(&output)
    );
}

/// The run report is the machine answer to "did it finish, and why".
#[test]
fn a_halted_run_reports_its_outcome() {
    let dir = scratch("report-halted");
    let image = write_image(&dir, "halt.csp", HALT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--until-halt"),
        OsStr::new("--report"),
        OsStr::new("json"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let line = stdout(&output);
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("the report is json");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["kind"], "run");
    assert_eq!(value["outcome"], "halted");
    assert!(value["ticks"].as_u64().expect("ticks is a number") >= 1);
    assert_eq!(
        value["instances"][0]["action"].as_str().unwrap(),
        image.display().to_string()
    );
}

/// A faulted run still produces a report: the failure is the most interesting
/// outcome, not the one that goes unreported.
#[test]
fn a_faulted_run_reports_its_outcome_and_exits_four() {
    let dir = scratch("report-faulted");
    let image = write_image(&dir, "loop.csp", LOOP, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
        OsStr::new("--report"),
        OsStr::new("json"),
    ]);

    assert_eq!(output.status.code(), Some(FAULTED));
    let line = stdout(&output);
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("the report is json");
    assert_eq!(value["outcome"], "faulted");
}

/// `docs/diagnostics.md` §5 has an outcome for Ctrl-C and §8 says a truncated
/// run is not "rounds were never applied" noise. A `--ticks` run interrupted
/// half way is neither a budget nor a divergence.
#[cfg(unix)]
#[test]
fn ctrl_c_reports_stopped_and_says_nothing_about_the_replay() {
    let dir = scratch("ctrl-c");
    let image = write_image(&dir, "spin.csp", SPIN, MANIFEST);
    let replay = dir.join("run.jsonl");
    fs::write(
        &replay,
        "{\"event\":9,\"tick\":0,\"actions\":[],\"completion\":{\"finished\":\"stop\"}}\n",
    )
    .expect("the recording is written");

    let mut child = Command::new(env!("CARGO_BIN_EXE_cyinevm"))
        .args([
            OsStr::new("-i"),
            image.as_os_str(),
            OsStr::new("--ticks"),
            OsStr::new("50"),
            OsStr::new("--replay"),
            replay.as_os_str(),
            OsStr::new("--report"),
            OsStr::new("json"),
            OsStr::new("-q"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the cyinevm binary should spawn");

    std::thread::sleep(Duration::from_millis(500));
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(signal.success(), "the test could not signal the run");

    let output = child.wait_with_output().expect("the run is collected");
    assert_eq!(
        output.status.code(),
        Some(0),
        "Ctrl-C is not a failure: stderr: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("\"outcome\":\"stopped\""),
        "the run reported: {}",
        stdout(&output)
    );
    assert!(
        !stderr(&output).contains("rt.replay-unused"),
        "a truncated run is not a divergence: {}",
        stderr(&output)
    );
}

/// `--json` changes the diagnostic format, never the stream.
#[test]
fn json_diagnostics_stay_on_stderr() {
    let dir = scratch("json-diagnostics");
    let missing = dir.join("absent.csp");

    let output = run(vec![
        OsStr::new("-i"),
        missing.as_os_str(),
        OsStr::new("--json"),
    ]);

    assert_eq!(output.status.code(), Some(REJECTED));
    assert!(
        stdout(&output).is_empty(),
        "stdout is for products, got: {}",
        stdout(&output)
    );

    let text = stderr(&output);
    let value = json_record_with_code(&text, "rt.unreadable-image");
    assert_eq!(value["kind"], "diagnostic");
    assert_eq!(
        value["primary"]["artifact"].as_str().unwrap(),
        missing.display().to_string()
    );
}

/// Interrupts and rulings are records too: `--json` covers the whole run, not
/// just its failures.
#[test]
fn json_events_are_one_object_per_line() {
    let dir = scratch("json-events");
    let image = write_image(&dir, "interrupt.csp", INTERRUPT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("1"),
        OsStr::new("--unpaced"),
        OsStr::new("--quiet"),
        OsStr::new("--json"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    let record = text
        .lines()
        .find(|line| line.starts_with('{'))
        .expect("an event record");
    let value: serde_json::Value = serde_json::from_str(record).expect("the line is json");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["kind"], "event");
    assert_eq!(value["what"], "interrupt");
    assert_eq!(value["instance"], 0);
    assert_eq!(value["reason"], "why 0.5");
}

/// A replay that stops early is not an error, but it is not the recording
/// either: the mismatch is a warning the caller has to be able to see.
#[test]
fn a_replay_that_diverges_is_reported_as_a_warning() {
    let dir = scratch("replay-diverged");
    let log = dir.join("run.jsonl");
    std::fs::write(
        &log,
        "{\"event\":0,\"tick\":0,\"actions\":[],\"completion\":\"unfinished\"}\n",
    )
    .expect("the decision log is written");
    let image = write_image(&dir, "halt.csp", HALT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--until-halt"),
        OsStr::new("--replay"),
        log.as_os_str(),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    assert!(text.contains("warning[rt.replay-unused]"), "{text}");
    assert!(
        text.contains("1 recorded round(s) were never applied"),
        "{text}"
    );
}

#[test]
fn a_diverged_replay_is_a_json_warning_too() {
    let dir = scratch("replay-diverged-json");
    let log = dir.join("run.jsonl");
    std::fs::write(
        &log,
        "{\"event\":0,\"tick\":0,\"actions\":[],\"completion\":\"unfinished\"}\n",
    )
    .expect("the decision log is written");
    let image = write_image(&dir, "halt.csp", HALT, MANIFEST);

    let output = run(vec![
        OsStr::new("-i"),
        image.as_os_str(),
        OsStr::new("--until-halt"),
        OsStr::new("--replay"),
        log.as_os_str(),
        OsStr::new("--json"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stderr(&output);
    let value = json_record_with_code(&text, "rt.replay-unused");
    assert_eq!(value["kind"], "diagnostic");
    assert_eq!(value["level"], "warning");
}
