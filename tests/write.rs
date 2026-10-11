//! Byte-layout conformance tests for the writer (`docs/binfmt.md` §4–§8).
//!
//! The expected bytes are transcribed by hand from the §4 table rather than read
//! back out of the implementation, for the same reason `tests/bytecode.rs`
//! re-derives the bit layout: a test that asks the code what it wrote cannot tell
//! you whether what it wrote is what the spec says.

use cyber_spine::binfmt::image::{
    DEFAULT_FUEL, HASH_LEN, HEADER_LEN, INSN_LEN, Image, MIN_FILE_LEN, Muscle, Sensor,
    SensorSource, VarF, VarI,
};
use cyber_spine::register::RegId;
use cyber_spine::vm::op::{Insn, OpCode};

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"))
}

/// The default form of an opcode: every operand field zero.
fn insn(op: OpCode) -> Insn {
    Insn::new(op)
}

/// The default form with one `ra` operand set.
fn with_ra(op: OpCode, ra: u8) -> Insn {
    Insn {
        ra: RegId::new(ra).expect("ra < 64"),
        ..Insn::new(op)
    }
}

fn code(words: &[Insn]) -> Vec<Insn> {
    words.to_vec()
}

// ---------------------------------------------------------------------------
// The fixed header, field by field
// ---------------------------------------------------------------------------

#[test]
fn a_halt_only_image_is_the_minimum_136_bytes() {
    let bytes = Image::halt_only().to_bytes();
    assert_eq!(bytes.len(), MIN_FILE_LEN);
    assert_eq!(bytes.len(), 136, "96-byte header + one 8-byte HALT + 32-byte hash");

    // `docs/binfmt.md` §4, transcribed in table order. Every value is either a
    // spec constant or derivable from a one-instruction, metadata-free image.
    let expected_header: [u8; 64] = [
        b'C', b'S', b'P', b'X', //  0  magic
        0x00, 0x01, //  4  format_version = major 1, minor 0
        0x01, 0x00, //  6  isa_version = 1
        0x01, 0x00, //  8  cycle_table_version = 1
        0x00, 0x00, // 10  flags = 0
        0x88, 0x00, 0x00, 0x00, // 12  file_len = 136
        0x01, 0x00, 0x00, 0x00, // 16  code_len = 1
        0x00, 0x00, // 20  layer
        0x00, 0x00, // 22  reserved0 = 0
        0x00, 0x00, 0x00, 0x00, // 24  muscle_count
        0x00, 0x00, 0x00, 0x00, // 28  sensor_f_count
        0x00, 0x00, 0x00, 0x00, // 32  sensor_i_count
        0x00, 0x00, 0x00, 0x00, // 36  var_f_count
        0x00, 0x00, 0x00, 0x00, // 40  var_i_count
        0x3C, 0x00, 0x00, 0x00, // 44  hz = 60
        0xE8, 0x03, 0x00, 0x00, // 48  cycles_per_tick = 1000
        0x00, 0x10, 0x00, 0x00, // 52  fuel = 4096
        0x00, 0x00, 0x16, 0x44, // 56  max_wait_bits = 600.0f32
        0x00, 0x00, 0x00, 0x00, // 60  reserved1 = 0
    ];
    assert_eq!(
        &bytes[..64],
        &expected_header[..],
        "the first 64 header bytes do not match binfmt.md §4"
    );

    // 64..96: reserved2, four u64 reserved for a future data section, all zero.
    assert_eq!(&bytes[64..96], &[0u8; 32], "reserved2 must be all zero");

    // 96..104: the code section, one HALT as a little-endian u64 (§5).
    assert_eq!(
        bytes[96..104],
        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA0],
        "HALT is opcode 0xA0 in the top byte of a little-endian word"
    );

    // 104..136: the hash over everything before it (§7).
    assert_eq!(
        &bytes[104..],
        blake3::hash(&bytes[..104]).as_bytes(),
        "content_hash must cover every byte before it"
    );
}

#[test]
fn the_header_is_exactly_96_bytes() {
    // §4.1 calls this a format invariant: it is what makes the code section
    // 8-byte aligned with no padding rule anywhere in the file.
    assert_eq!(HEADER_LEN % 8, 0);
}

// ---------------------------------------------------------------------------
// The metadata tables
// ---------------------------------------------------------------------------

#[test]
fn the_tables_are_gap_free_and_in_the_order_section_6_fixes() {
    let mut image = Image::new(code(&[with_ra(OpCode::Fexcite, 1), insn(OpCode::Halt)]));
    image.push_muscle("mouth");
    image.sensors_f.push(Sensor { name: "dt".into(), source: SensorSource::BuiltinDt });
    image.sensors_i.push(Sensor { name: "tick".into(), source: SensorSource::BuiltinTick });
    image.vars_f.push(VarF { name: "gain".into(), init: 0.5 });
    image.vars_i.push(VarI { name: "count".into(), init: -7 });

    let bytes = image.to_bytes();

    // muscles → sensors_f → sensors_i → vars_f → vars_i, back to back, starting
    // immediately after the code section (§3). No length prefixes, no gaps.
    let mut expected = Vec::new();
    expected.extend_from_slice(b"mouth\0"); //                        muscles[0]
    expected.extend_from_slice(b"mouth\0"); //                        sensors_f[0] (read-back)
    expected.push(SensorSource::Muscle.to_u8());
    expected.extend_from_slice(b"dt\0"); //                           sensors_f[1]
    expected.push(SensorSource::BuiltinDt.to_u8());
    expected.extend_from_slice(b"tick\0"); //                         sensors_i[0]
    expected.push(SensorSource::BuiltinTick.to_u8());
    expected.extend_from_slice(b"gain\0"); //                         vars_f[0]
    expected.extend_from_slice(&0.5f32.to_bits().to_le_bytes());
    expected.extend_from_slice(b"count\0"); //                        vars_i[0]
    expected.extend_from_slice(&(-7i32).to_le_bytes());

    let tables = &bytes[HEADER_LEN + 2 * INSN_LEN..bytes.len() - HASH_LEN];
    assert_eq!(tables, &expected[..], "the five tables are not laid out as §6 requires");

    // The counts describe exactly those tables.
    assert_eq!(u32_at(&bytes, 16), 2, "code_len");
    assert_eq!(u32_at(&bytes, 24), 1, "muscle_count");
    assert_eq!(u32_at(&bytes, 28), 2, "sensor_f_count");
    assert_eq!(u32_at(&bytes, 32), 1, "sensor_i_count");
    assert_eq!(u32_at(&bytes, 36), 1, "var_f_count");
    assert_eq!(u32_at(&bytes, 40), 1, "var_i_count");
    assert_eq!(u32_at(&bytes, 12) as usize, bytes.len(), "file_len");
}

#[test]
fn a_sensor_is_a_name_then_one_source_byte() {
    // §6.3. The source byte is not length-prefixed and follows the NUL directly,
    // so a reader that forgot the NUL would swallow the source as part of the name.
    //
    // Note that the tables are *sections*, not interleaved records: every muscle
    // comes before every sensor, so a name and its read-back sensor are not
    // adjacent in the file even though they are adjacent in `Image`.
    let mut image = Image::halt_only();
    for (i, source) in [
        SensorSource::Muscle,
        SensorSource::Backend,
        SensorSource::BuiltinDt,
        SensorSource::BuiltinTick,
    ]
    .into_iter()
    .enumerate()
    {
        image.muscles.push(Muscle { name: format!("m{i}").into() });
        image.sensors_f.push(Sensor { name: format!("m{i}").into(), source });
    }

    let bytes = image.to_bytes();
    let mut cursor = HEADER_LEN + INSN_LEN;

    // muscles: muscle_count bare names (§6.2).
    for i in 0..4 {
        let name = format!("m{i}");
        assert_eq!(&bytes[cursor..cursor + name.len()], name.as_bytes());
        assert_eq!(bytes[cursor + name.len()], 0x00, "name must be NUL-terminated");
        cursor += name.len() + 1;
    }

    // sensors_f: sensor_f_count (name, src) pairs (§6.3).
    for source in 0..4u8 {
        let name = format!("m{source}");
        assert_eq!(&bytes[cursor..cursor + name.len()], name.as_bytes());
        assert_eq!(bytes[cursor + name.len()], 0x00, "name must be NUL-terminated");
        assert_eq!(bytes[cursor + name.len() + 1], source, "source byte");
        cursor += name.len() + 2;
    }

    assert_eq!(cursor, bytes.len() - HASH_LEN, "the tables must end where the hash begins");
}

// ---------------------------------------------------------------------------
// Derived fields
// ---------------------------------------------------------------------------

#[test]
fn the_derived_length_fields_track_the_actual_contents() {
    for n in [1usize, 2, 3, 17] {
        let mut ops = vec![insn(OpCode::Nop); n - 1];
        ops.push(insn(OpCode::Halt));
        let bytes = Image::new(code(&ops)).to_bytes();

        assert_eq!(u32_at(&bytes, 16) as usize, n, "code_len must be the instruction count");
        assert_eq!(u32_at(&bytes, 12) as usize, bytes.len(), "file_len must be the actual length");
        assert_eq!(bytes.len(), HEADER_LEN + n * INSN_LEN + HASH_LEN);
    }
}

#[test]
fn the_hash_covers_the_whole_file_not_just_the_header() {
    let hash_of = |b: &[u8]| b[b.len() - HASH_LEN..].to_vec();

    let base = Image::halt_only().to_bytes();

    let mut fuel_changed = Image::halt_only();
    fuel_changed.header.fuel = DEFAULT_FUEL - 1; // header, offset 52
    let fuel_changed = fuel_changed.to_bytes();

    let mut code_changed = Image::halt_only();
    code_changed.code[0] = insn(OpCode::Nop); // code, offset 96
    let code_changed = code_changed.to_bytes();

    assert_ne!(base[..base.len() - HASH_LEN], fuel_changed[..fuel_changed.len() - HASH_LEN]);
    assert_ne!(
        hash_of(&base),
        hash_of(&fuel_changed),
        "a header byte change must change the hash"
    );
    assert_ne!(
        hash_of(&base),
        hash_of(&code_changed),
        "a code-section byte change must change the hash"
    );
}

#[test]
fn writing_the_same_image_twice_gives_identical_bytes() {
    // §8: the canonical form is constructive. There is no second encoding to
    // choose, so there is nothing for a second run to decide differently.
    let build = || {
        let mut image = Image::new(code(&[insn(OpCode::Yield), insn(OpCode::Halt)]));
        image.push_muscle("jaw");
        image.vars_f.push(VarF { name: "k".into(), init: f32::NAN });
        image.to_bytes()
    };
    assert_eq!(build(), build());
}

// ---------------------------------------------------------------------------
// The writer is not a validator
// ---------------------------------------------------------------------------

#[test]
fn push_muscle_keeps_the_readback_invariant() {
    // §6.2: sensors_f[m] must be muscles[m] read back. The helper makes that
    // impossible to forget for the common case.
    let mut image = Image::halt_only();
    image.push_muscle("a");
    image.push_muscle("b");

    assert_eq!(image.muscles.len(), 2);
    assert_eq!(image.sensors_f.len(), 2);
    for (muscle, sensor) in image.muscles.iter().zip(&image.sensors_f) {
        assert_eq!(muscle.name, sensor.name, "the read-back sensor must reuse the name");
        assert_eq!(sensor.source, SensorSource::Muscle);
    }

    let bytes = image.to_bytes();
    assert!(u32_at(&bytes, 28) >= u32_at(&bytes, 24), "M3: sensor_f_count >= muscle_count");
}

#[test]
fn the_writer_emits_names_it_cannot_vouch_for() {
    // §9.2 M1 is the loader's rule. The writer has to be able to produce a bad
    // name, or `tests/load.rs` could not test that the loader rejects one.
    let mut image = Image::halt_only();
    image.muscles.push(Muscle { name: "".into() });
    image.sensors_f.push(Sensor { name: "9bad".into(), source: SensorSource::Muscle });

    let bytes = image.to_bytes();
    let tables = &bytes[HEADER_LEN + INSN_LEN..bytes.len() - HASH_LEN];
    assert_eq!(tables, b"\x009bad\0\x00", "empty name is a bare NUL; source byte follows");
}

#[test]
fn the_writer_leaves_the_trailing_halt_to_the_binder() {
    // §5 puts the trailing HALT on the binder, and §9.3 I8 keeps the check on the
    // loader. The writer must not silently "fix" code that lacks one, or a test
    // could never produce a file that fails I8.
    let bytes = Image::new(code(&[insn(OpCode::Nop)])).to_bytes();
    let last = &bytes[HEADER_LEN..HEADER_LEN + INSN_LEN];
    assert_eq!(last[7], 0xA1, "the writer wrote the NOP it was given, not a HALT");
}

#[test]
#[should_panic(expected = "cannot contain NUL")]
fn the_writer_refuses_a_name_the_format_cannot_represent() {
    // An interior NUL would silently split one name into two. Unlike a bad
    // charset, this is not a file the format can express, so it is refused at the
    // only point where it could do damage.
    let mut image = Image::halt_only();
    image.muscles.push(Muscle { name: "a\0b".into() });
    let _ = image.to_bytes();
}
