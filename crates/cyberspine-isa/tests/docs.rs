//! The documented instruction table against the one the machine runs.
//!
//! `docs/isa.md` §4 is where a reader learns which byte is which mnemonic;
//! `isa_table!` is what the assembler and the VM actually execute. The two
//! cannot be derived from each other — the document owns the semantics column,
//! and only the code knows the operand types — so they are compared instead.
//!
//! Three columns are comparable, because the code decides all three: the opcode,
//! the mnemonic, and the operand names. Nothing here reads the semantics prose.

use cyberspine_isa::{OpCode, OperandParam, RegName};

const DOC: &str = include_str!("../../../docs/isa.md");

/// One row of `docs/isa.md` §4.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    opcode: u8,
    mnemonic: String,
    operands: Vec<String>,
}

fn documented() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut inside = false;

    for line in DOC.lines() {
        if line.starts_with("## 4.") {
            inside = true;
            continue;
        }
        if line.starts_with("## 5.") {
            break;
        }
        if !inside || !line.starts_with("| 0x") {
            continue;
        }

        let cells = line.split('|').map(str::trim).collect::<Vec<_>>();
        let (opcode, mnemonic, operands) = (cells[1], cells[2], cells[3]);
        let opcode = u8::from_str_radix(opcode.trim_start_matches("0x"), 16)
            .unwrap_or_else(|_| panic!("not an opcode byte: {line}"));
        let mnemonic = mnemonic.trim_matches('`').to_owned();
        let operands = match operands.find('`') {
            Some(start) => {
                let rest = &operands[start + 1..];
                let end = rest
                    .find('`')
                    .unwrap_or_else(|| panic!("unclosed operand list: {line}"));
                rest[..end]
                    .split(',')
                    .map(|operand| operand.trim().to_owned())
                    .collect()
            }
            None => {
                assert_eq!(operands, "—", "not an operand cell: {line}");
                Vec::new()
            }
        };
        rows.push(Row {
            opcode,
            mnemonic,
            operands,
        });
    }

    assert!(
        rows.len() > 40,
        "docs/isa.md §4 lost its table shape: found {} rows",
        rows.len()
    );
    rows
}

fn executed(opcode: OpCode) -> Row {
    let operands = opcode
        .signature()
        .iter()
        .map(|operand| match operand {
            OperandParam::Reg(RegName::Rd, _) => "rd",
            OperandParam::Reg(RegName::Ra, _) => "ra",
            OperandParam::Reg(RegName::Rb, _) => "rb",
            OperandParam::Reg(RegName::Rc, _) => "rc",
            OperandParam::Imm(_) => "imm",
        })
        .map(str::to_owned)
        .collect();

    Row {
        opcode: opcode.byte(),
        mnemonic: opcode.mnemonic().to_owned(),
        operands,
    }
}

#[test]
fn the_documented_table_is_the_one_that_runs() {
    let documented = documented();
    let executed = OpCode::ALL
        .iter()
        .copied()
        .map(executed)
        .collect::<Vec<_>>();

    let missing = executed
        .iter()
        .filter(|row| !documented.contains(row))
        .collect::<Vec<_>>();
    let extra = documented
        .iter()
        .filter(|row| !executed.contains(row))
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "docs/isa.md §4 disagrees with isa_table!:\n  \
         in `isa_table!` but not in the document: {missing:#?}\n  \
         in the document but not in `isa_table!`: {extra:#?}",
    );
    assert_eq!(
        documented.len(),
        executed.len(),
        "a row is documented twice, or one opcode is documented twice"
    );
}
