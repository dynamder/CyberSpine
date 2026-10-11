//! Loader tests: the positive round trips, and one negative case per checklist
//! line in `docs/binfmt.md` §9.
//!
//! **Why one test per rule.** "It was rejected" is not the property being
//! claimed. The claim is that *this* rule rejected it, so every negative test
//! asserts a specific error variant. Anything weaker would pass just as well
//! against a loader that rejected everything.
//!
//! **Why `forge`.** §8 makes the canonical writer the only source of a valid
//! file, so a file that violates the layout has no producer — no `Image` can
//! express it. The negative tests therefore patch the bytes and *repair the
//! hash*, exactly as a forger would. Without the repair every one of them would
//! be measuring C6 instead of its own rule.

use std::fs;
use std::path::PathBuf;

use cyber_spine::binfmt::image::{
    HASH_LEN, HEADER_LEN, INSN_LEN, Image, MAX_CYCLES_V1, Muscle, Sensor, SensorSource, VarF, VarI,
};
use cyber_spine::binfmt::load::{LoadError, MAX_FILE_LEN, Table};
use cyber_spine::register::RegId;
use cyber_spine::vm::op::{Insn, OpCode};

// ---------------------------------------------------------------------------
// Fixed-header offsets (§4), transcribed independently of the writer
// ---------------------------------------------------------------------------

const OFF_MAGIC: usize = 0;
const OFF_FORMAT_VERSION: usize = 4;
const OFF_ISA_VERSION: usize = 6;
const OFF_CYCLE_TABLE_VERSION: usize = 8;
const OFF_FLAGS: usize = 10;
const OFF_FILE_LEN: usize = 12;
const OFF_CODE_LEN: usize = 16;
const OFF_RESERVED0: usize = 22;
const OFF_MUSCLE_COUNT: usize = 24;
const OFF_SENSOR_F_COUNT: usize = 28;
const OFF_SENSOR_I_COUNT: usize = 32;
const OFF_VAR_F_COUNT: usize = 36;
const OFF_VAR_I_COUNT: usize = 40;
const OFF_HZ: usize = 44;
const OFF_CYCLES_PER_TICK: usize = 48;
const OFF_FUEL: usize = 52;
const OFF_MAX_WAIT: usize = 56;
const OFF_RESERVED1: usize = 60;
const OFF_RESERVED2: usize = 64;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn image_with(ops: &[Insn]) -> Image {
    Image::new(ops.to_vec())
}

/// An instruction with only the fields the caller names; the rest are zero.
/// `encode` writes only the fields the opcode declares, so zeroed extra fields
/// are harmless.
fn insn(op: OpCode, rd: u8, ra: u8, imm: u32) -> Insn {
    Insn {
        op,
        rd: RegId::new(rd).expect("rd < 64"),
        ra: RegId::new(ra).expect("ra < 64"),
        rb: RegId::ZERO,
        rc: RegId::ZERO,
        imm,
    }
}

/// Assemble a raw instruction word, re-deriving the layout (`isa.md` §1.1).
fn raw_word(op: u8, rd: u64, ra: u64, rb: u64, rc: u64, imm: u32) -> u64 {
    ((op as u64) << 56) | (rd << 50) | (ra << 44) | (rb << 38) | (rc << 32) | imm as u64
}

/// Patch bytes, then repair the trailing hash.
///
/// Repairing is the point: it leaves the file internally consistent except for
/// the single defect the test introduced, so the rule under test is the one that
/// fires.
fn forge(image: &Image, patches: &[(usize, &[u8])]) -> Vec<u8> {
    let mut bytes = image.to_bytes();
    for (offset, value) in patches {
        bytes[*offset..*offset + value.len()].copy_from_slice(value);
    }
    let end = bytes.len() - HASH_LEN;
    let digest = blake3::hash(&bytes[..end]);
    bytes[end..].copy_from_slice(digest.as_bytes());
    bytes
}

fn reject(bytes: &[u8]) -> LoadError {
    match Image::from_bytes(bytes) {
        Ok(image) => panic!(
            "expected rejection, but the loader accepted a {} instruction image",
            image.code.len()
        ),
        Err(err) => err,
    }
}

// ===========================================================================
// Positive: the round trip
// ===========================================================================

#[test]
fn a_minimal_image_round_trips() {
    let image = Image::halt_only();
    let loaded = Image::from_bytes(&image.to_bytes()).expect("the minimum legal file");
    assert_eq!(loaded, image);
}

#[test]
fn a_populated_image_round_trips() {
    let mut image = image_with(&[
        insn(OpCode::Fexcite, 0, 3, 0),
        insn(OpCode::Fsense, 4, 0, 0),
        insn(OpCode::Yield, 0, 0, 0),
        insn(OpCode::Halt, 0, 0, 0),
    ]);
    image.push_muscle("mouth_open");
    image.sensors_f.push(Sensor { name: "dt".into(), source: SensorSource::BuiltinDt });
    image.sensors_f.push(Sensor { name: "pressure".into(), source: SensorSource::Backend });
    image.sensors_i.push(Sensor { name: "tick".into(), source: SensorSource::BuiltinTick });
    image.sensors_i.push(Sensor { name: "errors".into(), source: SensorSource::Backend });
    image.vars_f.push(VarF { name: "gain".into(), init: 0.5 });
    image.vars_i.push(VarI { name: "count".into(), init: -7 });
    image.header.layer = 3;
    image.header.fuel = 1234;

    let loaded = Image::from_bytes(&image.to_bytes()).expect("a well-formed image");
    assert_eq!(loaded, image);
}

#[test]
fn float_var_initial_values_round_trip_bit_for_bit() {
    // §6.4: the initial value is stored as a bit pattern, so NaN payloads and
    // both signed zeros have to survive. `f32` equality cannot express that,
    // which is exactly why this compares bits.
    let cases: [u32; 5] = [0x0000_0000, 0x8000_0000, 0x7FC0_0001, 0xFFC0_0001, 0x0000_0001];

    let mut image = Image::halt_only();
    for (i, bits) in cases.iter().enumerate() {
        image.vars_f.push(VarF {
            name: format!("v{i}").into(),
            init: f32::from_bits(*bits),
        });
    }

    let loaded = Image::from_bytes(&image.to_bytes()).expect("a well-formed image");
    let bits: Vec<u32> = loaded.vars_f.iter().map(|v| v.init.to_bits()).collect();
    assert_eq!(bits, cases.to_vec());
}

#[test]
fn an_image_survives_a_trip_through_the_filesystem() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("create the test dir");
    let path = dir.join("round_trip.csp");

    let mut image = image_with(&[insn(OpCode::Yield, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    image.push_muscle("jaw");
    fs::write(&path, image.to_bytes()).expect("write the image");

    let loaded = Image::from_path(&path).expect("read the image back");
    assert_eq!(loaded, image);
}

#[test]
fn the_content_hash_matches_the_bytes_it_came_from() {
    let image = Image::halt_only();
    let bytes = image.to_bytes();
    assert_eq!(
        Image::content_hash(&bytes),
        Some(blake3::hash(&bytes[..bytes.len() - HASH_LEN]))
    );
}

// ===========================================================================
// §9.1 container
// ===========================================================================

#[test]
fn c1_a_file_shorter_than_the_minimum_is_refused() {
    assert!(matches!(reject(&[0u8; 100]), LoadError::TooShort { len: 100 }));
    assert!(matches!(reject(&[]), LoadError::TooShort { len: 0 }));
}

#[test]
fn c1_a_bad_magic_is_refused() {
    let image = Image::halt_only();
    let bytes = forge(&image, &[(OFF_MAGIC, b"XSPC")]);
    assert!(matches!(reject(&bytes), LoadError::BadMagic { found } if &found == b"XSPC"));
}

#[test]
fn c1_a_declared_length_that_disagrees_with_the_file_is_refused() {
    let image = Image::halt_only();
    let bytes = forge(&image, &[(OFF_FILE_LEN, &(136u32 + 1).to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::FileLenMismatch { declared: 137, actual: 136 }
    ));
}

#[test]
fn c2_an_unsupported_format_version_is_refused() {
    let image = Image::halt_only();

    // A different major is a different layout, so it must not be guessed at.
    let wrong_major = forge(&image, &[(OFF_FORMAT_VERSION, &0x0200u16.to_le_bytes())]);
    assert!(matches!(
        reject(&wrong_major),
        LoadError::UnsupportedFormatVersion { declared: 0x0200 }
    ));

    // A minor above this loader's is also refused: it may carry tables this
    // loader does not know to expect.
    let wrong_minor = forge(&image, &[(OFF_FORMAT_VERSION, &0x0001u16.to_le_bytes())]);
    assert!(matches!(
        reject(&wrong_minor),
        LoadError::UnsupportedFormatVersion { declared: 0x0001 }
    ));
}

#[test]
fn c2_unsupported_isa_and_cycle_table_versions_are_refused() {
    let image = Image::halt_only();

    let isa = forge(&image, &[(OFF_ISA_VERSION, &2u16.to_le_bytes())]);
    assert!(matches!(reject(&isa), LoadError::UnsupportedIsaVersion { found: 2 }));

    let cycles = forge(&image, &[(OFF_CYCLE_TABLE_VERSION, &2u16.to_le_bytes())]);
    assert!(matches!(
        reject(&cycles),
        LoadError::UnsupportedCycleTableVersion { found: 2 }
    ));
}

#[test]
fn c3_every_reserved_field_must_be_zero() {
    // "Undefined bits are refused, not ignored" — each of the four reserved
    // regions is checked, because each is a place a future version could put
    // something this loader would otherwise misread.
    let image = Image::halt_only();

    for (offset, field, width) in [
        (OFF_FLAGS, "flags", 2usize),
        (OFF_RESERVED0, "reserved0", 2),
        (OFF_RESERVED1, "reserved1", 4),
        (OFF_RESERVED2, "reserved2", 32),
    ] {
        let mut patch = vec![0u8; width];
        patch[0] = 1;
        let bytes = forge(&image, &[(offset, &patch)]);
        match reject(&bytes) {
            LoadError::ReservedNotZero { field: found } => {
                assert_eq!(found, field, "the wrong reserved field was reported")
            }
            other => panic!("{field} = 1 was not refused: {other}"),
        }
    }
}

#[test]
fn c4_hz_must_be_positive() {
    let image = Image::halt_only();
    let bytes = forge(&image, &[(OFF_HZ, &0u32.to_le_bytes())]);
    assert!(matches!(reject(&bytes), LoadError::ZeroHz));
}

#[test]
fn c4_max_wait_must_be_non_negative_and_finite() {
    let image = Image::halt_only();

    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        let bytes = forge(&image, &[(OFF_MAX_WAIT, &bad.to_bits().to_le_bytes())]);
        assert!(
            matches!(reject(&bytes), LoadError::InvalidMaxWait { .. }),
            "max_wait = {bad} was accepted"
        );
    }
}

#[test]
fn c4_the_tick_budget_must_cover_the_most_expensive_instruction() {
    let image = Image::halt_only();

    let bytes = forge(&image, &[(OFF_CYCLES_PER_TICK, &(MAX_CYCLES_V1 - 1).to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::CyclesPerTickTooSmall { declared, minimum }
            if declared == MAX_CYCLES_V1 - 1 && minimum == MAX_CYCLES_V1
    ));

    // Exactly the maximum is legal: the budget then holds one FSIN, barely.
    let exactly = forge(&image, &[(OFF_CYCLES_PER_TICK, &MAX_CYCLES_V1.to_le_bytes())]);
    assert!(Image::from_bytes(&exactly).is_ok());
}

#[test]
fn c5_a_count_that_cannot_fit_the_file_is_refused_before_allocating() {
    // The allocation bomb: a 136-byte file claiming four billion instructions.
    // 8 x 4e9 is 32 GB, so this must be caught by arithmetic on the declared
    // counts, before any `Vec::with_capacity`.
    let image = Image::halt_only();
    let bytes = forge(&image, &[(OFF_CODE_LEN, &u32::MAX.to_le_bytes())]);

    match reject(&bytes) {
        LoadError::CountsExceedFileLen { lower_bound, file_len } => {
            assert_eq!(lower_bound, 96 + 8 * u32::MAX as u64 + 32);
            assert_eq!(file_len, 136);
        }
        other => panic!("a 32 GB declared code section was not refused by C5: {other}"),
    }
}

#[test]
fn c5_is_computed_in_u64_so_the_bound_does_not_wrap() {
    // Every count at its maximum: the sum overflows u32 several times over. If
    // the lower bound were computed in u32 this file would look affordable.
    let image = Image::halt_only();
    let max = u32::MAX.to_le_bytes();
    let bytes = forge(
        &image,
        &[
            (OFF_CODE_LEN, &max),
            (OFF_MUSCLE_COUNT, &max),
            (OFF_SENSOR_F_COUNT, &max),
            (OFF_SENSOR_I_COUNT, &max),
            (OFF_VAR_F_COUNT, &max),
            (OFF_VAR_I_COUNT, &max),
        ],
    );

    match reject(&bytes) {
        LoadError::CountsExceedFileLen { lower_bound, .. } => {
            let entries = 5 * u32::MAX as u64;
            assert_eq!(lower_bound, 96 + 8 * u32::MAX as u64 + 32 + 2 * entries);
        }
        other => panic!("the wrapped bound was accepted: {other}"),
    }
}

#[test]
fn c6_a_corrupt_file_is_refused_by_the_hash() {
    // Flip one bit and leave the hash alone: corruption, not forgery.
    let image = Image::halt_only();
    let mut bytes = image.to_bytes();
    bytes[OFF_FUEL] ^= 0x01;

    assert!(matches!(
        reject(&bytes),
        LoadError::ContentHashMismatch { .. }
    ));
}

#[test]
fn c7_bytes_between_the_tables_and_the_hash_are_refused() {
    // Two instructions declared as one: the second word is neither code nor
    // table, and the file must not be accepted with it sitting there unread.
    let image = image_with(&[insn(OpCode::Nop, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    let bytes = forge(&image, &[(OFF_CODE_LEN, &1u32.to_le_bytes())]);

    assert!(matches!(
        reject(&bytes),
        LoadError::TrailingBytes { consumed: 104, expected: 112 }
    ));
}

#[test]
fn c8_code_len_zero_is_refused() {
    let image = Image::halt_only();
    let bytes = forge(&image, &[(OFF_CODE_LEN, &0u32.to_le_bytes())]);
    assert!(matches!(reject(&bytes), LoadError::CodeLenZero));
}

#[test]
fn c8_the_last_instruction_must_be_halt() {
    // Also I8, which is the same rule stated at instruction level.
    let image = image_with(&[insn(OpCode::Nop, 0, 0, 0)]);
    assert!(matches!(reject(&image.to_bytes()), LoadError::MissingTrailingHalt));

    let image = image_with(&[insn(OpCode::Yield, 0, 0, 0)]);
    assert!(matches!(reject(&image.to_bytes()), LoadError::MissingTrailingHalt));
}

// ===========================================================================
// §9.2 metadata
// ===========================================================================

#[test]
fn m1_an_empty_name_is_refused() {
    // Note the file has to carry slack for C5 to pass first. Its lower bound
    // assumes every metadata entry is at least 2 bytes, which is exactly right
    // for a *valid* file (§6.1 makes empty names illegal) but one byte too many
    // for an empty one. A longer valid name supplies that slack; without it C5
    // fires and this test would be measuring the wrong rule.
    let mut image = Image::halt_only();
    image.push_muscle("a_reasonably_long_name");
    image.push_muscle("");

    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::EmptyName { table: Table::Muscles, index: 1 }
    ));
}

#[test]
fn m1_a_name_that_is_not_an_ascii_identifier_is_refused() {
    // §6.1's v1 narrowing. Binding compares names byte for byte, so anything
    // that can look identical while differing is a hazard worth excluding.
    for bad in ["9lives", "with-dash", "with space", "café", "a.b"] {
        let mut image = Image::halt_only();
        image.muscles.push(Muscle { name: bad.into() });
        match reject(&image.to_bytes()) {
            LoadError::NameNotAnIdentifier { name, .. } => assert_eq!(name, bad),
            other => panic!("{bad:?} was not refused as a non-identifier: {other}"),
        }
    }

    // The legal edge of the same rule: `_` is a valid identifier on its own, and
    // digits are fine after the first character.
    let mut image = Image::halt_only();
    image.push_muscle("_");
    image.push_muscle("_9x");
    assert!(Image::from_bytes(&image.to_bytes()).is_ok(), "'_' and '_9x' are valid identifiers");
}

#[test]
fn m2_a_repeated_name_within_one_table_is_refused() {
    // Ambiguity at bind time: two muscles called "jaw" cannot be told apart.
    let mut image = Image::halt_only();
    image.push_muscle("jaw");
    image.push_muscle("jaw");

    match reject(&image.to_bytes()) {
        LoadError::DuplicateName { table, index: 1, name } => {
            assert_eq!(table, Table::Muscles);
            assert_eq!(name, "jaw");
        }
        other => panic!("a duplicate muscle name was not refused: {other}"),
    }
}

#[test]
fn m2_names_are_only_unique_within_a_table() {
    // The five tables are five separate namespaces (§6.5), so the same name may
    // appear in two of them — and does, in the read-back invariant.
    let mut image = Image::halt_only();
    image.push_muscle("shared");
    image.sensors_i.push(Sensor { name: "shared".into(), source: SensorSource::Backend });
    image.vars_f.push(VarF { name: "shared".into(), init: 0.0 });
    image.vars_i.push(VarI { name: "shared".into(), init: 0 });
    assert!(Image::from_bytes(&image.to_bytes()).is_ok());
}

#[test]
fn m3_sensor_f_count_must_cover_every_muscle() {
    // One muscle, no read-back sensor: the invariant in §6.2 has nothing to
    // point at.
    let mut image = Image::halt_only();
    image.muscles.push(Muscle { name: "jaw".into() });

    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::SensorFCountBelowMuscleCount { muscle_count: 1, sensor_f_count: 0 }
    ));
}

#[test]
fn m4_a_readback_sensor_must_carry_the_same_name() {
    let mut image = Image::halt_only();
    image.muscles.push(Muscle { name: "jaw".into() });
    image.sensors_f.push(Sensor { name: "jaw_".into(), source: SensorSource::Muscle });

    match reject(&image.to_bytes()) {
        LoadError::ReadbackNameMismatch { index: 0, muscle, sensor } => {
            assert_eq!(muscle, "jaw");
            assert_eq!(sensor, "jaw_");
        }
        other => panic!("a misnamed read-back sensor was not refused: {other}"),
    }
}

#[test]
fn m4_a_readback_sensor_must_be_sourced_from_the_muscle() {
    // Same name, wrong channel: FSENSE and FEXCITE would then disagree about
    // which integer means "jaw".
    let mut image = Image::halt_only();
    image.muscles.push(Muscle { name: "jaw".into() });
    image.sensors_f.push(Sensor { name: "jaw".into(), source: SensorSource::Backend });

    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::ReadbackSourceMismatch { index: 0, found: SensorSource::Backend }
    ));
}

#[test]
fn m5_an_undefined_source_byte_is_refused() {
    let mut image = Image::halt_only();
    image.push_muscle("a");
    let mut bytes = image.to_bytes();
    // sensors_f[0]'s src byte: after the header, the code, and both names.
    let src = HEADER_LEN + INSN_LEN + 2 + 2;
    bytes[src] = 9;
    let end = bytes.len() - HASH_LEN;
    let digest = blake3::hash(&bytes[..end]);
    bytes[end..].copy_from_slice(digest.as_bytes());

    assert!(matches!(
        reject(&bytes),
        LoadError::UnknownSensorSource { table: Table::SensorsF, index: 0, src: 9 }
    ));
}

#[test]
fn m5_builtin_dt_and_tick_are_single_and_table_specific() {
    // BUILTIN_TICK in sensors_f: the integer tick cannot be read as a float.
    let mut image = Image::halt_only();
    image.sensors_f.push(Sensor { name: "t".into(), source: SensorSource::BuiltinTick });
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::SensorSourceNotAllowedHere { table: Table::SensorsF, index: 0, .. }
    ));

    // BUILTIN_DT in sensors_i: the same rule the other way.
    let mut image = Image::halt_only();
    image.sensors_i.push(Sensor { name: "d".into(), source: SensorSource::BuiltinDt });
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::SensorSourceNotAllowedHere { table: Table::SensorsI, index: 0, .. }
    ));

    // A second dt.
    let mut image = Image::halt_only();
    image.sensors_f.push(Sensor { name: "d1".into(), source: SensorSource::BuiltinDt });
    image.sensors_f.push(Sensor { name: "d2".into(), source: SensorSource::BuiltinDt });
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::DuplicateBuiltinDt { index: 1 }
    ));

    // A second tick.
    let mut image = Image::halt_only();
    image.sensors_i.push(Sensor { name: "t1".into(), source: SensorSource::BuiltinTick });
    image.sensors_i.push(Sensor { name: "t2".into(), source: SensorSource::BuiltinTick });
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::DuplicateBuiltinTick { index: 1 }
    ));
}

#[test]
fn m6_muscle_readback_is_only_legal_where_a_muscle_exists() {
    // One muscle but two MUSCLE sensors: the second has no muscle to read.
    let mut image = Image::halt_only();
    image.push_muscle("a");
    image.sensors_f.push(Sensor { name: "b".into(), source: SensorSource::Muscle });

    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::MuscleSourceNotAllowedHere { table: Table::SensorsF, index: 1 }
    ));
}

// ===========================================================================
// §9.3 instructions
// ===========================================================================

#[test]
fn i1_a_reserved_opcode_is_refused() {
    // No `Image` can produce a reserved opcode (§8 canonical form), so patch one
    // into the code section and repair the hash, exactly as a forger would.
    let image = image_with(&[insn(OpCode::Nop, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    let reserved = raw_word(0x04, 0, 0, 0, 0, 0);
    let bytes = forge(&image, &[(HEADER_LEN, &reserved.to_le_bytes())]);
    match reject(&bytes) {
        LoadError::InvalidInstruction { index: 0, source } => {
            assert_eq!(source.to_string(), "Invalid Operation Code 4");
        }
        other => panic!("a reserved opcode was not refused by I1: {other}"),
    }
}

#[test]
fn i2_a_word_with_unused_bits_set_is_refused() {
    // NOP takes no operand, so imm must be zero. This is the equation that makes
    // the on-disk encoding canonical (§8): a hand-patched word is not.
    // The writer is canonical by construction, so a word with dirty bits has no
    // producer: patch it in and repair the hash.
    let image = image_with(&[insn(OpCode::Nop, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    let dirty = raw_word(0xA1, 0, 0, 0, 0, 1); // NOP with imm = 1
    let bytes = forge(&image, &[(HEADER_LEN, &dirty.to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::NonCanonicalInstruction { index: 0, .. }
    ));

    // A register field an instruction does not use is the other half of I2.
    let dirty = raw_word(0xA0, 0, 0, 0, 7, 0); // HALT with rc = 7
    let bytes = forge(&image, &[(HEADER_LEN, &dirty.to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::NonCanonicalInstruction { index: 0, .. }
    ));
}

#[test]
fn i3_sensor_indices_must_be_in_range() {
    let image = image_with(&[insn(OpCode::Fsense, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::SensorIndexOutOfRange { index: 0, table: Table::SensorsF, sensor_id: 0, count: 0 }
    ));

    let image = image_with(&[insn(OpCode::Isense, 0, 0, 3), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::SensorIndexOutOfRange { index: 0, table: Table::SensorsI, sensor_id: 3, count: 0 }
    ));
}

#[test]
fn i4_variable_indices_must_be_in_range() {
    let image = image_with(&[insn(OpCode::Fvget, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::VarIndexOutOfRange { index: 0, table: Table::VarsF, var_id: 0, count: 0 }
    ));

    let image = image_with(&[insn(OpCode::Ivset, 0, 0, 2), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::VarIndexOutOfRange { index: 0, table: Table::VarsI, var_id: 2, count: 0 }
    ));
}

#[test]
fn i5_muscle_indices_must_be_in_range() {
    let image = image_with(&[insn(OpCode::Fexcite, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::MuscleIndexOutOfRange { index: 0, muscle_id: 0, count: 0 }
    ));
}

#[test]
fn i6_a_jump_past_the_end_is_refused() {
    let image = image_with(&[insn(OpCode::Jmp, 0, 0, 100), insn(OpCode::Halt, 0, 0, 0)]);
    match reject(&image.to_bytes()) {
        LoadError::JumpTargetOutOfRange { index: 0, offset: 100, code_len: 2 } => {}
        other => panic!("a jump past the end was not refused by I6: {other}"),
    }
}

#[test]
fn i6_a_jump_before_the_start_is_refused() {
    // pc 0 with off -5 underflows index 0; `jump_target` returns None and I6
    // reports it with the same variant as a forward overshoot.
    let image = image_with(&[insn(OpCode::Jmp, 0, 0, (-5i32) as u32), insn(OpCode::Halt, 0, 0, 0)]);
    match reject(&image.to_bytes()) {
        LoadError::JumpTargetOutOfRange { index: 0, offset: -5, .. } => {}
        other => panic!("a jump before index 0 was not refused by I6: {other}"),
    }
}

#[test]
fn i6_a_jump_to_the_last_instruction_is_legal() {
    // `target < code_len` — "jump to the end" means the trailing HALT, which is
    // reachable. The boundary must be inclusive on the inside.
    let image = image_with(&[insn(OpCode::Jmp, 0, 0, 0), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(Image::from_bytes(&image.to_bytes()).is_ok());

    let image = image_with(&[insn(OpCode::Jmp, 0, 0, 1), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::JumpTargetOutOfRange { .. }
    ));
}

#[test]
fn i7_wait_must_be_a_non_negative_finite_duration() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5] {
        let image = image_with(&[insn(OpCode::Wait, 0, 0, bad.to_bits()), insn(OpCode::Halt, 0, 0, 0)]);
        assert!(
            matches!(reject(&image.to_bytes()), LoadError::WaitNotFiniteOrNegative { index: 0, .. }),
            "WAIT {bad} was accepted"
        );
    }

    // -0.0 is zero, not a negative duration, and is legal.
    let image = image_with(&[insn(OpCode::Wait, 0, 0, (-0.0f32).to_bits()), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(Image::from_bytes(&image.to_bytes()).is_ok());
}

#[test]
fn i7_wait_must_not_exceed_the_headers_max_wait() {
    // WAIT is unbounded by fuel (isa.md §6.7), so the header bounds it instead.
    let image = image_with(&[insn(OpCode::Wait, 0, 0, 601.0f32.to_bits()), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(matches!(
        reject(&image.to_bytes()),
        LoadError::WaitExceedsMaxWait { index: 0, seconds: 601.0, max: 600.0 }
    ));

    // Exactly the bound is legal.
    let image = image_with(&[insn(OpCode::Wait, 0, 0, 600.0f32.to_bits()), insn(OpCode::Halt, 0, 0, 0)]);
    assert!(Image::from_bytes(&image.to_bytes()).is_ok());
}

// ===========================================================================
// The disk entry point
// ===========================================================================

#[test]
fn a_missing_file_is_an_io_error() {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("does_not_exist.csp");
    assert!(matches!(Image::from_path(&path), Err(LoadError::Io { .. })));
}

#[test]
fn an_oversized_file_is_refused_by_the_metadata_gate() {
    // A sparse file: the length is real to the filesystem, the bytes are not.
    // C5 cannot help here, because the allocation would happen in the read
    // itself — this gate is what stops it.
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("oversized.csp");
    let file = fs::File::create(&path).expect("create");
    file.set_len(MAX_FILE_LEN as u64 + 1).expect("size the sparse file");
    drop(file);

    assert!(matches!(
        Image::from_path(&path),
        Err(LoadError::FileTooLarge { max: MAX_FILE_LEN, .. })
    ));

    let _ = fs::remove_file(&path);
}

#[test]
fn a_truncated_file_on_disk_is_refused() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(&dir).expect("create the test dir");
    let path = dir.join("truncated.csp");
    fs::write(&path, [0u8; 100]).expect("write");

    assert!(matches!(
        Image::from_path(&path),
        Err(LoadError::TooShort { len: 100 })
    ));
}
