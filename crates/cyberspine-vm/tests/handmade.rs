//! Loading tests against **hand-assembled** `.csp` files checked into the repo.
//!
//! Everything in `tests/load.rs` starts from `Image::to_bytes`, so it can only
//! ever prove the loader agrees with the writer. The fixtures here are built from
//! the offset table in `docs/binfmt.md` §4–§7 directly — no `Image`, no
//! `Insn::encode` — and committed as golden bytes. The tests load them from disk,
//! so a shared bug in the writer and loader cannot survive, because one side of
//! the comparison is a file that was written by hand.
//!
//! The valid golden is **full-coverage**: it contains every one of the 49 opcodes
//! exactly once, both read-back and backend/builtin sensors, all four immediate
//! interpretations, a backward jump, a non-zero `WAIT`, two muscles, and a
//! non-default clock and layer. It is 636 bytes:
//!
//! ```text
//! [0   .. 96)    fixed header
//! [96  .. 488)   49 instructions, one per opcode
//! [488 .. 604)   metadata: muscles, sensors_f, sensors_i, vars_f, vars_i
//! [604 .. 636)   content_hash = blake3(bytes[..604])
//! ```
//!
//! Regenerate the goldens deliberately with:
//!
//! ```text
//! cargo test -p cyberspine-vm --test handmade -- --ignored
//! ```

use std::fs;
use std::path::PathBuf;

use cyberspine_vm::binfmt::image::{HASH_LEN, HEADER_LEN, Image, SensorSource};
use cyberspine_vm::binfmt::load::{LoadError, Table};
use cyberspine_vm::vm::op::OpCode;

// ---------------------------------------------------------------------------
// The golden files
// ---------------------------------------------------------------------------

const VALID: &str = "valid_all.csp";
const INVALID_BAD_HASH: &str = "invalid_bad_hash.csp";
const INVALID_RESERVED: &str = "invalid_reserved_opcode.csp";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// A hand-written producer, used only to document and regenerate the goldens
// ---------------------------------------------------------------------------

/// The number of instructions in the valid golden: one per opcode.
const CODE_LEN: usize = 49;

/// Assemble one instruction word at the exact bit layout of `isa.md` §1.1:
/// `[63..56] op  [55..50] rd  [49..44] ra  [43..38] rb  [37..32] rc  [31..0] imm`.
const fn w(op: u64, rd: u64, ra: u64, rb: u64, rc: u64, imm: u32) -> u64 {
    (op << 56) | (rd << 50) | (ra << 44) | (rb << 38) | (rc << 32) | (imm as u64)
}

/// Every opcode exactly once, with operands chosen so the loader's I3–I7 range
/// checks all pass:
///
/// * `FSENSE imm=3` / `ISENSE imm=1` / `FEXCITE imm=1` / var `imm` values are all
///   in range for the tables below.
/// * `JMP -25` at pc 24 targets pc 1 — a backward jump inside the unit.
/// * conditional jumps use `off = 0`, which falls through to the next
///   instruction and is the one offset that cannot overshoot.
/// * `WAIT 0.5` is under the header's `max_wait = 12.5`.
const CODE: [u64; CODE_LEN] = [
    // 0x00 transfer and constants
    w(0x00, 1, 0, 0, 0, 0x3F80_0000), //  0  FMOVI  f1, 1.0
    w(0x01, 1, 0, 0, 0, 5),           //  1  IMOVI  i1, 5
    w(0x02, 2, 1, 0, 0, 0),           //  2  FMOV   f2, f1
    w(0x03, 2, 1, 0, 0, 0),           //  3  IMOV   i2, i1
    // 0x10 float binary
    w(0x10, 3, 1, 2, 0, 0),           //  4  FADD   f3, f1, f2
    w(0x11, 4, 3, 1, 0, 0),           //  5  FSUB   f4, f3, f1
    w(0x12, 5, 4, 2, 0, 0),           //  6  FMUL   f5, f4, f2
    w(0x13, 6, 5, 3, 0, 0),           //  7  FDIV   f6, f5, f3
    w(0x14, 7, 1, 2, 0, 0),           //  8  FMIN   f7, f1, f2
    w(0x15, 8, 1, 2, 0, 0),           //  9  FMAX   f8, f1, f2
    // 0x20 float unary
    w(0x20, 9, 1, 0, 0, 0),           // 10  FNEG   f9, f1
    w(0x21, 10, 9, 0, 0, 0),          // 11  FABS   f10, f9
    w(0x22, 11, 10, 0, 0, 0),         // 12  FSQRT  f11, f10
    w(0x23, 12, 1, 0, 0, 0),          // 13  FSIN   f12, f1
    w(0x24, 13, 1, 0, 0, 0),          // 14  FCOS   f13, f1
    w(0x25, 14, 1, 0, 0, 0),          // 15  FFRAC  f14, f1
    // 0x30 float ternary
    w(0x30, 15, 1, 2, 3, 0),          // 16  FMADD  f15, f1, f2, f3
    w(0x31, 16, 1, 2, 3, 0),          // 17  FLERP  f16, f1, f2, f3
    w(0x32, 17, 1, 2, 3, 0),          // 18  FCLAMP f17, f1, f2, f3
    // 0x40 integer and conversion
    w(0x40, 3, 1, 2, 0, 0),           // 19  IADD   i3, i1, i2
    w(0x41, 4, 3, 1, 0, 0),           // 20  ISUB   i4, i3, i1
    w(0x42, 5, 4, 2, 0, 0),           // 21  IMUL   i5, i4, i2
    w(0x43, 18, 1, 0, 0, 0),          // 22  ITOF   f18, i1   (rd f, ra i)
    w(0x44, 6, 1, 0, 0, 0),           // 23  FTOI   i6, f1    (rd i, ra f)
    // 0x50 control flow
    w(0x50, 0, 0, 0, 0, (-25i32) as u32), // 24  JMP    back to pc 1
    w(0x51, 0, 1, 0, 0, 0),           // 25  IJZ    i1, +0
    w(0x52, 0, 1, 0, 0, 0),           // 26  IJNZ   i1, +0
    w(0x53, 0, 1, 2, 0, 0),           // 27  FLT    f1, f2, +0
    w(0x54, 0, 1, 2, 0, 0),           // 28  FLE    f1, f2, +0
    w(0x55, 0, 1, 2, 0, 0),           // 29  FGT    f1, f2, +0
    w(0x56, 0, 1, 2, 0, 0),           // 30  FGE    f1, f2, +0
    w(0x57, 0, 1, 2, 0, 0),           // 31  ILT    i1, i2, +0
    w(0x58, 0, 1, 2, 0, 0),           // 32  ILE    i1, i2, +0
    w(0x59, 0, 1, 2, 0, 0),           // 33  IGT    i1, i2, +0
    w(0x5A, 0, 1, 2, 0, 0),           // 34  IGE    i1, i2, +0
    w(0x5B, 0, 1, 2, 0, 0),           // 35  IEQ    i1, i2, +0
    w(0x5C, 0, 1, 2, 0, 0),           // 36  INE    i1, i2, +0
    // 0x60 input
    w(0x60, 19, 0, 0, 0, 3),          // 37  FSENSE f19, sensor_f[3] = pressure
    w(0x61, 7, 0, 0, 0, 1),           // 38  ISENSE i7, sensor_i[1] = errors
    // 0x70 suspend
    w(0x70, 0, 0, 0, 0, 0),           // 39  YIELD
    w(0x71, 0, 0, 0, 0, 0x3F00_0000), // 40  WAIT   0.5
    // 0x80 excite
    w(0x80, 0, 20, 0, 0, 1),          // 41  FEXCITE brow (muscle 1), f20
    // 0x90 task variables
    w(0x90, 21, 0, 0, 0, 2),          // 42  FVGET  f21, var_f[2] = tmp
    w(0x91, 0, 1, 0, 0, 1),           // 43  FVSET  var_f[1] = phase, f1
    w(0x92, 8, 0, 0, 0, 1),           // 44  IVGET  i8, var_i[1] = state
    w(0x93, 0, 1, 0, 0, 2),           // 45  IVSET  var_i[2] = limit, i1
    // 0xA0 misc
    w(0xA1, 0, 0, 0, 0, 0),           // 46  NOP
    w(0xA2, 0, 0, 0, 0, 0),           // 47  FLUSH
    w(0xA0, 0, 0, 0, 0, 0),           // 48  HALT
];

/// Build the 96-byte fixed header at the exact offsets of `binfmt.md` §4.
///
/// Written as an explicit offset table rather than a struct, so it inherits no
/// ordering decision from the implementation. Every scalar is non-default, so a
/// loader that hard-coded a default would be caught.
fn hand_header(file_len: usize, code_len: usize) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];

    h[0..4].copy_from_slice(b"CSPX"); //  0  magic
    h[4..6].copy_from_slice(&0x0100u16.to_le_bytes()); //  4  format 1.0
    h[6..8].copy_from_slice(&1u16.to_le_bytes()); //  6  isa_version = 1
    h[8..10].copy_from_slice(&0u16.to_le_bytes()); //  8  reserved_a = 0
    h[10..12].copy_from_slice(&0u16.to_le_bytes()); // 10  flags = 0
    h[12..16].copy_from_slice(&(file_len as u32).to_le_bytes()); // 12  file_len
    h[16..20].copy_from_slice(&(code_len as u32).to_le_bytes()); // 16  code_len
    h[20..22].copy_from_slice(&3u16.to_le_bytes()); // 20  layer = 3
    h[22..24].copy_from_slice(&0u16.to_le_bytes()); // 22  reserved0 = 0
    h[24..28].copy_from_slice(&2u32.to_le_bytes()); // 24  muscle_count = 2
    h[28..32].copy_from_slice(&4u32.to_le_bytes()); // 28  sensor_f_count = 4
    h[32..36].copy_from_slice(&3u32.to_le_bytes()); // 32  sensor_i_count = 3
    h[36..40].copy_from_slice(&3u32.to_le_bytes()); // 36  var_f_count = 3
    h[40..44].copy_from_slice(&3u32.to_le_bytes()); // 40  var_i_count = 3
    h[44..48].copy_from_slice(&120u32.to_le_bytes()); // 44  hz = 120
    h[48..52].copy_from_slice(&0u32.to_le_bytes()); // 48  reserved_b = 0
    h[52..56].copy_from_slice(&8192u32.to_le_bytes()); // 52  fuel = 8192
    h[56..60].copy_from_slice(&12.5f32.to_bits().to_le_bytes()); // 56  max_wait = 12.5
    h[60..64].copy_from_slice(&0u32.to_le_bytes()); // 60  reserved1 = 0
    // 64..96 reserved2 stays all zero.

    h
}

/// The valid file the golden set is built around, assembled byte for byte.
///
/// The program uses every opcode once, then falls through a backward `JMP` and a
/// run of conditional branches to the trailing `HALT` at pc 48.
fn handmade_valid() -> Vec<u8> {
    // --- metadata tables, in the §3 order, gap free ---
    let mut tables: Vec<u8> = Vec::new();

    // muscles (§6.2): index is the muscle id.
    tables.extend_from_slice(b"mouth\0");
    tables.extend_from_slice(b"brow\0");

    // sensors_f (§6.3): the first muscle_count entries are the read-back (M4),
    // then the built-in dt, then a backend observable.
    tables.extend_from_slice(b"mouth\0");
    tables.push(0); // src = MUSCLE
    tables.extend_from_slice(b"brow\0");
    tables.push(0); // src = MUSCLE
    tables.extend_from_slice(b"dt\0");
    tables.push(2); // src = BUILTIN_DT
    tables.extend_from_slice(b"pressure\0");
    tables.push(1); // src = BACKEND

    // sensors_i (§6.3): the built-in tick, then two backend observables.
    tables.extend_from_slice(b"tick\0");
    tables.push(3); // src = BUILTIN_TICK
    tables.extend_from_slice(b"errors\0");
    tables.push(1); // src = BACKEND
    tables.extend_from_slice(b"phase\0");
    tables.push(1); // src = BACKEND

    // vars_f (§6.4): initial values are f32 bit patterns, NaN included.
    tables.extend_from_slice(b"gain\0");
    tables.extend_from_slice(&0.5f32.to_bits().to_le_bytes());
    tables.extend_from_slice(b"phase\0");
    tables.extend_from_slice(&0.0f32.to_bits().to_le_bytes());
    tables.extend_from_slice(b"tmp\0");
    tables.extend_from_slice(&f32::from_bits(0x7FC0_0001).to_bits().to_le_bytes());

    // vars_i (§6.4): two's-complement i32.
    tables.extend_from_slice(b"count\0");
    tables.extend_from_slice(&(-7i32).to_le_bytes());
    tables.extend_from_slice(b"state\0");
    tables.extend_from_slice(&0i32.to_le_bytes());
    tables.extend_from_slice(b"limit\0");
    tables.extend_from_slice(&100i32.to_le_bytes());

    let file_len = HEADER_LEN + 8 * CODE.len() + tables.len() + HASH_LEN;

    let mut bytes = Vec::with_capacity(file_len);
    bytes.extend_from_slice(&hand_header(file_len, CODE.len()));
    for word in CODE {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&tables);

    // §7: the hash covers every byte before it.
    let digest = blake3::hash(&bytes);
    bytes.extend_from_slice(digest.as_bytes());

    assert_eq!(bytes.len(), file_len, "the hand-written layout must be self-consistent");
    bytes
}

/// Patch bytes and repair the trailing hash (§7), so only the intended defect
/// remains for the loader to find.
fn rehash(bytes: &[u8], patches: &[(usize, &[u8])]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for (offset, value) in patches {
        out[*offset..*offset + value.len()].copy_from_slice(value);
    }
    let end = out.len() - HASH_LEN;
    let digest = blake3::hash(&out[..end]);
    out[end..].copy_from_slice(digest.as_bytes());
    out
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

/// The code section starts here; word `i` is at `code_word(i)`.
const fn code_word(i: usize) -> usize {
    HEADER_LEN + 8 * i
}

/// The last instruction of the valid golden.
const LAST: usize = CODE_LEN - 1;

// ---------------------------------------------------------------------------
// Golden generation (deliberate, never part of a normal test run)
// ---------------------------------------------------------------------------

/// Rewrite the fixture files from the hand-written baseline.
///
/// Run with `cargo test --test handmade -- --ignored`. The committed files are
/// the golden reference; this exists so the bytes are reproducible and the
/// layout is documented in code.
#[test]
#[ignore = "writes tests/fixtures/*.csp; run deliberately to regenerate goldens"]
fn regenerate_fixtures() {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir).expect("create tests/fixtures");

    let valid = handmade_valid();
    fs::write(dir.join(VALID), &valid).expect("write the valid golden");

    // Corruption, not forgery: flip a code bit and leave the stored hash alone.
    let mut bad_hash = valid.clone();
    bad_hash[HEADER_LEN] ^= 0x01;
    fs::write(dir.join(INVALID_BAD_HASH), &bad_hash).expect("write the bad-hash golden");

    // A reserved opcode with a correctly repaired hash, so I1 is the rule that
    // fires rather than C6.
    let reserved = rehash(&valid, &[(code_word(0), &(0x04u64 << 56).to_le_bytes())]);
    fs::write(dir.join(INVALID_RESERVED), &reserved).expect("write the reserved-opcode golden");

    eprintln!("regenerated 3 fixtures in {}", dir.display());
}

// ===========================================================================
// The hand-written layout is what the spec says
// ===========================================================================

#[test]
fn the_hand_assembled_bytes_match_the_offset_table() {
    let bytes = handmade_valid();
    assert_eq!(bytes.len(), 636, "96 header + 392 code + 116 tables + 32 hash");

    // §4, field by field, read straight out of the byte string.
    assert_eq!(&bytes[0..4], b"CSPX");
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 0x0100);
    assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 1);
    assert_eq!(
        u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        0,
        "reserved_a must be zero"
    );
    assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 636);
    assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 49);
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 4);
    assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 3);
    assert_eq!(u32::from_le_bytes(bytes[44..48].try_into().unwrap()), 120);
    assert_eq!(
        u32::from_le_bytes(bytes[48..52].try_into().unwrap()),
        0,
        "reserved_b must be zero"
    );
    assert_eq!(u32::from_le_bytes(bytes[52..56].try_into().unwrap()), 8192);
    assert_eq!(u32::from_le_bytes(bytes[56..60].try_into().unwrap()), 0x4148_0000); // 12.5f32
    assert_eq!(&bytes[64..96], &[0u8; 32], "reserved2 must be zero");

    // §5: little-endian words, top byte = opcode. pc 0 is FMOVI, the last is HALT.
    assert_eq!(bytes[code_word(0) + 7], 0x00, "pc 0 must be FMOVI");
    assert_eq!(bytes[code_word(LAST) + 7], 0xA0, "the last word must be HALT");

    // §7: the hash is over everything before it.
    assert_eq!(
        &bytes[bytes.len() - HASH_LEN..],
        blake3::hash(&bytes[..bytes.len() - HASH_LEN]).as_bytes()
    );
}

// ===========================================================================
// Golden: the valid file
// ===========================================================================

#[test]
fn golden_valid_all_exercises_every_opcode() {
    let image = Image::from_path(fixtures_dir().join(VALID)).expect("the golden file is valid");

    let mut seen: Vec<u8> = image.code.iter().map(|insn| insn.op.byte()).collect();
    seen.sort_unstable();
    seen.dedup();

    let mut all: Vec<u8> = OpCode::ALL.iter().map(|op| op.byte()).collect();
    all.sort_unstable();

    assert_eq!(image.code.len(), CODE_LEN, "one instruction per opcode");
    assert_eq!(
        seen, all,
        "the valid golden must contain every one of the {} opcodes",
        OpCode::ALL.len()
    );
}

#[test]
fn golden_valid_all_loads_and_decodes() {
    let image = Image::from_path(fixtures_dir().join(VALID)).expect("the golden file is valid");

    // Header: every scalar non-default.
    assert_eq!(image.header.layer, 3);
    assert_eq!(image.header.isa_version, 1);
    assert_eq!(image.header.hz, 120);
    assert_eq!(image.header.fuel, 8192);
    assert_eq!(image.header.max_wait, 12.5);

    // Code, spot-checked at the points where an operand reading could go wrong.
    assert_eq!(image.code.len(), CODE_LEN);
    assert_eq!(image.code[0].op, OpCode::Fmovi);
    assert_eq!(image.code[0].rd.get(), 1);
    assert_eq!(image.code[0].imm_f32(), 1.0);
    assert_eq!(image.code[22].op, OpCode::Itof); // mixed register files
    assert_eq!(image.code[23].op, OpCode::Ftoi);
    assert_eq!(image.code[24].op, OpCode::Jmp);
    assert_eq!(image.code[24].imm_i32(), -25, "a backward jump");
    assert_eq!(image.code[37].op, OpCode::Fsense);
    assert_eq!(image.code[37].rd.get(), 19);
    assert_eq!(image.code[37].imm_index(), 3);
    assert_eq!(image.code[40].op, OpCode::Wait);
    assert_eq!(image.code[40].imm_f32(), 0.5);
    assert_eq!(image.code[41].op, OpCode::Fexcite);
    assert_eq!(image.code[41].ra.get(), 20);
    assert_eq!(image.code[41].imm_index(), 1);
    // `FLUSH` sits between `NOP` and the trailing `HALT`, and its canonical form
    // is the bare opcode: every other field zero (§9.3 I2).
    assert_eq!(image.code[47].op, OpCode::Flush);
    assert_eq!(
        image.code[47].encode(),
        0xA2u64 << 56,
        "FLUSH must be a zero-operand word"
    );
    assert_eq!(image.code[LAST].op, OpCode::Halt);

    // Metadata: every table populated, read-back invariant (M4) intact.
    assert_eq!(image.muscles.len(), 2);
    assert_eq!(&*image.muscles[0].name, "mouth");
    assert_eq!(&*image.muscles[1].name, "brow");

    assert_eq!(image.sensors_f.len(), 4);
    assert_eq!(&*image.sensors_f[0].name, "mouth");
    assert_eq!(image.sensors_f[0].source, SensorSource::Muscle);
    assert_eq!(&*image.sensors_f[1].name, "brow");
    assert_eq!(image.sensors_f[1].source, SensorSource::Muscle);
    assert_eq!(&*image.sensors_f[2].name, "dt");
    assert_eq!(image.sensors_f[2].source, SensorSource::BuiltinDt);
    assert_eq!(&*image.sensors_f[3].name, "pressure");
    assert_eq!(image.sensors_f[3].source, SensorSource::Backend);

    assert_eq!(image.sensors_i.len(), 3);
    assert_eq!(&*image.sensors_i[0].name, "tick");
    assert_eq!(image.sensors_i[0].source, SensorSource::BuiltinTick);
    assert_eq!(image.sensors_i[1].source, SensorSource::Backend);

    assert_eq!(image.vars_f.len(), 3);
    assert_eq!(image.vars_f[0].init, 0.5);
    assert_eq!(image.vars_f[1].init, 0.0);
    assert_eq!(image.vars_f[2].init.to_bits(), 0x7FC0_0001, "NaN payload survives");

    assert_eq!(image.vars_i.len(), 3);
    assert_eq!(image.vars_i[0].init, -7);
    assert_eq!(image.vars_i[2].init, 100);
}

#[test]
fn golden_valid_all_is_byte_exact() {
    let path = fixtures_dir().join(VALID);
    let bytes = fixture_bytes(VALID);

    // The committed artifact must be exactly the layout this file documents, so
    // an accidental edit (or a silent format change) is caught here.
    assert_eq!(
        bytes,
        handmade_valid(),
        "tests/fixtures/{VALID} drifted from the documented layout"
    );

    // And the writer must reproduce a file it did not produce (§8).
    let image = Image::from_path(&path).expect("the golden file is valid");
    assert_eq!(image.to_bytes(), bytes, "the writer must reproduce the golden bytes");
}

// ===========================================================================
// Golden: the invalid files
// ===========================================================================

#[test]
fn golden_invalid_bad_hash_is_refused() {
    match Image::from_path(fixtures_dir().join(INVALID_BAD_HASH)) {
        Err(LoadError::ContentHashMismatch { .. }) => {}
        other => panic!("the bad-hash golden was not refused by C6: {other:?}"),
    }
}

#[test]
fn golden_invalid_reserved_opcode_is_refused() {
    match Image::from_path(fixtures_dir().join(INVALID_RESERVED)) {
        Err(LoadError::InvalidInstruction { index: 0, source }) => {
            assert_eq!(source.to_string(), "Invalid Operation Code 4");
        }
        other => panic!("the reserved-opcode golden was not refused by I1: {other:?}"),
    }
}

// ===========================================================================
// Derived negatives, patched from the in-memory baseline (no extra files)
// ===========================================================================

#[test]
fn a_bad_magic_byte_is_refused_before_the_hash() {
    // The magic is checked while reading the header, so this fires even though
    // the hash was not repaired.
    let mut bytes = handmade_valid();
    bytes[0] = b'X';
    assert!(matches!(reject(&bytes), LoadError::BadMagic { found } if &found == b"XSPX"));
}

#[test]
fn a_corrupt_stored_hash_is_refused() {
    let mut bytes = handmade_valid();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    assert!(matches!(reject(&bytes), LoadError::ContentHashMismatch { .. }));
}

#[test]
fn a_word_with_unused_bits_is_refused() {
    // NOP with imm = 1: the opcode is defined, but the word is not canonical.
    let nop_with_imm = (0xA1u64 << 56) | 1;
    let bytes = rehash(&handmade_valid(), &[(code_word(0), &nop_with_imm.to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::NonCanonicalInstruction { index: 0, .. }
    ));
}

#[test]
fn a_file_not_ending_in_halt_is_refused() {
    let yield_word = 0x70u64 << 56;
    let bytes = rehash(&handmade_valid(), &[(code_word(LAST), &yield_word.to_le_bytes())]);
    assert!(matches!(reject(&bytes), LoadError::MissingTrailingHalt));
}

#[test]
fn a_jump_past_the_end_is_refused() {
    // JMP +100 from pc 0 targets instruction 101 of a 49-instruction program.
    let jmp = (0x50u64 << 56) | 100;
    let bytes = rehash(&handmade_valid(), &[(code_word(0), &jmp.to_le_bytes())]);
    match reject(&bytes) {
        LoadError::JumpTargetOutOfRange { index: 0, offset: 100, code_len: 49 } => {}
        other => panic!("a jump past the end was not refused by I6: {other}"),
    }
}

#[test]
fn a_sensor_index_past_the_table_is_refused() {
    // FSENSE imm = 5 with only 4 sensors declared.
    let fsense = (0x60u64 << 56) | (5u64 << 50) | 5;
    let bytes = rehash(&handmade_valid(), &[(code_word(0), &fsense.to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::SensorIndexOutOfRange {
            index: 0,
            table: Table::SensorsF,
            sensor_id: 5,
            count: 4
        }
    ));
}

#[test]
fn a_declared_length_that_disagrees_with_the_bytes_is_refused() {
    let len = handmade_valid().len() as u32;
    let bytes = rehash(&handmade_valid(), &[(12, &(len + 1).to_le_bytes())]);
    assert!(matches!(
        reject(&bytes),
        LoadError::FileLenMismatch { declared, actual } if declared == len + 1 && actual == len as usize
    ));
}
