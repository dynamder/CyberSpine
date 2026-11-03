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

/// A fresh directory under the test target, emptied if it already exists.
pub fn scratch_dir(name: &str) -> PathBuf {
    // Set by Cargo for the test process; a library cannot read it at compile
    // time, so it is read at run time and falls back to the temp directory.
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("testkit").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
}
