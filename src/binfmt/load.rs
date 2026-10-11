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

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::vm::jump::jump_target;
use crate::vm::op::{DecodeError, Insn, OpCode};

use super::image::{
    CYCLE_TABLE_VERSION, FORMAT_MAJOR, FORMAT_MINOR, FixedHeader, HASH_LEN, HEADER_LEN, INSN_LEN,
    ISA_VERSION, Image, MAGIC, MAX_CYCLES_V1, MIN_FILE_LEN, Muscle, Sensor, SensorSource, VarF,
    VarI,
};

/// The largest file accepted from disk, in bytes.
///
/// §9.1 C5 stops the loader allocating from a *declared* count, but it cannot
/// help with the read itself: `fs::read` allocates by actual file size before any
/// of that runs. So the disk entry point needs its own gate, and this is it.
pub const MAX_FILE_LEN: usize = 64 * 1024 * 1024;

/// Which metadata table an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Muscles,
    SensorsF,
    SensorsI,
    VarsF,
    VarsI,
}

impl std::fmt::Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Muscles => "muscles",
            Self::SensorsF => "sensors_f",
            Self::SensorsI => "sensors_i",
            Self::VarsF => "vars_f",
            Self::VarsI => "vars_i",
        };
        f.write_str(name)
    }
}

/// Every way a byte string can fail to be a valid image.
///
/// The variants follow the §9 checklist; the rule identifier is in each doc
/// comment so a failure can be traced back to the spec line that caused it.
#[derive(Debug, Error)]
pub enum LoadError {
    // ----- reading from disk (not a format rule) -----
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("file is {len} bytes, above the {max}-byte limit this loader will read")]
    FileTooLarge { len: usize, max: usize },

    // ----- §9.1 container -----
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
    #[error(
        "cycle_table_version {found} is not supported (this loader speaks {CYCLE_TABLE_VERSION})"
    )]
    UnsupportedCycleTableVersion { found: u16 }, // C2
    #[error("`{field}` must be zero; the format does not ignore unknown bits")]
    ReservedNotZero { field: &'static str }, // C3
    #[error("hz must be greater than zero")]
    ZeroHz, // C4
    #[error("max_wait is not a non-negative finite f32 (bit pattern {bits:#010x})")]
    InvalidMaxWait { bits: u32 }, // C4
    #[error(
        "cycles_per_tick {declared} is below the {minimum}-cycle cost of the most expensive instruction"
    )]
    CyclesPerTickTooSmall { declared: u32, minimum: u32 }, // C4
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
    #[error("ran off the end of the section at byte {offset}: needed {needed} more byte(s)")]
    Truncated { offset: usize, needed: usize },

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

impl Image {
    /// Read and fully verify a `.csp` file from disk.
    ///
    /// The size gates run on the file's metadata *before* it is read, so an
    /// oversized or undersized file never becomes an allocation. See
    /// [`MAX_FILE_LEN`] for why this cannot be left to [`Image::from_bytes`].
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| LoadError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        let len = file
            .metadata()
            .map_err(|source| LoadError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .len();

        // Gate on the length we are about to trust, before trusting it.
        if (len as usize) < MIN_FILE_LEN {
            return Err(LoadError::TooShort { len: len as usize });
        }
        if (len as usize) > MAX_FILE_LEN {
            return Err(LoadError::FileTooLarge {
                len: len as usize,
                max: MAX_FILE_LEN,
            });
        }

        // Read one byte past the declared length: if the file grew between the
        // metadata call and now, the extra byte shows up and C1 rejects the
        // mismatch instead of the loader silently reading a partial image.
        let mut bytes = Vec::with_capacity(len as usize + 1);
        file.take(len + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| LoadError::Io {
                path: path.to_path_buf(),
                source,
            })?;

        Self::from_bytes(&bytes)
    }

    /// Verify and parse an image from memory.
    ///
    /// On success the image has passed §9.1 C1–C8, §9.2 M1–M6 and §9.3 I1–I8, so
    /// the runtime that consumes it owes it no further bounds checks
    /// (`docs/isa.md` §0.4).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LoadError> {
        // ----- Step 1: the fixed header, and every check that needs no parsing.
        // Nothing below allocates until `check_container` has passed (C5).
        if bytes.len() < MIN_FILE_LEN {
            return Err(LoadError::TooShort { len: bytes.len() });
        }

        let raw = RawHeader::parse(&bytes[..HEADER_LEN])?;
        check_container(&raw, bytes)?;

        // ----- Step 2: allocate, now that the counts are known to be sane.
        let body_end = bytes.len() - HASH_LEN;
        let mut cursor = Cursor::new(&bytes[..body_end], HEADER_LEN);

        // The code section is read as raw words; I1/I2 are checked in
        // `check_code` (the I phase), after M — the order §9 fixes.
        let mut raw_code = Vec::with_capacity(raw.code_len as usize);
        for _ in 0..raw.code_len {
            raw_code.push(cursor.u64_le()?);
        }

        let muscles = read_names(&mut cursor, Table::Muscles, raw.muscle_count)?;
        let sensors_f = read_sensors(&mut cursor, Table::SensorsF, raw.sensor_f_count)?;
        let sensors_i = read_sensors(&mut cursor, Table::SensorsI, raw.sensor_i_count)?;
        let vars_f = read_vars_f(&mut cursor, raw.var_f_count)?;
        let vars_i = read_vars_i(&mut cursor, raw.var_i_count)?;

        if cursor.pos != body_end {
            return Err(LoadError::TrailingBytes {
                consumed: cursor.pos,
                expected: body_end,
            }); // C7
        }

        let max_wait = f32::from_bits(raw.max_wait_bits);
        let mut image = Image {
            header: FixedHeader {
                format_version: raw.format_version,
                isa_version: raw.isa_version,
                cycle_table_version: raw.cycle_table_version,
                layer: raw.layer,
                hz: raw.hz,
                cycles_per_tick: raw.cycles_per_tick,
                fuel: raw.fuel,
                max_wait,
            },
            code: Vec::new(),
            muscles,
            sensors_f,
            sensors_i,
            vars_f,
            vars_i,
        };

        // ----- M, then I. Both are O(n) single passes; the runtime pays nothing.
        check_metadata(&image)?;
        image.code = check_code(&raw_code, &image)?;

        Ok(image)
    }

    /// Compute the content hash the way §7 defines it, for use as a cache key.
    pub fn content_hash(bytes: &[u8]) -> Option<blake3::Hash> {
        let prefix = bytes.len().checked_sub(HASH_LEN)?;
        Some(blake3::hash(&bytes[..prefix]))
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
    cycle_table_version: u16,
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
    cycles_per_tick: u32,
    fuel: u32,
    max_wait_bits: u32,
    reserved1: u32,
    reserved2: [u8; 32],
}

impl RawHeader {
    /// Read the 96-byte header in §4's field order. The slice is exactly
    /// `HEADER_LEN` bytes, so no read here can fall short.
    ///
    /// The order below is the on-disk order, which is also the order of the §4
    /// table — including `reserved2`, which sits at offset 64 and not next to the
    /// magic. Reading in any other order silently transposes every later field.
    fn parse(header: &[u8]) -> Result<Self, LoadError> {
        let mut c = Cursor::new(header, 0);

        let mut magic = [0u8; 4];
        magic.copy_from_slice(c.take(4)?);
        if magic != MAGIC {
            return Err(LoadError::BadMagic { found: magic }); // C1
        }

        let format_version = c.u16_le()?;
        let isa_version = c.u16_le()?;
        let cycle_table_version = c.u16_le()?;
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
        let cycles_per_tick = c.u32_le()?;
        let fuel = c.u32_le()?;
        let max_wait_bits = c.u32_le()?;
        let reserved1 = c.u32_le()?;

        let mut reserved2 = [0u8; 32];
        reserved2.copy_from_slice(c.take(32)?);
        debug_assert_eq!(
            c.pos, HEADER_LEN,
            "the fixed header is exactly {HEADER_LEN} bytes"
        );

        Ok(Self {
            format_version,
            isa_version,
            cycle_table_version,
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
            cycles_per_tick,
            fuel,
            max_wait_bits,
            reserved1,
            reserved2,
        })
    }
}

/// C1–C6, in the order §9.1 recommends: the hash (C6) is checked *before* the
/// count sanity bound (C5), because it vetoes most bad files without any parsing
/// at all — and neither of them allocates.
fn check_container(raw: &RawHeader, bytes: &[u8]) -> Result<(), LoadError> {
    // C1 — the declared length must be the actual length, so every later offset
    // computed from it is trustworthy.
    if raw.file_len as usize != bytes.len() {
        return Err(LoadError::FileLenMismatch {
            declared: raw.file_len,
            actual: bytes.len(),
        });
    }

    // C2 — one supported version of each kind. "In the supported set" is a
    // singleton set today; the comparison is written as membership so that adding
    // a version is a change to the set, not to the shape of the check.
    let major = (raw.format_version >> 8) as u8;
    let minor = (raw.format_version & 0xFF) as u8;
    if major != FORMAT_MAJOR || minor > FORMAT_MINOR {
        return Err(LoadError::UnsupportedFormatVersion {
            declared: raw.format_version,
        });
    }
    if raw.isa_version != ISA_VERSION {
        return Err(LoadError::UnsupportedIsaVersion {
            found: raw.isa_version,
        });
    }
    if raw.cycle_table_version != CYCLE_TABLE_VERSION {
        return Err(LoadError::UnsupportedCycleTableVersion {
            found: raw.cycle_table_version,
        });
    }

    // C3 — undefined bits are refused, never ignored.
    if raw.flags != 0 {
        return Err(LoadError::ReservedNotZero { field: "flags" });
    }
    if raw.reserved0 != 0 {
        return Err(LoadError::ReservedNotZero { field: "reserved0" });
    }
    if raw.reserved1 != 0 {
        return Err(LoadError::ReservedNotZero { field: "reserved1" });
    }
    if raw.reserved2.iter().any(|&b| b != 0) {
        return Err(LoadError::ReservedNotZero { field: "reserved2" });
    }

    // C4 — the clock must be usable and the tick budget must be able to hold the
    // most expensive instruction, or such a tick could never complete.
    if raw.hz == 0 {
        return Err(LoadError::ZeroHz);
    }
    let max_wait = f32::from_bits(raw.max_wait_bits);
    if !max_wait.is_finite() || max_wait < 0.0 {
        return Err(LoadError::InvalidMaxWait {
            bits: raw.max_wait_bits,
        });
    }
    if raw.cycles_per_tick < MAX_CYCLES_V1 {
        return Err(LoadError::CyclesPerTickTooSmall {
            declared: raw.cycles_per_tick,
            minimum: MAX_CYCLES_V1,
        });
    }

    // C8, first half. Checked here rather than with the code section so that a
    // zero-instruction file is refused before anything is allocated for it.
    if raw.code_len == 0 {
        return Err(LoadError::CodeLenZero);
    }

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

    // C5 — the anti-allocation-bomb bound, and the reason this whole function
    // runs before step 2. Each instruction is 8 bytes and each metadata entry is
    // at least 2 (a one-byte name and its NUL), so this is a lower bound on the
    // file these counts imply. Computed in u64: counts are attacker-controlled
    // u32 values, and `8 * 4e9` must not wrap.
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

/// M3–M6. M1 (name validity) and the unknown-`src` half of M5 are enforced while
/// reading, since that is where the bytes are in hand.
fn check_metadata(image: &Image) -> Result<(), LoadError> {
    // M3 — the read-back invariant needs at least one float sensor per muscle.
    if image.sensors_f.len() < image.muscles.len() {
        return Err(LoadError::SensorFCountBelowMuscleCount {
            muscle_count: image.muscles.len(),
            sensor_f_count: image.sensors_f.len(),
        });
    }

    // M4 — for every muscle, sensors_f at the same index reads it back. This is
    // what lets FSENSE and FEXCITE share one integer for a muscle, with no
    // mapping table at bind time and no indirection at run time (§6.2).
    for (index, muscle) in image.muscles.iter().enumerate() {
        let sensor = &image.sensors_f[index];
        if sensor.name != muscle.name {
            return Err(LoadError::ReadbackNameMismatch {
                index,
                muscle: muscle.name.to_string(),
                sensor: sensor.name.to_string(),
            });
        }
        if sensor.source != SensorSource::Muscle {
            return Err(LoadError::ReadbackSourceMismatch {
                index,
                found: sensor.source,
            });
        }
    }

    // M5 / M6 — source placement, plus the at-most-one built-ins.
    let mut seen_dt: Option<usize> = None;
    let mut seen_tick: Option<usize> = None;

    for (index, sensor) in image.sensors_f.iter().enumerate() {
        match sensor.source {
            SensorSource::Muscle => {
                if index >= image.muscles.len() {
                    return Err(LoadError::MuscleSourceNotAllowedHere {
                        table: Table::SensorsF,
                        index,
                    }); // M6
                }
            }
            SensorSource::Backend => {}
            SensorSource::BuiltinDt => {
                if seen_dt.is_some() {
                    return Err(LoadError::DuplicateBuiltinDt { index });
                }
                seen_dt = Some(index);
            }
            // BUILTIN_TICK is legal only in sensors_i (§6.3).
            SensorSource::BuiltinTick => {
                return Err(LoadError::SensorSourceNotAllowedHere {
                    table: Table::SensorsF,
                    index,
                    found: sensor.source,
                });
            }
        }
    }

    for (index, sensor) in image.sensors_i.iter().enumerate() {
        match sensor.source {
            SensorSource::Backend => {}
            SensorSource::BuiltinTick => {
                if seen_tick.is_some() {
                    return Err(LoadError::DuplicateBuiltinTick { index });
                }
                seen_tick = Some(index);
            }
            // MUSCLE is float-only; the two built-ins are each single-table.
            _ => {
                return Err(LoadError::SensorSourceNotAllowedHere {
                    table: Table::SensorsI,
                    index,
                    found: sensor.source,
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// §9.3 instructions
// ---------------------------------------------------------------------------

/// I1–I8. One linear pass over the raw words; everything it needs was already
/// bounded by C and M, which is what lets the runtime skip all of it
/// (`docs/isa.md` §0.4).
///
/// Returns the decoded instructions, which become `Image::code`. Decoding here
/// rather than at execution time is what lets the runtime read a canonical
/// `Insn` with no per-tick word parsing.
fn check_code(raw_code: &[u64], image: &Image) -> Result<Vec<Insn>, LoadError> {
    let code_len = raw_code.len();

    // C8, second half / I8 — the implicit HALT that the binder appends is the
    // last instruction here, so "jump to the end" means "jump to code_len - 1".
    if !matches!(
        raw_code.last().map(|&word| Insn::decode(word)),
        Some(Ok(insn)) if insn.op == OpCode::Halt
    ) {
        return Err(LoadError::MissingTrailingHalt);
    }

    let mut code = Vec::with_capacity(code_len);

    for (index, &word) in raw_code.iter().enumerate() {
        // I1 — a reserved opcode is refused outright.
        let insn = Insn::decode(word)
            .map_err(|source| LoadError::InvalidInstruction { index, source })?;

        // I2 — the canonical-form equation. Decoding zeroes the fields the
        // opcode does not use, so a word that re-encodes differently is exactly
        // a word with bits in unused positions.
        if insn.encode() != word {
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
            // (`docs/isa.md` §6.7). NaN fails `is_finite`; -0.0 is zero, not
            // negative, and is accepted.
            OpCode::Wait => {
                let seconds = insn.imm_f32();
                if !seconds.is_finite() || seconds < 0.0 {
                    return Err(LoadError::WaitNotFiniteOrNegative { index, seconds });
                }
                if seconds > image.header.max_wait {
                    return Err(LoadError::WaitExceedsMaxWait {
                        index,
                        seconds,
                        max: image.header.max_wait,
                    });
                }
            }

            _ => {}
        }

        code.push(insn);
    }

    Ok(code)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// A position in a byte string, bounded to one section.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(LoadError::Truncated {
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

/// Read one name and enforce M1 on it.
///
/// §6.1 tightens names to ASCII identifiers in v1 — a deliberate choice, because
/// binding compares names byte for byte and arbitrary UTF-8 would admit
/// look-alikes that bind differently while reading the same.
fn read_name<'a>(
    cursor: &mut Cursor<'a>,
    table: Table,
    index: usize,
) -> Result<&'a str, LoadError> {
    let bytes = cursor.cstr(table, index)?;

    if bytes.is_empty() {
        return Err(LoadError::EmptyName { table, index });
    }

    let valid = matches!(bytes[0], b'A'..=b'Z' | b'a'..=b'z' | b'_')
        && bytes[1..]
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'_');

    if !valid {
        return Err(LoadError::NameNotAnIdentifier {
            table,
            index,
            name: String::from_utf8_lossy(bytes).into_owned(),
        });
    }

    // Every byte was just checked to be ASCII, so this cannot fail.
    Ok(std::str::from_utf8(bytes).expect("ASCII is valid UTF-8"))
}

/// Read `count` bare names, enforcing M1 and M2.
fn read_names(cursor: &mut Cursor<'_>, table: Table, count: u32) -> Result<Vec<Muscle>, LoadError> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(count as usize);
    let mut out = Vec::with_capacity(count as usize);

    for index in 0..count as usize {
        let name = read_name(cursor, table, index)?;
        if !seen.insert(name) {
            return Err(LoadError::DuplicateName {
                table,
                index,
                name: name.to_string(),
            }); // M2
        }
        out.push(Muscle { name: name.into() });
    }

    Ok(out)
}

/// Read `count` `(name, src)` pairs, enforcing M1, M2 and the unknown-`src` half
/// of M5.
fn read_sensors(
    cursor: &mut Cursor<'_>,
    table: Table,
    count: u32,
) -> Result<Vec<Sensor>, LoadError> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(count as usize);
    let mut out = Vec::with_capacity(count as usize);

    for index in 0..count as usize {
        let name = read_name(cursor, table, index)?;
        if !seen.insert(name) {
            return Err(LoadError::DuplicateName {
                table,
                index,
                name: name.to_string(),
            });
        }

        let src = cursor.take(1)?[0];
        let source = SensorSource::from_u8(src).ok_or(LoadError::UnknownSensorSource {
            table,
            index,
            src,
        })?;

        out.push(Sensor {
            name: name.into(),
            source,
        });
    }

    Ok(out)
}

fn read_vars_f(cursor: &mut Cursor<'_>, count: u32) -> Result<Vec<VarF>, LoadError> {
    let table = Table::VarsF;
    let mut seen: HashSet<&str> = HashSet::with_capacity(count as usize);
    let mut out = Vec::with_capacity(count as usize);

    for index in 0..count as usize {
        let name = read_name(cursor, table, index)?;
        if !seen.insert(name) {
            return Err(LoadError::DuplicateName {
                table,
                index,
                name: name.to_string(),
            });
        }
        // §6.4: the initial value is a bit pattern, so NaN and both zeros survive.
        let init = f32::from_bits(cursor.u32_le()?);
        out.push(VarF {
            name: name.into(),
            init,
        });
    }

    Ok(out)
}

fn read_vars_i(cursor: &mut Cursor<'_>, count: u32) -> Result<Vec<VarI>, LoadError> {
    let table = Table::VarsI;
    let mut seen: HashSet<&str> = HashSet::with_capacity(count as usize);
    let mut out = Vec::with_capacity(count as usize);

    for index in 0..count as usize {
        let name = read_name(cursor, table, index)?;
        if !seen.insert(name) {
            return Err(LoadError::DuplicateName {
                table,
                index,
                name: name.to_string(),
            });
        }
        let init = cursor.u32_le()? as i32;
        out.push(VarI {
            name: name.into(),
            init,
        });
    }

    Ok(out)
}
