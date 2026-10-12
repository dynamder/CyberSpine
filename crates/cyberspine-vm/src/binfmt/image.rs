//! The in-memory image and its writer: `Image` → `.csp` bytes.
//!
//! `docs/binfmt.md` is the only spec for the byte layout, and this module is its
//! transcription. It is written so that §8's canonical form holds **by
//! construction**: every layout-derived field — `file_len`, `code_len`, the five
//! counts, `flags`, `reserved*` — is *computed* here rather than stored, so there
//! is no way to set one and no second way to encode the same action.
//!
//! That has a consequence worth stating plainly: **a file that violates the
//! layout has no producer in this crate.** The negative tests in `tests/load.rs`
//! patch their bytes in by hand afterwards, which is exactly the claim being
//! tested — a malformed file is indistinguishable from a hand-forged one, and
//! the loader has to reject both.

use crate::vm::op::{Insn, OpCode};

/// `b"CSPX"` (`docs/binfmt.md` §4). A big-endian reader sees `"XPSC"`, which is
/// a byte-order check for free.
pub const MAGIC: [u8; 4] = *b"CSPX";

/// Fixed header length (§4). A format invariant: it must stay a multiple of 8,
/// which is what makes the code section 8-byte aligned without any padding rule.
pub const HEADER_LEN: usize = 96;

/// One instruction is always 8 bytes (§5).
pub const INSN_LEN: usize = 8;

/// `blake3` digest length (§7).
pub const HASH_LEN: usize = 32;

/// The smallest legal file: header, one `HALT`, hash (§3).
pub const MIN_FILE_LEN: usize = HEADER_LEN + INSN_LEN + HASH_LEN;

/// Format version this crate writes: major 1, minor 0 (§4). The high byte is the
/// major number; a loader must reject any other major and any minor above its
/// own (§9.1 C2).
pub const FORMAT_MAJOR: u8 = 1;
pub const FORMAT_MINOR: u8 = 0;
pub const FORMAT_VERSION: u16 = ((FORMAT_MAJOR as u16) << 8) | (FORMAT_MINOR as u16);

/// Instruction encoding and semantics version (`docs/binfmt.md` §4).
///
/// Currently **1**: the ISA is not frozen yet, so the instruction set is still
/// revised incrementally (`FLUSH` was added this way) and a single added
/// instruction does not step the version. `isa_version` starts moving once the
/// ISA is frozen.
pub const ISA_VERSION: u16 = 1;

/// `hz` default (`docs/isa.md` §6.1). `dt = 1 / hz`.
pub const DEFAULT_HZ: u32 = 60;

/// `fuel` default (`docs/binfmt.md` §4): the per-resume instruction budget.
pub const DEFAULT_FUEL: u32 = 4096;

/// `max_wait` default (`docs/isa.md` §6.7): 600 virtual seconds.
pub const DEFAULT_MAX_WAIT: f32 = 600.0;

/// The scalar half of the fixed header (§4).
///
/// The other half — `magic`, `flags`, `file_len`, `code_len`, the five counts,
/// `reserved_a`, `reserved_b` and `reserved0`/`reserved1`/`reserved2` — is either
/// derived from the image or required to be zero, so it deliberately has no
/// representation here (P3: 布局全推导). `reserved_a` (offset 8) and
/// `reserved_b` (offset 48) used to hold `cycle_table_version` and
/// `cycles_per_tick`; the cycle-accounting table is retired (`docs/isa.md` §6.5)
/// and both are now plain zero words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedHeader {
    pub format_version: u16,
    pub isa_version: u16,
    pub layer: u16,
    pub hz: u32,
    pub fuel: u32,
    pub max_wait: f32,
}

impl Default for FixedHeader {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            isa_version: ISA_VERSION,
            layer: 0,
            hz: DEFAULT_HZ,
            fuel: DEFAULT_FUEL,
            max_wait: DEFAULT_MAX_WAIT,
        }
    }
}

/// A muscle is a name and nothing else (`docs/binfmt.md` §6.2): its index in
/// `Image::muscles` is its id, and its meaning lives entirely in the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Muscle {
    pub name: Box<str>,
}

/// Where a sensor's value comes from (§6.3). The discriminants are the on-disk
/// `src` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SensorSource {
    /// 0 — the read-back of a muscle. Legal only in `sensors_f`, only at
    /// `index < muscle_count`, and forced by the read-back invariant (§6.2).
    Muscle = 0,
    /// 1 — a backend observable, bound by name.
    Backend = 1,
    /// 2 — the built-in `dt`. Float sensors only, at most one.
    BuiltinDt = 2,
    /// 3 — the built-in `tick`. Integer sensors only, at most one.
    BuiltinTick = 3,
}

impl SensorSource {
    /// The on-disk `src` byte.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decode an on-disk `src` byte. `None` for a value the format does not
    /// define — §9.2 M5 rejects it rather than ignoring it.
    pub const fn from_u8(src: u8) -> Option<Self> {
        match src {
            0 => Some(Self::Muscle),
            1 => Some(Self::Backend),
            2 => Some(Self::BuiltinDt),
            3 => Some(Self::BuiltinTick),
            _ => None,
        }
    }
}

/// `sensor := name src(u8)` (`docs/binfmt.md` §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sensor {
    pub name: Box<str>,
    pub source: SensorSource,
}

/// `var_f := name init(u32 = f32 bit pattern)` (§6.4). The initial value is a bit
/// pattern, so NaN payloads, infinities and subnormals all round-trip.
#[derive(Debug, Clone, PartialEq)]
pub struct VarF {
    pub name: Box<str>,
    pub init: f32,
}

/// `var_i := name init(i32)` (§6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarI {
    pub name: Box<str>,
    pub init: i32,
}

/// A whole action: the semantics of a `.csp` file, before any layout.
///
/// The five tables are in the order §3 and §6 fix for the file:
/// `muscles → sensors_f → sensors_i → vars_f → vars_i`. Their order is part of
/// the program's meaning (§6.2), not an implementation choice.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub header: FixedHeader,
    pub code: Vec<Insn>,
    pub muscles: Vec<Muscle>,
    pub sensors_f: Vec<Sensor>,
    pub sensors_i: Vec<Sensor>,
    pub vars_f: Vec<VarF>,
    pub vars_i: Vec<VarI>,
}

impl Image {
    /// An image of exactly one instruction, `HALT`, with no metadata. This is the
    /// smallest legal file (§3) and the fixture most tests start from.
    pub fn halt_only() -> Self {
        Self::new(vec![Insn::new(OpCode::Halt)])
    }

    /// An image with the default header, no metadata, and the code given.
    ///
    /// `code` is taken as-is: this writer does not append the trailing `HALT` that
    /// §5 assigns to the binder, and does not check that one is present. That
    /// check belongs to the loader (§9.3 I8), and keeping it out of here is what
    /// lets a test emit a file that fails it.
    pub fn new(code: Vec<Insn>) -> Self {
        Self {
            header: FixedHeader::default(),
            code,
            muscles: Vec::new(),
            sensors_f: Vec::new(),
            sensors_i: Vec::new(),
            vars_f: Vec::new(),
            vars_i: Vec::new(),
        }
    }

    /// Append a muscle together with the read-back sensor the format requires of
    /// it (§6.2), keeping the invariant true by construction.
    ///
    /// To build a file that *violates* the invariant — which §9.2 M4 exists to
    /// catch — push to `muscles` and `sensors_f` independently.
    pub fn push_muscle(&mut self, name: &str) {
        self.muscles.push(Muscle { name: name.into() });
        self.sensors_f.push(Sensor {
            name: name.into(),
            source: SensorSource::Muscle,
        });
    }

    /// The metadata tables, laid out back to back with no gaps and no length
    /// prefixes (§6). Lengths are derived by writing, never stored (§3).
    fn encode_tables(&self) -> Vec<u8> {
        let mut out = Vec::new();

        for muscle in &self.muscles {
            push_name(&mut out, &muscle.name);
        }
        for sensor in &self.sensors_f {
            push_name(&mut out, &sensor.name);
            out.push(sensor.source.to_u8());
        }
        for sensor in &self.sensors_i {
            push_name(&mut out, &sensor.name);
            out.push(sensor.source.to_u8());
        }
        for var in &self.vars_f {
            push_name(&mut out, &var.name);
            out.extend_from_slice(&var.init.to_bits().to_le_bytes());
        }
        for var in &self.vars_i {
            push_name(&mut out, &var.name);
            out.extend_from_slice(&var.init.to_le_bytes());
        }

        out
    }

    /// Serialize to the canonical `.csp` byte string (§8).
    ///
    /// The result always has an accurate `file_len`, zeroed `flags`/`reserved*`,
    /// and a `content_hash` over everything before it. There is no other output
    /// this function can produce for a given image.
    ///
    /// # Panics
    ///
    /// If the image exceeds a 32-bit field — 4 GiB of file, or 2³² instructions.
    /// The format cannot represent such an action, so this is a builder bug rather
    /// than an input error.
    pub fn to_bytes(&self) -> Vec<u8> {
        let tables = self.encode_tables();

        let file_len = HEADER_LEN + INSN_LEN * self.code.len() + tables.len() + HASH_LEN;
        let count = |what: &str, n: usize| {
            u32::try_from(n)
                .unwrap_or_else(|_| panic!("{what} = {n} does not fit the format's 32-bit field"))
        };

        let mut out = Vec::with_capacity(file_len);

        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&self.header.format_version.to_le_bytes());
        out.extend_from_slice(&self.header.isa_version.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved_a: must be 0 (§9.1 C3)
        out.extend_from_slice(&0u16.to_le_bytes()); // flags: must be 0 (§9.1 C3)
        out.extend_from_slice(&count("file_len", file_len).to_le_bytes());
        out.extend_from_slice(&count("code_len", self.code.len()).to_le_bytes());
        out.extend_from_slice(&self.header.layer.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved0: must be 0
        out.extend_from_slice(&count("muscle_count", self.muscles.len()).to_le_bytes());
        out.extend_from_slice(&count("sensor_f_count", self.sensors_f.len()).to_le_bytes());
        out.extend_from_slice(&count("sensor_i_count", self.sensors_i.len()).to_le_bytes());
        out.extend_from_slice(&count("var_f_count", self.vars_f.len()).to_le_bytes());
        out.extend_from_slice(&count("var_i_count", self.vars_i.len()).to_le_bytes());
        out.extend_from_slice(&self.header.hz.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved_b: must be 0 (§9.1 C3)
        out.extend_from_slice(&self.header.fuel.to_le_bytes());
        out.extend_from_slice(&self.header.max_wait.to_bits().to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved1: must be 0
        out.extend_from_slice(&[0u8; 32]); // reserved2: must be all zero
        debug_assert_eq!(
            out.len(),
            HEADER_LEN,
            "the fixed header must be exactly {HEADER_LEN} bytes"
        );

        for insn in &self.code {
            out.extend_from_slice(&insn.encode().to_le_bytes());
        }
        out.extend_from_slice(&tables);

        // §7: the hash covers every byte before it, which is the whole file minus
        // its last 32 bytes. No self-reference rule, no hole in the header.
        out.extend_from_slice(blake3::hash(&out).as_bytes());

        debug_assert_eq!(out.len(), file_len);
        out
    }
}

/// Write one NUL-terminated name (§6.1).
///
/// The name's *validity* is not checked here — §9.2 M1 is the loader's job, and a
/// test needs to be able to emit `""` or `"9bad"` to exercise it. The one thing
/// refused is an interior NUL, because that is not a file this format can express
/// at all: it would silently split one name into two.
fn push_name(out: &mut Vec<u8>, name: &str) {
    let bytes = name.as_bytes();
    assert!(
        !bytes.contains(&0),
        "a name cannot contain NUL: the format has no way to escape it ({name:?})"
    );
    out.extend_from_slice(bytes);
    out.push(0);
}
