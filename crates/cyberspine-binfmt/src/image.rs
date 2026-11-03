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

use alloc::vec;
use alloc::vec::Vec;

use cyberspine_isa::clock::{Hz, Seconds};
use cyberspine_isa::{Insn, OpCode};
use cyberspine_rig::{Fingerprint, Name};

/// The largest image the loader will accept (§9.1 C0).
///
/// C5 already bounds the *counts* an image declares against its own length, so the
/// loader never allocates from an attacker-controlled number. What it does not
/// bound is the ratio: a table entry costs two or three bytes on disk and sixteen
/// to twenty-four in memory, so a decodable file turns into about four times its
/// size. This constant is what keeps "four times a file someone chose to hand us"
/// from being "four times anything at all".
///
/// A `.csp` this big is already pathological — 4 MiB is half a million
/// instructions — and it lives here rather than in the host so that every path
/// that decodes bytes is covered, not just the one that reads a file.
pub const MAX_IMAGE_LEN: usize = 4 * 1024 * 1024;

/// A handle on one name inside [`Image`]'s arena.
///
/// The image owns its names, so a table entry is a pair of offsets rather than a
/// pointer: `(start, len)` into a single backing buffer. That is what makes
/// loading an image a constant number of allocations instead of one per name, and
/// it is why an `Image` can be moved and dropped freely with no borrowing.
///
/// Both halves are `u32`: the arena is bounded by [`MAX_IMAGE_LEN`] and no rule
/// caps a name's length, so a narrower `len` would invent a rule the format does
/// not have.
///
/// Two handles for the same text are not necessarily equal: a read-back sensor
/// repeats its muscle's name, and each repetition is its own string in the file.
/// Compare the text ([`Image::name`]), never the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NameRef {
    start: u32,
    len: u32,
}

/// The name arena: every name in an image, concatenated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    bytes: Vec<u8>,
}

impl Names {
    /// An arena with room for `bytes` bytes of names.
    ///
    /// The loader knows an exact upper bound before it reads a single table (the
    /// body minus the code section), so reserving once is what turns "one
    /// allocation per name" into one allocation in total.
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
        }
    }

    /// Append `text` and return its handle.
    ///
    /// Infallible: the caller decides what a legal name is (§9.2 M1 for the
    /// loader, [`Name`] for a builder).
    pub fn push(&mut self, text: &str) -> NameRef {
        let start = self.bytes.len() as u32;
        self.bytes.extend_from_slice(text.as_bytes());
        NameRef {
            start,
            len: text.len() as u32,
        }
    }

    /// The text a handle names.
    ///
    /// The bytes were UTF-8 when they were pushed, and nothing can mutate the
    /// arena, so the conversion cannot fail.
    pub fn get(&self, name: NameRef) -> &str {
        let end = (name.start + name.len) as usize;
        core::str::from_utf8(&self.bytes[name.start as usize..end])
            .expect("a name was pushed as UTF-8 and the arena is immutable")
    }
}

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

/// The format version this crate writes, packed as semver in a `u16` (§4):
/// `major:5 · minor:5 · patch:6`.
///
/// Semver is the description everyone already knows: a major change is breaking
/// (C2 refuses it), a minor change must not alter parsing (§10), and a patch
/// change is the same format. Only the major is a gate; see C2 in `load.rs` for
/// why a minor is not.
///
/// The split is `5/5/6` because the three fields are not equally busy: a major
/// bump is rare enough that 32 of them outlive any of us, while a patch bump is
/// the most frequent and gets the most room.
///
/// # History
///
/// 1.0.0 is the first published shape — nothing shipped before it, so no older
/// minor is accepted for compatibility's sake. The last 32 bytes of the header
/// carry the rig fingerprint ([`Fingerprint`]) rather than reserved zeros.
pub const FORMAT_MAJOR: u8 = 1;
pub const FORMAT_MINOR: u8 = 0;
pub const FORMAT_PATCH: u8 = 0;

const MAJOR_SHIFT: u32 = 11;
const MINOR_SHIFT: u32 = 6;
const PATCH_BITS: u16 = 0b0011_1111;

pub const FORMAT_VERSION: u16 = ((FORMAT_MAJOR as u16) << MAJOR_SHIFT)
    | ((FORMAT_MINOR as u16) << MINOR_SHIFT)
    | (FORMAT_PATCH as u16);

/// Split a packed `format_version` back into its three numbers.
pub const fn format_parts(version: u16) -> (u8, u8, u8) {
    (
        (version >> MAJOR_SHIFT) as u8,
        ((version >> MINOR_SHIFT) & 0b0001_1111) as u8,
        (version & PATCH_BITS) as u8,
    )
}

/// Instruction encoding and semantics version (`docs/binfmt.md` §4).
///
/// Currently **1**: the ISA is not frozen yet, so the instruction set is still
/// revised incrementally (`FLUSH` was added this way) and a single added
/// instruction does not step the version. `isa_version` starts moving once the
/// ISA is frozen.
pub const ISA_VERSION: u16 = 1;

/// `hz` default (`docs/isa.md` §6.1). `dt = 1 / hz`.
///
/// Re-exported so a caller does not have to name `isa::clock` for the one
/// constant the format's header documents.
pub use cyberspine_isa::clock::DEFAULT_HZ;

/// `fuel` default (`docs/binfmt.md` §4): the per-resume instruction budget.
pub const DEFAULT_FUEL: u32 = 4096;

/// `max_wait` default (`docs/isa.md` §6.4): 600 virtual seconds.
pub use cyberspine_isa::clock::DEFAULT_MAX_WAIT;

/// The scalar half of the fixed header (§4).
///
/// The other half — `magic`, `flags`, `file_len`, `code_len`, the five counts,
/// `reserved_a`, `reserved_b` and `reserved0`/`reserved1` — is either derived
/// from the image or required to be zero, so it deliberately has no
/// representation here (P3: 布局全推导). `reserved_a` (offset 8) and
/// `reserved_b` (offset 48) are reserved and must be zero; C3 refuses any
/// non-zero value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedHeader {
    pub format_version: u16,
    pub isa_version: u16,
    pub layer: u16,
    pub hz: Hz,
    pub fuel: u32,
    pub max_wait: Seconds,
    /// The rig this image was assembled against (`docs/isa.md` §13). Lives in
    /// the header's last 32 bytes.
    pub rig: Fingerprint,
}

impl Default for FixedHeader {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            isa_version: ISA_VERSION,
            layer: 0,
            hz: Hz::DEFAULT,
            fuel: DEFAULT_FUEL,
            max_wait: Seconds::DEFAULT_MAX_WAIT,
            rig: Fingerprint::UNBOUND,
        }
    }
}

/// A muscle is a name and nothing else (`docs/binfmt.md` §6.2): its index in
/// `Image::muscles` is its id, and its meaning lives entirely in the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Muscle {
    pub name: NameRef,
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
    pub name: NameRef,
    pub source: SensorSource,
}

/// `var_f := name init(u32 = f32 bit pattern)` (§6.4). The initial value is a bit
/// pattern, so NaN payloads, infinities and subnormals all round-trip.
#[derive(Debug, Clone, PartialEq)]
pub struct VarF {
    pub name: NameRef,
    pub init: f32,
}

/// `var_i := name init(i32)` (§6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarI {
    pub name: NameRef,
    pub init: i32,
}

/// A whole action: the semantics of a `.csp` file, before any layout.
///
/// The five tables are in the order §3 and §6 fix for the file:
/// `muscles → sensors_f → sensors_i → vars_f → vars_i`. Their order is part of
/// the program's meaning (§6.2), not an implementation choice.
#[derive(Debug, Clone)]
pub struct Image {
    pub(crate) header: FixedHeader,
    pub(crate) code: Vec<Insn>,
    pub(crate) names: Names,
    pub(crate) muscles: Vec<Muscle>,
    pub(crate) sensors_f: Vec<Sensor>,
    pub(crate) sensors_i: Vec<Sensor>,
    pub(crate) vars_f: Vec<VarF>,
    pub(crate) vars_i: Vec<VarI>,
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
            names: Names::default(),
            muscles: Vec::new(),
            sensors_f: Vec::new(),
            sensors_i: Vec::new(),
            vars_f: Vec::new(),
            vars_i: Vec::new(),
        }
    }

    /// The text a table entry's name handle points at.
    pub fn name(&self, name: NameRef) -> &str {
        self.names.get(name)
    }

    /// The instructions, in execution order.
    pub fn code(&self) -> &[Insn] {
        &self.code
    }

    /// The instructions, for a builder that has to patch one.
    ///
    /// A loaded action cannot reach this: `VerifiedImage` only ever hands out
    /// `&Image`, and the writer refuses an image whose code is no longer
    /// canonical.
    pub fn code_mut(&mut self) -> &mut Vec<Insn> {
        &mut self.code
    }

    /// The five metadata tables, in the order §3 fixes for the file.
    pub fn muscles(&self) -> &[Muscle] {
        &self.muscles
    }

    pub fn sensors_f(&self) -> &[Sensor] {
        &self.sensors_f
    }

    pub fn sensors_i(&self) -> &[Sensor] {
        &self.sensors_i
    }

    pub fn vars_f(&self) -> &[VarF] {
        &self.vars_f
    }

    pub fn vars_i(&self) -> &[VarI] {
        &self.vars_i
    }

    /// Append a muscle on its own.
    ///
    /// This is the M4-violating half of [`Image::push_muscle`]: a test that needs
    /// a file no legal producer can write pushes the muscle here and the read-back
    /// (or not) separately.
    pub fn push_muscle_only(&mut self, name: &Name) {
        let name = self.names.push(name.as_str());
        self.muscles.push(Muscle { name });
    }

    /// Append a muscle together with the read-back sensor the format requires of
    /// it (§6.2), keeping the invariant true by construction.
    ///
    /// To build a file that *violates* the invariant — which §9.2 M4 exists to
    /// catch — push to `muscles` and `sensors_f` independently.
    pub fn push_muscle(&mut self, name: &Name) {
        let name = self.names.push(name.as_str());
        self.muscles.push(Muscle { name });
        self.sensors_f.push(Sensor {
            name,
            source: SensorSource::Muscle,
        });
    }

    pub fn push_sensor_f(&mut self, name: &Name, source: SensorSource) {
        let name = self.names.push(name.as_str());
        self.sensors_f.push(Sensor { name, source });
    }

    pub fn push_sensor_i(&mut self, name: &Name, source: SensorSource) {
        let name = self.names.push(name.as_str());
        self.sensors_i.push(Sensor { name, source });
    }

    pub fn push_var_f(&mut self, name: &Name, init: f32) {
        let name = self.names.push(name.as_str());
        self.vars_f.push(VarF { name, init });
    }

    pub fn push_var_i(&mut self, name: &Name, init: i32) {
        let name = self.names.push(name.as_str());
        self.vars_i.push(VarI { name, init });
    }

    /// The fixed header: the clocks, the budget and the fingerprint.
    ///
    /// Read-only outside the crate: its fields are values with invariants
    /// (`Hz` cannot be zero, `Seconds` is finite and non-negative), so they are
    /// built where the header is decoded and replaced whole — never poked one
    /// field at a time from a caller.
    pub fn header(&self) -> &FixedHeader {
        &self.header
    }

    /// The header, for the one legitimate caller that assembles an image.
    ///
    /// `generate_image` is the producer: it decides every field from the
    /// source's declarations. Everything else reads.
    pub fn header_mut(&mut self) -> &mut FixedHeader {
        &mut self.header
    }

    /// The metadata tables, laid out back to back with no gaps and no length
    /// prefixes (§6). Lengths are derived by writing, never stored (§3).
    fn encode_tables(&self) -> Vec<u8> {
        let mut out = Vec::new();

        for muscle in &self.muscles {
            push_name(&mut out, self.name(muscle.name));
        }
        for sensor in &self.sensors_f {
            push_name(&mut out, self.name(sensor.name));
            out.push(sensor.source.to_u8());
        }
        for sensor in &self.sensors_i {
            push_name(&mut out, self.name(sensor.name));
            out.push(sensor.source.to_u8());
        }
        for var in &self.vars_f {
            push_name(&mut out, self.name(var.name));
            out.extend_from_slice(&var.init.to_bits().to_le_bytes());
        }
        for var in &self.vars_i {
            push_name(&mut out, self.name(var.name));
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

        let mut out = Vec::with_capacity(file_len);
        self.write_fixed_header(&mut out, file_len);

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

    /// §4, in table order, with the fields this writer derives or fixes spelled out
    /// where they are written.
    ///
    /// Written as one function on purpose: the order *is* the layout, so a reader
    /// of §4's table can check this against it line by line. The four literal
    /// zeros are C3's four reserved fields and their comments name them; a helper
    /// that hid them would hide the rule.
    fn write_fixed_header(&self, out: &mut Vec<u8>, file_len: usize) {
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&self.header.format_version.to_le_bytes());
        out.extend_from_slice(&self.header.isa_version.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved_a: must be 0 (§9.1 C3)
        out.extend_from_slice(&0u16.to_le_bytes()); // flags: must be 0 (§9.1 C3)
        push_u32(out, file_len, "file_len");
        push_u32(out, self.code.len(), "code_len");
        out.extend_from_slice(&self.header.layer.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved0: must be 0 (§9.1 C3)
        // The five counts, in the order §3 fixes. Asking for them as a table makes
        // "all five were checked" a property of the code rather than of the eye.
        for (what, count) in [
            ("muscle_count", self.muscles.len()),
            ("sensor_f_count", self.sensors_f.len()),
            ("sensor_i_count", self.sensors_i.len()),
            ("var_f_count", self.vars_f.len()),
            ("var_i_count", self.vars_i.len()),
        ] {
            push_u32(out, count, what);
        }
        out.extend_from_slice(&self.header.hz.get().to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved_b: must be 0 (§9.1 C3)
        out.extend_from_slice(&self.header.fuel.to_le_bytes());
        out.extend_from_slice(&self.header.max_wait.get().to_bits().to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved1: must be 0 (§9.1 C3)
        out.extend_from_slice(&self.header.rig.0); // rig fingerprint (§4)

        debug_assert_eq!(
            out.len(),
            HEADER_LEN,
            "the fixed header must be exactly {HEADER_LEN} bytes"
        );
    }
}

/// One 32-bit header field.
///
/// # Panics
///
/// If the value does not fit the format's field. Callers document that as a
/// builder bug: a `Vec` that large is already a mistake.
fn push_u32(out: &mut Vec<u8>, value: usize, what: &str) {
    let value = u32::try_from(value)
        .unwrap_or_else(|_| panic!("{what} = {value} does not fit the format's 32-bit field"));
    out.extend_from_slice(&value.to_le_bytes());
}

/// Write one NUL-terminated name (§6.1).
///
/// A [`Name`] is an ASCII identifier, so it is non-empty and carries no NUL:
/// the bytes of a legal image are expressible by construction, and the writer
/// has nothing left to refuse.
fn push_name(out: &mut Vec<u8>, name: &str) {
    out.extend_from_slice(name.as_bytes());
    out.push(0);
}
