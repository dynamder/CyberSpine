//! Repository tasks that a single cargo invocation cannot express.

mod docs;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

/// A step's log is truncated here; the terminal still sees every line.
const LOG_LIMIT: usize = 10 * 1024 * 1024;

const FIXTURES: [&str; 4] = [
    "cyberspine-backend-fixture",
    "cyberspine-arbiter-fixture",
    "cyberspine-panic-fixture",
    "cyberspine-demo-maze-dylib",
];

enum Plan {
    Cargo(&'static [&'static str]),
    Docs,
}

const STEPS: [(&str, Plan); 9] = [
    ("fmt", Plan::Cargo(&["fmt", "--all", "--check"])),
    (
        "clippy",
        Plan::Cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--no-deps",
            "--",
            "-D",
            "warnings",
        ]),
    ),
    (
        "nostd",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-isa",
            "-p",
            "cyberspine-rig",
            "-p",
            "cyberspine-binfmt",
            "-p",
            "cyberspine-exec",
            "-p",
            "cyberspine-core",
            "--target",
            "thumbv7em-none-eabihf",
        ]),
    ),
    (
        "runtime-no-default",
        Plan::Cargo(&[
            "clippy",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--all-targets",
            "--no-deps",
            "--",
            "-D",
            "warnings",
        ]),
    ),
    (
        "runtime-dylib",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--features",
            "dylib",
        ]),
    ),
    (
        "runtime-rpc",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-runtime",
            "--no-default-features",
            "--features",
            "rpc",
        ]),
    ),
    (
        "backend-export",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-backend",
            "--no-default-features",
            "--features",
            "export",
        ]),
    ),
    (
        "arbiter-export",
        Plan::Cargo(&[
            "check",
            "-p",
            "cyberspine-arbiter",
            "--no-default-features",
            "--features",
            "export",
        ]),
    ),
    ("docs", Plan::Docs),
];

const SUBCOMMANDS: [(&str, &str); 6] = [
    ("doctor", "check the build environment and worktree hygiene"),
    ("docs", "check the documents against the tree"),
    ("fixtures", "build the shared libraries the tests load"),
    ("test", "build the fixtures, then run the workspace tests"),
    ("check", "run the lint, no_std, feature and document checks"),
    ("ci", "run check, then the workspace tests"),
];

fn usage() -> String {
    let mut text = String::from("usage: cargo xtask <command>\n\n");
    for (name, what) in SUBCOMMANDS {
        text.push_str(&format!("  {name:<10} {what}\n"));
    }
    text.push_str("\n  --json     print the step report as one JSON object\n");
    text
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();

    let json = rest.iter().any(|argument| argument == "--json");
    let rest = rest
        .into_iter()
        .filter(|argument| argument != "--json")
        .collect::<Vec<_>>();

    let outcome = match command.as_deref() {
        Some("doctor") => doctor(),
        Some("docs") => docs_only(),
        Some("fixtures") => fixtures(&rest),
        Some("test") => test(&rest, json),
        Some("check") => check(json),
        Some("ci") => ci(json),
        _ => {
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives beside the workspace root")
        .to_path_buf()
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
}

fn profile(args: &[String]) -> &'static str {
    if args.iter().any(|arg| arg == "--release") {
        "release"
    } else {
        "debug"
    }
}

fn fixture_path(package: &str, profile: &str) -> PathBuf {
    let stem = package.replace('-', "_");
    let name = format!(
        "{}{stem}.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    );
    target_dir().join(profile).join(name)
}

fn run(mut command: Command, what: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("{what}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} exited with {status}"))
    }
}

fn doctor() -> Result<(), String> {
    let root = root();
    let mut complaints = Vec::new();

    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
        let dir = PathBuf::from(dir);
        let absolute = if dir.is_absolute() {
            dir
        } else {
            std::env::current_dir()
                .map_err(|error| format!("current directory: {error}"))?
                .join(dir)
        };
        if !absolute.starts_with(&root) {
            complaints.push(format!(
                "CARGO_TARGET_DIR leaves this checkout: {}\n  \
                 a shared target directory reuses artifacts across source trees, which surfaces \
                 as a compile error no source produces; unset it and let cargo use {}",
                absolute.display(),
                root.join("target").display()
            ));
        }
    }

    for path in worktrees(&root)? {
        if !path.exists() {
            complaints.push(format!(
                "stale worktree: {} (run `git worktree prune`)",
                path.display()
            ));
        }
    }

    if let Some(pinned) = toolchain_channel(&root) {
        let rustc = Command::new("rustc")
            .arg("--version")
            .output()
            .map_err(|error| format!("rustc: {error}"))?;
        let line = String::from_utf8_lossy(&rustc.stdout).into_owned();
        let actual = line.split_whitespace().nth(1).unwrap_or_default();
        if actual != pinned {
            complaints.push(format!(
                "rustc is {actual}, rust-toolchain.toml pins {pinned}"
            ));
        }
    }

    if complaints.is_empty() {
        println!("doctor: ok");
        println!("  root    {}", root.display());
        println!("  target  {}", target_dir().display());
        Ok(())
    } else {
        for complaint in &complaints {
            eprintln!("doctor: {complaint}");
        }
        Err(format!("{} problem(s)", complaints.len()))
    }
}

fn worktrees(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|error| format!("git worktree list: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect())
}

fn toolchain_channel(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("rust-toolchain.toml")).ok()?;
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("channel")?.trim();
        let value = value.strip_prefix('=')?.trim();
        Some(value.trim_matches('"').to_owned())
    })
}

fn fixtures(args: &[String]) -> Result<(), String> {
    let mut build = cargo();
    build.arg("build").arg("--locked");
    for package in FIXTURES {
        build.arg("-p").arg(package);
    }
    build.args(args);
    run(build, "building the fixtures")?;

    let profile = profile(args);
    for package in FIXTURES {
        let path = fixture_path(package, profile);
        if !path.exists() {
            return Err(format!("cargo built no {} at {}", package, path.display()));
        }
        println!("{package} -> {}", path.display());
    }
    Ok(())
}

fn tests(args: &[String], report: &mut Report) {
    // The test build compiles every workspace member that a test depends on,
    // and the backend and arbiter fixtures are dev-dependencies, so their
    // shared libraries come out of it. The demo's library is a `cdylib` only
    // and nothing depends on it; it has to be built here, and building it does
    // not invalidate the test build.
    let mut library = cargo();
    library.args(["build", "--locked", "-p", "cyberspine-demo-maze-dylib"]);
    if !run_logged("fixtures", library, report) {
        return;
    }

    let has_nextest = cargo()
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());

    let mut runner = cargo();
    if has_nextest {
        runner.args(["nextest", "run", "--locked", "--workspace", "--all-targets"]);
    } else {
        eprintln!("tests: cargo test (install cargo-nextest for parallel binaries)");
        runner.args(["test", "--locked", "--workspace", "--all-targets"]);
    }
    runner.args(args);
    if !run_logged("tests", runner, report) {
        return;
    }

    // nextest runs test binaries only; a doc example is still a test.
    let mut doctests = cargo();
    doctests.args(["test", "--locked", "--workspace", "--doc"]);
    run_logged("doctests", doctests, report);
}

fn cargo_step(args: &[&str]) -> Command {
    let mut command = cargo();
    command.arg(args[0]);
    if args[0] != "fmt" {
        command.arg("--locked");
    }
    command.args(&args[1..]);
    command
}

/// One step's outcome: enough of it for a person to read and for a machine to
/// check afterwards.
struct Step {
    name: &'static str,
    ok: bool,
    code: Option<i32>,
    millis: u128,
    log: Option<PathBuf>,
}

struct Report {
    steps: Vec<Step>,
    /// With `--json`, stdout carries the report and nothing else.
    json: bool,
}

impl Report {
    fn new(json: bool) -> Self {
        Self {
            steps: Vec::new(),
            json,
        }
    }

    fn ok(&self) -> bool {
        self.steps.iter().all(|step| step.ok)
    }

    fn failed(&self) -> Vec<&'static str> {
        self.steps
            .iter()
            .filter(|step| !step.ok)
            .map(|step| step.name)
            .collect()
    }

    fn record(&mut self, step: Step) {
        let line = |text: String| {
            if self.json {
                eprintln!("{text}");
            } else {
                println!("{text}");
            }
        };
        if step.ok {
            line(format!(
                "PASS {} {:.1}s",
                step.name,
                step.millis as f64 / 1000.0
            ));
        } else {
            let log = step
                .log
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            eprintln!(
                "FAIL {} {:.1}s ({log})",
                step.name,
                step.millis as f64 / 1000.0
            );
        }
        self.steps.push(step);
    }

    /// The report as one JSON object (`docs/diagnostics.md` uses the same
    /// `schema` field, so a reader of either stream knows how to parse it).
    fn json(&self, root: &Path) -> String {
        let commit = git(root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
        let dirty = git(root, &["status", "--porcelain"]).is_none_or(|text| !text.is_empty());

        let mut text = format!(
            "{{\"schema\":1,\"commit\":{},\"dirty\":{dirty},\"ok\":{},\"steps\":[",
            quote(&commit),
            self.ok()
        );
        for (index, step) in self.steps.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            let log = match &step.log {
                Some(path) => quote(&relative(root, path)),
                None => "null".to_owned(),
            };
            text.push_str(&format!(
                "{{\"name\":{},\"status\":\"{}\",\"exit_code\":{},\"millis\":{},\"log\":{log}}}",
                quote(step.name),
                if step.ok { "pass" } else { "fail" },
                step.code.map_or("null".to_owned(), |code| code.to_string()),
                step.millis,
            ));
        }
        text.push_str("]}");
        text
    }

    fn write(&self, root: &Path) -> Result<(), String> {
        let path = logs_dir().join("report.json");
        std::fs::write(&path, self.json(root))
            .map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn logs_dir() -> PathBuf {
    target_dir().join("verify")
}

fn write_log(name: &str, text: &str) -> Result<PathBuf, String> {
    let directory = logs_dir();
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = directory.join(format!("{name}.log"));
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

/// Run one command, forwarding its output while keeping a copy for the report.
///
/// The copy is capped: a step that prints without end must not fill the disk,
/// and the terminal still sees every line.
fn run_logged(name: &'static str, mut command: Command, report: &mut Report) -> bool {
    let started = Instant::now();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            report.record(Step {
                name,
                ok: false,
                code: None,
                millis: 0,
                log: None,
            });
            eprintln!("xtask: {name}: cannot run: {error}");
            return false;
        }
    };

    // Under `--json` stdout carries the report and nothing else, so a step's
    // own stdout has to go to stderr with the rest of the running commentary.
    let force = report.json;
    let out = child.stdout.take().expect("stdout is piped");
    let err = child.stderr.take().expect("stderr is piped");
    let out_reader = std::thread::spawn(move || relay(out, false, force));
    let err_reader = std::thread::spawn(move || relay(err, true, force));

    let status = child.wait();
    let mut captured = out_reader.join().unwrap_or_default();
    captured.extend_from_slice(&err_reader.join().unwrap_or_default());
    let millis = started.elapsed().as_millis();

    let mut text = String::from_utf8_lossy(&captured).into_owned();
    if text.len() > LOG_LIMIT {
        let cut = (0..=LOG_LIMIT)
            .rev()
            .find(|index| text.is_char_boundary(*index))
            .unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n[truncated]\n");
    }

    let code = status.as_ref().ok().and_then(|status| status.code());
    let ok = status.is_ok_and(|status| status.success());
    report.record(Step {
        name,
        ok,
        code,
        millis,
        log: write_log(name, &text).ok(),
    });
    ok
}

fn relay(mut reader: impl Read, to_stderr: bool, force: bool) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let bytes = &chunk[..read];
                if to_stderr || force {
                    let mut stream = std::io::stderr().lock();
                    let _ = stream.write_all(bytes);
                    let _ = stream.flush();
                } else {
                    let mut stream = std::io::stdout().lock();
                    let _ = stream.write_all(bytes);
                    let _ = stream.flush();
                }
                if captured.len() < LOG_LIMIT {
                    captured.extend_from_slice(bytes);
                }
            }
        }
    }
    captured
}

fn run_docs(name: &'static str, report: &mut Report) {
    let started = Instant::now();
    let root = root();
    let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
    let problems = docs::check(&root);
    let millis = started.elapsed().as_millis();

    let mut text = String::new();
    for problem in &problems {
        let line = problem.replace(&prefix, "");
        eprintln!("{line}");
        text.push_str(&line);
        text.push('\n');
    }

    report.record(Step {
        name,
        ok: problems.is_empty(),
        code: None,
        millis,
        log: write_log(name, &text).ok(),
    });
}

fn lint(report: &mut Report) {
    for (name, plan) in STEPS {
        match plan {
            Plan::Cargo(args) => {
                run_logged(name, cargo_step(args), report);
            }
            Plan::Docs => run_docs(name, report),
        }
    }
}

fn conclude(report: &Report, json: bool) -> Result<(), String> {
    report.write(&root())?;
    if json {
        println!("{}", report.json(&root()));
    }
    let failed = report.failed();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join(", "))
    }
}

fn check(json: bool) -> Result<(), String> {
    let mut report = Report::new(json);
    lint(&mut report);
    conclude(&report, json)
}

fn ci(json: bool) -> Result<(), String> {
    let mut report = Report::new(json);
    lint(&mut report);
    tests(&[], &mut report);
    conclude(&report, json)
}

fn test(args: &[String], json: bool) -> Result<(), String> {
    let mut report = Report::new(json);
    tests(args, &mut report);
    conclude(&report, json)
}

fn docs_only() -> Result<(), String> {
    let mut report = Report::new(false);
    run_docs("docs", &mut report);
    if report.ok() {
        Ok(())
    } else {
        Err(format!("{} step(s) failed", report.failed().len()))
    }
}
