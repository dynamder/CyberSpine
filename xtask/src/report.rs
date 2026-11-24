//! One step's outcome, and the run it belongs to.
//!
//! A step streams its output while keeping a copy in `target/verify/<step>.log`,
//! and the run ends with `target/verify/report.json`, so a machine can decide
//! what a person would otherwise read. The log names are fixed and the copy is
//! capped: a run replaces the last one instead of accumulating.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::workspace::target_dir;

/// A step's log is truncated here; the terminal still sees every line.
const LOG_LIMIT: usize = 10 * 1024 * 1024;

pub struct Step {
    pub name: &'static str,
    pub ok: bool,
    pub code: Option<i32>,
    pub millis: u128,
    pub log: Option<PathBuf>,
}

pub struct Report {
    steps: Vec<Step>,
    /// With `--json`, stdout carries the report and nothing else.
    json: bool,
    /// With `--no-log`, a step keeps the terminal it was given: a copy would
    /// need a pipe, and a pipe is what turns color and progress off.
    no_log: bool,
}

impl Report {
    pub fn new(json: bool, no_log: bool) -> Self {
        Self {
            steps: Vec::new(),
            json,
            no_log,
        }
    }

    pub fn ok(&self) -> bool {
        self.steps.iter().all(|step| step.ok)
    }

    pub fn failed(&self) -> Vec<&'static str> {
        self.steps
            .iter()
            .filter(|step| !step.ok)
            .map(|step| step.name)
            .collect()
    }

    pub fn record(&mut self, step: Step) {
        let say = |text: String| {
            if self.json {
                eprintln!("{text}");
            } else {
                println!("{text}");
            }
        };
        if step.ok {
            say(format!(
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

    /// The report as one JSON object. `schema` is the same idea as the runtime's
    /// `--report json`: a reader decides the shape by the number, not by guessing.
    pub fn json(&self, root: &Path) -> String {
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

    pub fn write(&self, root: &Path) -> Result<(), String> {
        let path = logs_dir().join("report.json");
        std::fs::write(&path, self.json(root))
            .map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// Run one command, forwarding its output while keeping a copy for the report.
///
/// Under `--json` stdout carries the report and nothing else, so a step's own
/// stdout has to go to stderr with the rest of the running commentary.
pub fn run_logged(name: &'static str, mut command: Command, report: &mut Report) -> bool {
    let started = Instant::now();

    if report.no_log {
        let status = command.status();
        let millis = started.elapsed().as_millis();
        let code = status.as_ref().ok().and_then(|status| status.code());
        let ok = status.is_ok_and(|status| status.success());
        report.record(Step {
            name,
            ok,
            code,
            millis,
            log: None,
        });
        return ok;
    }

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

pub fn write_log(name: &str, text: &str) -> Result<PathBuf, String> {
    let directory = logs_dir();
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let path = directory.join(format!("{name}.log"));
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
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
