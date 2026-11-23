//! Fixtures shared by the workspace's tests.
//!
//! Every integration test used to carry its own copy of "make a `RegId`", "build
//! an `FMOVI`", "build a rig of N muscles", "build the image the assembler would
//! have produced". They drifted — one `action` re-implemented `generate_image`'s
//! table order, another forgot the implicit `HALT` — and each fix had to be made
//! three or four times.
//!
//! This crate is a dev-dependency only: nothing in a shipped binary may use it.

use std::path::PathBuf;

use cyberspine_core::binfmt::image::{Image, SensorSource};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::exec::Sensors;
use cyberspine_core::isa::register::RegId;
use cyberspine_core::isa::{Insn, OpCode};
use cyberspine_core::rig::{Name, Rig};

/// `f0`..`f63` / `i0`..`i63`.
pub fn reg(id: u8) -> RegId {
    RegId::new(id).expect("test fixtures use registers below RegId::COUNT")
}

/// A name that is known to be legal, or a panic that names the fixture.
pub fn name(text: &str) -> Name {
    Name::new(text).expect("test fixture names are identifiers")
}

/// A name built without validating it, for fixtures that have to be illegal.
///
/// The only caller of `Name::new_unchecked` in the workspace, which is what makes
/// "no runtime path builds an invalid name" checkable by grep.
pub fn raw_name(text: &str) -> Name {
    Name::new_unchecked(text)
}

pub fn halt() -> Insn {
    Insn::new(OpCode::Halt)
}

pub fn nop() -> Insn {
    Insn::new(OpCode::Nop)
}

pub fn yield_() -> Insn {
    Insn::new(OpCode::Yield)
}

pub fn flush() -> Insn {
    Insn::new(OpCode::Flush)
}

pub fn fmovi(rd: u8, value: f32) -> Insn {
    Insn {
        rd: reg(rd),
        imm: value.to_bits(),
        ..Insn::new(OpCode::Fmovi)
    }
}

pub fn imovi(rd: u8, value: i32) -> Insn {
    Insn {
        rd: reg(rd),
        imm: value as u32,
        ..Insn::new(OpCode::Imovi)
    }
}

pub fn unary(op: OpCode, rd: u8, ra: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        ..Insn::new(op)
    }
}

pub fn binary(op: OpCode, rd: u8, ra: u8, rb: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        rb: reg(rb),
        ..Insn::new(op)
    }
}

pub fn ternary(op: OpCode, rd: u8, ra: u8, rb: u8, rc: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        rb: reg(rb),
        rc: reg(rc),
        ..Insn::new(op)
    }
}

pub fn compare(op: OpCode, ra: u8, rb: u8) -> Insn {
    Insn {
        ra: reg(ra),
        rb: reg(rb),
        ..Insn::new(op)
    }
}

pub fn jump(offset: i32) -> Insn {
    Insn {
        imm: offset as u32,
        ..Insn::new(OpCode::Jmp)
    }
}

pub fn wait(seconds: f32) -> Insn {
    Insn {
        imm: seconds.to_bits(),
        ..Insn::new(OpCode::Wait)
    }
}

/// Raise interrupt `reason` (an index into the image's reason table).
pub fn intr(reason: u32) -> Insn {
    Insn {
        imm: reason,
        ..Insn::new(OpCode::Intr)
    }
}

pub fn excite(muscle: usize, ra: u8) -> Insn {
    Insn {
        ra: reg(ra),
        imm: muscle as u32,
        ..Insn::new(OpCode::Fexcite)
    }
}

pub fn sense(rd: u8, index: usize) -> Insn {
    Insn {
        rd: reg(rd),
        imm: index as u32,
        ..Insn::new(OpCode::Fsense)
    }
}

pub fn isense(rd: u8, index: usize) -> Insn {
    Insn {
        rd: reg(rd),
        imm: index as u32,
        ..Insn::new(OpCode::Isense)
    }
}

pub fn var_set(index: usize, ra: u8) -> Insn {
    Insn {
        ra: reg(ra),
        imm: index as u32,
        ..Insn::new(OpCode::Fvset)
    }
}

/// No sensors at all: for the tests that never `FSENSE`.
pub fn no_sensors() -> Sensors<'static> {
    Sensors::new(&[], &[])
}

/// An image with the given code and no metadata.
pub fn image(code: Vec<Insn>) -> Image {
    Image::new(code)
}

/// A rig of `count` muscles named `m0`.., and no observables.
pub fn rig(count: usize) -> Rig {
    let mut rig = Rig::new("test", 1);
    for index in 0..count {
        rig = rig.muscle(name(&format!("m{index}")));
    }
    rig
}

/// A rig of the given names, in declaration order.
pub fn rig_with(muscles: &[&str], sensors_f: &[&str], sensors_i: &[&str]) -> Rig {
    let mut rig = Rig::new("test", 1);
    for muscle in muscles {
        rig = rig.muscle(name(muscle));
    }
    for sensor in sensors_f {
        rig = rig.sensor_f(name(sensor));
    }
    for sensor in sensors_i {
        rig = rig.sensor_i(name(sensor));
    }
    rig
}

/// The image `generate_image` would have produced for `code` and `rig`
/// (`docs/asm.md` §3.3): the rig's fingerprint, the muscle table, the read-back
/// sensors, the backend observables, `dt`/`tick`, and the implicit `HALT`.
pub fn action(mut code: Vec<Insn>, rig: &Rig) -> Image {
    if code.last().map(|insn| insn.op) != Some(OpCode::Halt) {
        code.push(halt());
    }

    let mut image = Image::new(code);
    image.header_mut().rig = rig.fingerprint();

    for muscle in &rig.muscles {
        image.push_muscle(&muscle.clone());
    }
    for sensor in &rig.sensors_f {
        image.push_sensor_f(&sensor.clone(), SensorSource::Backend);
    }
    image.push_sensor_f(&name("dt"), SensorSource::BuiltinDt);
    for sensor in &rig.sensors_i {
        image.push_sensor_i(&sensor.clone(), SensorSource::Backend);
    }
    image.push_sensor_i(&name("tick"), SensorSource::BuiltinTick);
    image
}

/// Validate a fixture image. A fixture that does not pass is a broken test, so
/// the failure names the image rather than a rule number.
pub fn verified(image: Image) -> VerifiedImage {
    VerifiedImage::verify(image).expect("the fixture is a legal image")
}

/// The shared library a workspace member builds as a `cdylib`.
///
/// A test binary lives in `<profile>/deps`, so the profile directory is two
/// levels up: the lookup does not depend on `CARGO_TARGET_DIR`, the profile or
/// the platform's library suffix.
pub fn fixture_library(package: &str) -> PathBuf {
    let stem = package.replace('-', "_");
    let file = format!(
        "{}{stem}.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    );
    let exe = std::env::current_exe().expect("the test binary has a path");
    let profile = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("the test binary lives in a profile directory");

    // `cargo build` lifts the library next to the binaries; `cargo test` builds
    // it only into `deps`, so both are searched.
    for dir in [profile.to_path_buf(), profile.join("deps")] {
        let path = dir.join(&file);
        if path.exists() {
            return path;
        }
    }

    panic!(
        "{} is missing; build it with `cargo build -p {package}`, or run `cargo xtask test`",
        profile.join(&file).display()
    );
}

/// Run a binary to completion, killing it if it outlives `limit`.
///
/// A hung child would otherwise hang the whole test binary: `Command::output`
/// has no deadline of its own.
pub fn run_within(
    program: &std::ffi::OsStr,
    args: &[&std::ffi::OsStr],
    limit: std::time::Duration,
) -> std::process::Output {
    run_in(None, program, args, limit)
}

/// [`run_within`], with the child's working directory.
pub fn run_in(
    cwd: Option<&std::path::Path>,
    program: &std::ffi::OsStr,
    args: &[&std::ffi::OsStr],
    limit: std::time::Duration,
) -> std::process::Output {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("{program:?} should spawn: {error}"));

    let mut out = child.stdout.take().expect("stdout is piped");
    let mut err = child.stderr.take().expect("stderr is piped");
    let read_out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out.read_to_end(&mut bytes);
        bytes
    });
    let read_err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = err.read_to_end(&mut bytes);
        bytes
    });

    let deadline = std::time::Instant::now() + limit;
    loop {
        if child.try_wait().expect("the child is waitable").is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{program:?} did not exit within {limit:?}: {args:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    std::process::Output {
        status: child.wait().expect("the child is waitable"),
        stdout: read_out.join().expect("the stdout reader finishes"),
        stderr: read_err.join().expect("the stderr reader finishes"),
    }
}

/// A fresh directory under the test target, emptied if it already exists.
///
/// The process id makes the path unique per test process: a runner that gives
/// every test its own process (nextest) would otherwise let two tests empty the
/// same directory under each other.
pub fn scratch_dir(name: &str) -> PathBuf {
    // Set by Cargo for the test process; a library cannot read it at compile
    // time, so it is read at run time and falls back to the temp directory.
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base
        .join("testkit")
        .join(name)
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
}
