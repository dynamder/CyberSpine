//! The loader: `.csp` bytes → an `Image` that passed the whole of §9.
//!
//! `docs/binfmt.md` §9 splits the checklist into three levels and fixes the
//! order — **C (container) → M (metadata) → I (instructions)**. Each rule below
//! is marked with its identifier and gets its own error variant, because a
//! negative test that only asserts "rejected" proves *something* fired, not that
//! the intended rule fired.
//!
//! Two structural decisions come straight out of the spec:
//!
//! * **Two-step load.** The header is parsed and C1–C6 are checked before a
//!   single `Vec` is allocated. §9.1 C5 is the reason: pre-allocating from a
//!   declared count lets a 20-byte file ask for gigabytes.
//! * **The body ends where the hash begins.** The cursor is bounded at
//!   `len - 32`, so a table that overruns its section is a truncation rather
//!   than something that silently eats part of the hash.

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use core::fmt;

use thiserror::Error;

use cyberspine_diag::Diagnostics;
use cyberspine_isa::clock::{Hz, Seconds};
use cyberspine_isa::jump::jump_target;
use cyberspine_isa::{DecodeError, Insn, OpCode};

use super::diag::Warning;
use super::image::{
    FORMAT_MAJOR, FORMAT_MINOR, FixedHeader, HASH_LEN, HEADER_LEN, INSN_LEN, ISA_VERSION, Image,
    MAGIC, MAX_IMAGE_LEN, MIN_FILE_LEN, Muscle, NameRef, Names, Sensor, SensorSource, VarF, VarI,
    format_parts,
};
use super::plan::{Plan, SensorFSource, SensorISource};
use cyberspine_rig::{Fingerprint, Name, NameError};

/// Which metadata table an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Muscles,
    SensorsF,
    SensorsI,
    VarsF,
    VarsI,
}

impl Table {
    /// The spelling §6 uses for the table, and the section name a
    /// [`LoadError::Truncated`] reports while reading it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Muscles => "muscles",
            Self::SensorsF => "sensors_f",
            Self::SensorsI => "sensors_i",
            Self::VarsF => "vars_f",
            Self::VarsI => "vars_i",
        }
    }
}

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every way a byte string can fail to be a valid image.
///
/// The variants follow the §9 checklist; the rule identifier is in each doc
/// comment so a failure can be traced back to the spec line that caused it.
#[derive(Debug, Error)]
pub enum LoadError {
    // ----- §9.1 container -----
    #[error("file is {len} bytes; the format reads at most {max}")]
    TooLarge { len: usize, max: usize }, // C0
    #[error("file is {len} bytes; the format needs at least {MIN_FILE_LEN}")]
    TooShort { len: usize }, // C1
    #[error("bad magic {found:?}; every image starts with {MAGIC:?}")]
    BadMagic { found: [u8; 4] }, // C1
    #[error("header declares file_len = {declared}, but the file is {actual} bytes")]
    FileLenMismatch { declared: u32, actual: usize }, // C1
    #[error(
        "format_version {declared:#06x} is not supported (this loader speaks {FORMAT_MAJOR}.{FORMAT_MINOR})"
    )]
    UnsupportedFormatVersion { declared: u16 }, // C2
    #[error("isa_version {found} is not supported (this loader speaks {ISA_VERSION})")]
    UnsupportedIsaVersion { found: u16 }, // C2
    #[error("`{field}` must be zero; the format does not ignore unknown bits")]
    ReservedNotZero { field: &'static str }, // C3
    #[error("hz must be greater than zero")]
    ZeroHz, // C4
    #[error("max_wait is not a non-negative finite f32 (bit pattern {bits:#010x})")]
    InvalidMaxWait { bits: u32 }, // C4
    #[error("content_hash mismatch: header says {declared:02x?}, contents hash to {computed:02x?}")]
    ContentHashMismatch {
        declared: [u8; 32],
        computed: [u8; 32],
    }, // C6
    #[error("declared counts need at least {lower_bound} bytes but the file is only {file_len}")]
    CountsExceedFileLen { lower_bound: u64, file_len: u64 }, // C5
    #[error("code_len must be at least 1: every action ends in HALT")]
    CodeLenZero, // C8
    #[error("the tables end at byte {consumed}, but the hash begins at byte {expected}")]
    TrailingBytes { consumed: usize, expected: usize }, // C7
    #[error("the last instruction must be HALT")]
    MissingTrailingHalt, // C8 / I8
    #[error(
        "ran off the end of the {section} section at byte {offset}: needed {needed} more byte(s)"
    )]
    Truncated {
        section: &'static str,
        offset: usize,
        needed: usize,
    },

    // ----- §9.2 metadata -----
    #[error("{table}[{index}]: a name must be at least one character")]
    EmptyName { table: Table, index: usize }, // M1
    #[error("{table}[{index}]: {name:?} is not an ASCII identifier ([A-Za-z_][A-Za-z0-9_]*)")]
    NameNotAnIdentifier {
        table: Table,
        index: usize,
        name: String,
    }, // M1
    #[error("{table}[{index}]: the name {name:?} is already used in this table")]
    DuplicateName {
        table: Table,
        index: usize,
        name: String,
    }, // M2
    #[error("sensor_f_count {sensor_f_count} is below muscle_count {muscle_count}")]
    SensorFCountBelowMuscleCount {
        muscle_count: usize,
        sensor_f_count: usize,
    }, // M3
    #[error(
        "sensors_f[{index}] must read back muscles[{index}] ({muscle:?}), but is named {sensor:?}"
    )]
    ReadbackNameMismatch {
        index: usize,
        muscle: String,
        sensor: String,
    }, // M4
    #[error(
        "sensors_f[{index}] must read back muscles[{index}] with src = MUSCLE, but is {found:?}"
    )]
    ReadbackSourceMismatch { index: usize, found: SensorSource }, // M4
    #[error("{table}[{index}]: src {src} is not a defined source")]
    UnknownSensorSource { table: Table, index: usize, src: u8 }, // M5
    #[error("{table}[{index}]: {found:?} is not legal in this table")]
    SensorSourceNotAllowedHere {
        table: Table,
        index: usize,
        found: SensorSource,
    }, // M5
    #[error("sensors_f[{index}]: a second BUILTIN_DT; there can be at most one")]
    DuplicateBuiltinDt { index: usize }, // M5
    #[error("sensors_i[{index}]: a second BUILTIN_TICK; there can be at most one")]
    DuplicateBuiltinTick { index: usize }, // M5
    #[error("{table}[{index}]: MUSCLE is legal only in sensors_f at index < muscle_count")]
    MuscleSourceNotAllowedHere { table: Table, index: usize }, // M6

    // ----- §9.3 instructions -----
    #[error("instruction {index}: {source}")]
    InvalidInstruction {
        index: usize,
        #[source]
        source: DecodeError,
    }, // I1
    #[error("instruction {index} ({word:#018x}) carries bits its opcode does not use")]
    NonCanonicalInstruction { index: usize, word: u64 }, // I2
    #[error("instruction {index}: {table} index {sensor_id} is out of range (count {count})")]
    SensorIndexOutOfRange {
        index: usize,
        table: Table,
        sensor_id: usize,
        count: usize,
    }, // I3
    #[error("instruction {index}: {table} index {var_id} is out of range (count {count})")]
    VarIndexOutOfRange {
        index: usize,
        table: Table,
        var_id: usize,
        count: usize,
    }, // I4
    #[error("instruction {index}: muscle index {muscle_id} is out of range (count {count})")]
    MuscleIndexOutOfRange {
        index: usize,
        muscle_id: usize,
        count: usize,
    }, // I5
    #[error("instruction {index}: target {index} + 1 + ({offset}) is outside [0, {code_len})")]
    JumpTargetOutOfRange {
        index: usize,
        offset: i32,
        code_len: usize,
    }, // I6
    #[error("instruction {index}: WAIT {seconds} is not a non-negative finite duration")]
    WaitNotFiniteOrNegative { index: usize, seconds: f32 }, // I7
    #[error("instruction {index}: WAIT {seconds} exceeds the header's max_wait {max}")]
    WaitExceedsMaxWait {
        index: usize,
        seconds: f32,
        max: f32,
    }, // I7
    #[error("{table}[{index}]: the name is not NUL-terminated before the end of the file")]
    UnterminatedName { table: Table, index: usize }, // M1
}

/// An image that passed `docs/binfmt.md` §9.
///
/// The only ways to get one are [`VerifiedImage::from_bytes`] (decode, then
/// validate) and [`VerifiedImage::verify`] (validate an image the assembler or a
/// test built). Runtime code takes this type rather than [`Image`], so "the
/// loader already checked" stops being a promise and becomes a compile-time fact
/// (`docs/isa.md` §0.4).
///
/// There is no `&mut` access: an action is immutable once it has loaded.
#[derive(Debug, Clone)]
pub struct VerifiedImage(Image, Plan);

impl VerifiedImage {
    /// Decode and verify an image from memory.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        Self::from_bytes_with(bytes, &mut Diagnostics::default())
    }

    /// Decode and verify, collecting everything the loader noticed on the way.
    ///
    /// The diagnostics are *additional* to the `Result`: a warning here never
    /// turns a refusal into a success, and a refusal never suppresses a warning
    /// that explains why the image was odd.
    pub fn from_bytes_with(bytes: &[u8], diagnostics: &mut Diagnostics) -> Result<Self, LoadError> {
        // ----- the container, before a single `Vec` is allocated (C0, C5) -----
        if bytes.len() > MAX_IMAGE_LEN {
            return Err(LoadError::TooLarge {
                len: bytes.len(),
                max: MAX_IMAGE_LEN,
            });
        }
        if bytes.len() < MIN_FILE_LEN {
            return Err(LoadError::TooShort { len: bytes.len() });
        }

        let raw = RawHeader::parse(&bytes[..HEADER_LEN])?;
        check_container(&raw, bytes, diagnostics)?;

        let body_end = bytes.len() - HASH_LEN;
        let mut cursor = Cursor::new(&bytes[..body_end], HEADER_LEN, "code");

        // I1 happens here: a word that is not an instruction fails the decode.
        //
        // I2 is checked twice on purpose, once per representation: here it is
        // the §9.3 equation `encode(decode(word)) == word`, which only the byte
        // path can decide; `check_code` checks the other direction
        // (`decode(encode(insn)) == insn`) so an `Image` built in memory is held
        // to the same canonical form.
        let mut code = Vec::with_capacity(raw.code_len as usize);
        for index in 0..raw.code_len as usize {
            let word = cursor.u64_le()?;
            let insn = Insn::decode(word)
                .map_err(|source| LoadError::InvalidInstruction { index, source })?;
            if insn.encode() != word {
                return Err(LoadError::NonCanonicalInstruction { index, word });
            }
            code.push(insn);
        }

        // The five tables are exactly what is left of the body after the code, so
        // this is an exact upper bound on the names in them — C5 already proved
        // the code section fits. One reservation, no growth, no per-name
        // allocation.
        let tables_span = body_end - HEADER_LEN - INSN_LEN * raw.code_len as usize;
        let mut names = Names::with_capacity(tables_span);
        let muscles =
            read_table::<Muscle>(&mut cursor, &mut names, Table::Muscles, raw.muscle_count)?;
        let sensors_f =
            read_table::<Sensor>(&mut cursor, &mut names, Table::SensorsF, raw.sensor_f_count)?;
        let sensors_i =
            read_table::<Sensor>(&mut cursor, &mut names, Table::SensorsI, raw.sensor_i_count)?;
        let vars_f = read_table::<VarF>(&mut cursor, &mut names, Table::VarsF, raw.var_f_count)?;
        let vars_i = read_table::<VarI>(&mut cursor, &mut names, Table::VarsI, raw.var_i_count)?;

        if cursor.pos != body_end {
            return Err(LoadError::TrailingBytes {
                consumed: cursor.pos,
                expected: body_end,
            }); // C7
        }

        Self::verify(Image {
            header: FixedHeader {
                format_version: raw.format_version,
                isa_version: raw.isa_version,
                layer: raw.layer,
                hz: Hz::new(raw.hz).ok_or(LoadError::ZeroHz)?,
                fuel: raw.fuel,
                max_wait: Seconds::new(f32::from_bits(raw.max_wait_bits)).ok_or(
                    LoadError::InvalidMaxWait {
                        bits: raw.max_wait_bits,
                    },
                )?,
                rig: Fingerprint(raw.rig),
            },
            code,
            names,
            muscles,
            sensors_f,
            sensors_i,
            vars_f,
            vars_i,
        })
    }

    /// Validate an image that was built in memory.
    ///
    /// This is the rule set [`VerifiedImage::from_bytes`] applies minus the
    /// container checks, which need the original bytes.
    pub fn verify(image: Image) -> Result<Self, LoadError> {
        let plan = image.validate()?;
        Ok(Self(image, plan))
    }

    /// The image itself. Mutable access is deliberately absent.
    pub fn image(&self) -> &Image {
        &self.0
    }

    /// What each sensor reads from, resolved by M3–M6.
    ///
    /// Deliberately not an `Image` method: the address form only exists once the
    /// tables have been checked, and `Image` is the pre-verification state. As a
    /// consequence `Image` must never grow a method named `plan` — `VerifiedImage`
    /// derefs to it, and method resolution would silently prefer that one.
    pub fn plan(&self) -> &Plan {
        &self.1
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }
}

impl core::ops::Deref for VerifiedImage {
    type Target = Image;

    fn deref(&self) -> &Image {
        &self.0
    }
}

impl Image {
    /// §9.1 C4 / C8 and §9.2 M1–M6, §9.3 I2–I8: everything a valid byte string
    /// implies once it has been decoded.
    ///
    /// [`VerifiedImage::from_bytes`] runs this after the container checks;
    /// [`VerifiedImage::verify`] runs it on its own. One implementation, so a
    /// rule cannot pass on the disk path and fail on the in-memory one.
    ///
    /// The [`Plan`] is the M3–M6 product: the only thing execution reads that the
    /// bytes do not spell out directly.
    pub fn validate(&self) -> Result<Plan, LoadError> {
        // ----- M, then I, the order §9 fixes -----
        let plan = check_metadata(self)?;
        check_code(self)?;
        Ok(plan)
    }

    /// Compute the content hash the way §7 defines it, for use as a cache key.
    ///
    /// Bytes shorter than the hash cannot carry one; that is an error about the
    /// input, not a "no hash available" value, so it is reported as such.
    pub fn content_hash(bytes: &[u8]) -> Result<blake3::Hash, LoadError> {
        let prefix = bytes
            .len()
            .checked_sub(HASH_LEN)
            .ok_or(LoadError::TooShort { len: bytes.len() })?;
        Ok(blake3::hash(&bytes[..prefix]))
    }
}

// ---------------------------------------------------------------------------
// §9.1 container
// ---------------------------------------------------------------------------

/// Every fixed-header field as it sits on disk, including the ones `FixedHeader`
/// deliberately does not store because the writer derives them.
struct RawHeader {
    format_version: u16,
    isa_version: u16,
    reserved_a: u16,
    flags: u16,
    file_len: u32,
    code_len: u32,
    layer: u16,
    reserved0: u16,
    muscle_count: u32,
    sensor_f_count: u32,
    sensor_i_count: u32,
    var_f_count: u32,
    var_i_count: u32,
    hz: u32,
    reserved_b: u32,
    fuel: u32,
    max_wait_bits: u32,
    reserved1: u32,
    rig: [u8; 32],
}

impl RawHeader {
    /// Read the 96-byte header in §4's field order. The slice is exactly
    /// `HEADER_LEN` bytes, so no read here can fall short.
    ///
    /// The order below is the on-disk order, which is also the order of the §4
    /// table — including the rig fingerprint, which sits at offset 64 and not
    /// next to the magic. Reading in any other order silently transposes every
    /// later field.
    fn parse(header: &[u8]) -> Result<Self, LoadError> {
        let mut c = Cursor::new(header, 0, "fixed header");

        let mut magic = [0u8; 4];
        magic.copy_from_slice(c.take(4)?);
        if magic != MAGIC {
            return Err(LoadError::BadMagic { found: magic }); // C1
        }

        let format_version = c.u16_le()?;
        let isa_version = c.u16_le()?;
        let reserved_a = c.u16_le()?;
        let flags = c.u16_le()?;
        let file_len = c.u32_le()?;
        let code_len = c.u32_le()?;
        let layer = c.u16_le()?;
        let reserved0 = c.u16_le()?;
        let muscle_count = c.u32_le()?;
        let sensor_f_count = c.u32_le()?;
        let sensor_i_count = c.u32_le()?;
        let var_f_count = c.u32_le()?;
        let var_i_count = c.u32_le()?;
        let hz = c.u32_le()?;
        let reserved_b = c.u32_le()?;
        let fuel = c.u32_le()?;
        let max_wait_bits = c.u32_le()?;
        let reserved1 = c.u32_le()?;

        let mut rig = [0u8; 32];
        rig.copy_from_slice(c.take(32)?);
        debug_assert_eq!(
            c.pos, HEADER_LEN,
            "the fixed header is exactly {HEADER_LEN} bytes"
        );

        Ok(Self {
            format_version,
            isa_version,
            reserved_a,
            flags,
            file_len,
            code_len,
            layer,
            reserved0,
            muscle_count,
            sensor_f_count,
            sensor_i_count,
            var_f_count,
            var_i_count,
            hz,
            reserved_b,
            fuel,
            max_wait_bits,
            reserved1,
            rig,
        })
    }
}

/// C1–C6, in the order §9.1 recommends: the hash (C6) is checked *before* the
/// count sanity bound (C5), because it vetoes most bad files without any parsing
/// at all — and neither of them allocates.
fn check_container(
    raw: &RawHeader,
    bytes: &[u8],
    diagnostics: &mut Diagnostics,
) -> Result<(), LoadError> {
    // C1 — the declared length must be the actual length, so every later offset
    // computed from it is trustworthy.
    if raw.file_len as usize != bytes.len() {
        return Err(LoadError::FileLenMismatch {
            declared: raw.file_len,
            actual: bytes.len(),
        });
    }

    // C2 — the major is a gate, the minor is a report, the patch is ignored.
    //
    // A major change is one that breaks parsing, so a loader that does not speak
    // it has nothing to say. A minor change is one the format owner promised does
    // not alter parsing: refusing it would reject a working image, so the loader
    // notes it and lets the ordinary rules decide — an image that *does* change
    // how the bytes are read fails on C7 or C3, right where it happens. A patch
    // change is the same format, so there is nothing to say about it.
    //
    // Below 1.0 semver makes no compatibility promise at all, so a minor mismatch
    // there is a gate too.
    let (major, minor, _patch) = format_parts(raw.format_version);
    if major != FORMAT_MAJOR || (FORMAT_MAJOR == 0 && minor != FORMAT_MINOR) {
        return Err(LoadError::UnsupportedFormatVersion {
            declared: raw.format_version,
        });
    }
    if minor != FORMAT_MINOR {
        diagnostics.push(
            &Warning::NewerMinor {
                declared: minor,
                known: FORMAT_MINOR,
            },
            None,
        );
    }
    if raw.isa_version != ISA_VERSION {
        return Err(LoadError::UnsupportedIsaVersion {
            found: raw.isa_version,
        });
    }

    // C3 — undefined bits are refused, never ignored.
    if raw.flags != 0 {
        return Err(LoadError::ReservedNotZero { field: "flags" });
    }
    if raw.reserved_a != 0 {
        return Err(LoadError::ReservedNotZero {
            field: "reserved_a",
        });
    }
    if raw.reserved0 != 0 {
        return Err(LoadError::ReservedNotZero { field: "reserved0" });
    }
    if raw.reserved_b != 0 {
        return Err(LoadError::ReservedNotZero {
            field: "reserved_b",
        });
    }
    if raw.reserved1 != 0 {
        return Err(LoadError::ReservedNotZero { field: "reserved1" });
    }

    // C8, the early half. `validate` checks this too, for images built in
    // memory; here it also has to be checked *before* the cursor starts reading a
    // code section of length zero, or the table reads begin one instruction too
    // early and report a misleading truncation instead.
    if raw.code_len == 0 {
        return Err(LoadError::CodeLenZero);
    }

    // C4 and C8 live in `Image::validate`: they are properties of the decoded
    // image, so the in-memory path has to check them too, and one implementation
    // is what keeps the two paths from drifting.

    // C6 — one blake3 pass over everything before the hash. A file that fails
    // here is corrupt or forged, and nothing downstream is worth attempting.
    let declared: [u8; 32] = bytes[bytes.len() - HASH_LEN..]
        .try_into()
        .expect("the length was checked above");
    let computed = blake3::hash(&bytes[..bytes.len() - HASH_LEN]);
    if *computed.as_bytes() != declared {
        return Err(LoadError::ContentHashMismatch {
            declared,
            computed: *computed.as_bytes(),
        });
    }

    // C5 — the count bound, and the reason this whole function runs before step 2.
    // Each instruction is 8 bytes and each metadata entry is at least 2 (a
    // one-byte name and its NUL), so this is a lower bound on the file these
    // counts imply. Computed in u64: counts are attacker-controlled u32 values,
    // and `8 * 4e9` must not wrap.
    //
    // C5 bounds the counts *relative to the file*; C0 bounds the file. Both are
    // needed, because C5 alone still permits a linear blow-up: a table entry costs
    // 2–3 bytes on disk and 8–24 in memory, an ~4× expansion on the tight end. C0
    // is what turns "four times a file someone handed us" into "four times 4 MiB".
    //
    // The 2-bytes-per-entry figure is exact for a *valid* file, because §6.1
    // makes an empty name illegal. A file that does contain one is 1 byte short
    // per empty name, so C5 can fire on it before M1 ever sees the name. That is
    // still a rejection, and the bound is the spec's, so it is left as written.
    let entries = raw.muscle_count as u64
        + raw.sensor_f_count as u64
        + raw.sensor_i_count as u64
        + raw.var_f_count as u64
        + raw.var_i_count as u64;
    let lower_bound =
        HEADER_LEN as u64 + INSN_LEN as u64 * raw.code_len as u64 + HASH_LEN as u64 + 2 * entries;
    if lower_bound > bytes.len() as u64 {
        return Err(LoadError::CountsExceedFileLen {
            lower_bound,
            file_len: bytes.len() as u64,
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// §9.2 metadata
// ---------------------------------------------------------------------------

/// M3–M6, plus the [`Plan`] they imply. M1 (name validity) and the unknown-`src`
/// half of M5 are enforced while reading, since that is where the bytes are in hand.
///
/// The plan is built inside the same loops that check the sources, so a plan can
/// never disagree with the rules that admitted the image.
fn check_metadata(image: &Image) -> Result<Plan, LoadError> {
    // M3 — the read-back invariant needs at least one float sensor per muscle.
    if image.sensors_f.len() < image.muscles.len() {
        return Err(LoadError::SensorFCountBelowMuscleCount {
            muscle_count: image.muscles.len(),
            sensor_f_count: image.sensors_f.len(),
        });
    }

    // M4 — for every muscle, sensors_f at the same index reads it back. This is
    // what lets FSENSE and FEXCITE share one integer for a muscle, with no
    // mapping table at bind time and no indirection at run time
    // (`docs/binfmt.md` §6.2).
    for (index, muscle) in image.muscles.iter().enumerate() {
        let sensor = &image.sensors_f[index];
        if image.name(sensor.name) != image.name(muscle.name) {
            return Err(LoadError::ReadbackNameMismatch {
                index,
                muscle: image.name(muscle.name).to_string(),
                sensor: image.name(sensor.name).to_string(),
            });
        }
        if sensor.source != SensorSource::Muscle {
            return Err(LoadError::ReadbackSourceMismatch {
                index,
                found: sensor.source,
            });
        }
    }

    // M5 / M6 — source placement, plus the at-most-one built-ins. The two `k`
    // counters are per table: a backend's `f` and `i` arrays are independent.
    let mut plan = Plan {
        f: Vec::with_capacity(image.sensors_f.len()),
        i: Vec::with_capacity(image.sensors_i.len()),
    };
    let mut seen_dt: Option<usize> = None;
    let mut seen_tick: Option<usize> = None;
    let mut muscles = 0usize;
    let mut backends_f = 0usize;
    let mut backends_i = 0usize;

    for (index, sensor) in image.sensors_f.iter().enumerate() {
        plan.f.push(match sensor.source {
            SensorSource::Muscle => {
                if index >= image.muscles.len() {
                    return Err(LoadError::MuscleSourceNotAllowedHere {
                        table: Table::SensorsF,
                        index,
                    }); // M6
                }
                let muscle = muscles;
                muscles += 1;
                SensorFSource::Muscle(muscle)
            }
            SensorSource::Backend => {
                let observable = backends_f;
                backends_f += 1;
                SensorFSource::Backend(observable)
            }
            SensorSource::BuiltinDt => {
                if seen_dt.replace(index).is_some() {
                    return Err(LoadError::DuplicateBuiltinDt { index });
                }
                SensorFSource::Dt
            }
            // BUILTIN_TICK is legal only in sensors_i (`docs/binfmt.md` §6.3).
            // Refusing it here is also what keeps a float `tick` unrepresentable
            // in `SensorFSource`.
            SensorSource::BuiltinTick => {
                return Err(LoadError::SensorSourceNotAllowedHere {
                    table: Table::SensorsF,
                    index,
                    found: sensor.source,
                });
            }
        });
    }

    for (index, sensor) in image.sensors_i.iter().enumerate() {
        plan.i.push(match sensor.source {
            SensorSource::Backend => {
                let observable = backends_i;
                backends_i += 1;
                SensorISource::Backend(observable)
            }
            SensorSource::BuiltinTick => {
                if seen_tick.replace(index).is_some() {
                    return Err(LoadError::DuplicateBuiltinTick { index });
                }
                SensorISource::Tick
            }
            // MUSCLE is float-only; the two built-ins are each single-table.
            _ => {
                return Err(LoadError::SensorSourceNotAllowedHere {
                    table: Table::SensorsI,
                    index,
                    found: sensor.source,
                });
            }
        });
    }

    Ok(plan)
}

// ---------------------------------------------------------------------------
// §9.3 instructions
// ---------------------------------------------------------------------------

/// I2–I8. One linear pass over the decoded code; everything it needs was already
/// bounded by C and M, which is what lets the runtime skip all of it
/// (`docs/isa.md` §0.4).
///
/// I1 is the decoder's job and lives in `VerifiedImage::from_bytes`: a word that
/// is not an instruction never becomes an `Insn`, so there is nothing to check
/// here.
fn check_code(image: &Image) -> Result<(), LoadError> {
    let code_len = image.code.len();

    // C8 / I8 — the implicit HALT that the binder appends is the last
    // instruction, so "jump to the end" means "jump to code_len - 1".
    if image.code.is_empty() {
        return Err(LoadError::CodeLenZero);
    }
    if image.code.last().map(|insn| insn.op) != Some(OpCode::Halt) {
        return Err(LoadError::MissingTrailingHalt);
    }

    for (index, insn) in image.code.iter().enumerate() {
        // I2 — the canonical-form equation. Decoding zeroes the fields the
        // opcode does not use, so an instruction whose re-encoding does not
        // decode back to itself is exactly one with bits in unused positions.
        let word = insn.encode();
        if Insn::decode(word) != Ok(*insn) {
            return Err(LoadError::NonCanonicalInstruction { index, word });
        }

        match insn.op {
            // I3
            OpCode::Fsense => {
                let imm = insn.imm_index();
                if imm >= image.sensors_f.len() {
                    return Err(LoadError::SensorIndexOutOfRange {
                        index,
                        table: Table::SensorsF,
                        sensor_id: imm,
                        count: image.sensors_f.len(),
                    });
                }
            }
            OpCode::Isense => {
                let imm = insn.imm_index();
                if imm >= image.sensors_i.len() {
                    return Err(LoadError::SensorIndexOutOfRange {
                        index,
                        table: Table::SensorsI,
                        sensor_id: imm,
                        count: image.sensors_i.len(),
                    });
                }
            }

            // I4 — float and integer variables are separate id spaces (§2.4).
            OpCode::Fvget | OpCode::Fvset => {
                let imm = insn.imm_index();
                if imm >= image.vars_f.len() {
                    return Err(LoadError::VarIndexOutOfRange {
                        index,
                        table: Table::VarsF,
                        var_id: imm,
                        count: image.vars_f.len(),
                    });
                }
            }
            OpCode::Ivget | OpCode::Ivset => {
                let imm = insn.imm_index();
                if imm >= image.vars_i.len() {
                    return Err(LoadError::VarIndexOutOfRange {
                        index,
                        table: Table::VarsI,
                        var_id: imm,
                        count: image.vars_i.len(),
                    });
                }
            }

            // I5
            OpCode::Fexcite => {
                let imm = insn.imm_index();
                if imm >= image.muscles.len() {
                    return Err(LoadError::MuscleIndexOutOfRange {
                        index,
                        muscle_id: imm,
                        count: image.muscles.len(),
                    });
                }
            }

            // I6 — the one shared offset function (§1.3), never a local `pc + 1 + off`.
            OpCode::Jmp
            | OpCode::Ijz
            | OpCode::Ijnz
            | OpCode::Flt
            | OpCode::Fle
            | OpCode::Fgt
            | OpCode::Fge
            | OpCode::Ilt
            | OpCode::Ile
            | OpCode::Igt
            | OpCode::Ige
            | OpCode::Ieq
            | OpCode::Ine => {
                let offset = insn.imm_i32();
                let target = jump_target(index, offset).ok_or(LoadError::JumpTargetOutOfRange {
                    index,
                    offset,
                    code_len,
                })?;
                if target >= code_len {
                    return Err(LoadError::JumpTargetOutOfRange {
                        index,
                        offset,
                        code_len,
                    });
                }
            }

            // I7 — WAIT is unbounded by fuel, so the header bounds it instead
            // (`docs/isa.md` §6.4). NaN fails `is_finite`; -0.0 is zero, not
            // negative, and is accepted.
            OpCode::Wait => {
                let seconds =
                    Seconds::new(insn.imm_f32()).ok_or(LoadError::WaitNotFiniteOrNegative {
                        index,
                        seconds: insn.imm_f32(),
                    })?;
                if seconds > image.header.max_wait {
                    return Err(LoadError::WaitExceedsMaxWait {
                        index,
                        seconds: seconds.get(),
                        max: image.header.max_wait.get(),
                    });
                }
            }

            _ => {}
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// A position in a byte string, bounded to one section.
///
/// `section` is what a [`LoadError::Truncated`] names, so the caller can tell
/// a short code section from a short metadata table without decoding the offset.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    section: &'static str,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8], pos: usize, section: &'static str) -> Self {
        Self {
            bytes,
            pos,
            section,
        }
    }

    fn enter(&mut self, section: &'static str) {
        self.section = section;
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(LoadError::Truncated {
                section: self.section,
                offset: self.pos,
                needed: n,
            })?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u16_le(&mut self) -> Result<u16, LoadError> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
    }

    fn u32_le(&mut self) -> Result<u32, LoadError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn u64_le(&mut self) -> Result<u64, LoadError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    /// Read up to the next NUL and consume it (§6.1).
    fn cstr(&mut self, table: Table, index: usize) -> Result<&'a [u8], LoadError> {
        let rest = &self.bytes[self.pos..];
        let nul = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(LoadError::UnterminatedName { table, index })?;
        let name = &rest[..nul];
        self.pos += nul + 1;
        Ok(name)
    }
}

/// One row of a metadata table: how to read it, and what to compare for the
/// duplicate check (M2).
trait Entry: Sized {
    fn read(
        cursor: &mut Cursor<'_>,
        names: &mut Names,
        table: Table,
        index: usize,
    ) -> Result<Self, LoadError>;

    fn name(&self) -> NameRef;
}

impl Entry for Muscle {
    fn read(
        cursor: &mut Cursor<'_>,
        names: &mut Names,
        table: Table,
        index: usize,
    ) -> Result<Self, LoadError> {
        Ok(Self {
            name: read_name(cursor, names, table, index)?,
        })
    }

    fn name(&self) -> NameRef {
        self.name
    }
}

impl Entry for Sensor {
    fn read(
        cursor: &mut Cursor<'_>,
        names: &mut Names,
        table: Table,
        index: usize,
    ) -> Result<Self, LoadError> {
        let name = read_name(cursor, names, table, index)?;
        let src = cursor.take(1)?[0];
        let source = SensorSource::from_u8(src).ok_or(LoadError::UnknownSensorSource {
            table,
            index,
            src,
        })?;
        Ok(Self { name, source })
    }

    fn name(&self) -> NameRef {
        self.name
    }
}

impl Entry for VarF {
    fn read(
        cursor: &mut Cursor<'_>,
        names: &mut Names,
        table: Table,
        index: usize,
    ) -> Result<Self, LoadError> {
        let name = read_name(cursor, names, table, index)?;
        // §6.4: the initial value is a bit pattern, so NaN and both zeros survive.
        let init = f32::from_bits(cursor.u32_le()?);
        Ok(Self { name, init })
    }

    fn name(&self) -> NameRef {
        self.name
    }
}

impl Entry for VarI {
    fn read(
        cursor: &mut Cursor<'_>,
        names: &mut Names,
        table: Table,
        index: usize,
    ) -> Result<Self, LoadError> {
        let name = read_name(cursor, names, table, index)?;
        let init = cursor.u32_le()? as i32;
        Ok(Self { name, init })
    }

    fn name(&self) -> NameRef {
        self.name
    }
}

/// Read `count` entries, enforcing M1 on each name and M2 across the table.
fn read_table<T: Entry>(
    cursor: &mut Cursor<'_>,
    names: &mut Names,
    table: Table,
    count: u32,
) -> Result<Vec<T>, LoadError> {
    // C5 already bounded `count` against the file length.
    cursor.enter(table.as_str());
    let mut out = Vec::with_capacity(count as usize);

    for index in 0..count as usize {
        let entry = T::read(cursor, names, table, index)?;
        // O(n²) on purpose: the tables are tiny, and a `Vec` carries no iteration
        // order that could leak into anything. §9.2 M2 only needs equality.
        //
        // Compared as text, not as handles: two equal names can occupy two
        // separate regions of the arena, so a handle comparison would let a
        // duplicate through.
        if out
            .iter()
            .any(|existing: &T| names.get(existing.name()) == names.get(entry.name()))
        {
            return Err(LoadError::DuplicateName {
                table,
                index,
                name: names.get(entry.name()).to_string(),
            });
        }
        out.push(entry);
    }

    Ok(out)
}

/// Read one name, enforce M1 on it, and intern it in the image's arena.
///
/// §6.1 tightens names to ASCII identifiers in v1 — a deliberate choice, because
/// binding compares names byte for byte and arbitrary UTF-8 would admit
/// look-alikes that bind differently while reading the same.
fn read_name(
    cursor: &mut Cursor<'_>,
    names: &mut Names,
    table: Table,
    index: usize,
) -> Result<NameRef, LoadError> {
    let bytes = cursor.cstr(table, index)?;

    let text = core::str::from_utf8(bytes).map_err(|_| LoadError::NameNotAnIdentifier {
        table,
        index,
        name: String::from_utf8_lossy(bytes).into_owned(),
    })?;

    // M1, checked on the borrowed text: a `Name` here would be an allocation the
    // arena immediately makes redundant.
    Name::validate(text).map_err(|error| match error {
        NameError::Empty => LoadError::EmptyName { table, index },
        NameError::NotAnIdentifier => LoadError::NameNotAnIdentifier {
            table,
            index,
            name: String::from(text),
        },
    })?;

    Ok(names.push(text))
}
