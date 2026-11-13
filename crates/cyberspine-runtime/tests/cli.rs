//! End-to-end tests for the `cyinevm` binary: an image on disk, a body, and the
//! process a user actually runs.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Rig;
use cyberspine_testkit::scratch_dir;

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

fn write_image(dir: &Path, name: &str, source: &str, manifest: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, image(source, manifest)).expect("the image writes");
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

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("fuel exhausted"),
        "stderr: {}",
        stderr(&output)
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

#[test]
fn a_missing_image_is_reported() {
    let dir = scratch("missing-image");
    let missing = dir.join("absent.csp");

    let output = run(vec![
        OsStr::new("-i"),
        missing.as_os_str(),
        OsStr::new("--ticks"),
        OsStr::new("0"),
    ]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("could not read"),
        "stderr: {}",
        stderr(&output)
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

    assert_eq!(output.status.code(), Some(1));
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

    assert_eq!(output.status.code(), Some(1));
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

    assert_eq!(output.status.code(), Some(1));
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
