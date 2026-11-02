//! Builds a rig that contains every name the fuzz input mentions.
//!
//! `docs/asm.md` §3.2 moved muscles and observables out of the source, so a
//! generated source that names one would otherwise stop at "unknown name" and
//! never reach the code this fuzzer is about. Deriving the rig from the input
//! keeps the accept path reachable for arbitrary bytes.
//!
//! The rig has to be *legal*, or `generate_image` rejects it before emitting
//! anything: no built-in names, and no name in two tables. A name that appears
//! as both a muscle and a sensor stays a muscle — its read-back has the same
//! name, so the sensor reference still resolves.

use std::collections::HashSet;

use cyberspine_asm::assembler::Asm;
use cyberspine_asm::assembler::text::{AsmOperand, TextItem};
use cyberspine_core::isa::OpCode;
use cyberspine_core::rig::{Name, Rig};

fn is_reserved(name: &str) -> bool {
    if matches!(name, "dt" | "tick") {
        return true;
    }
    let mut chars = name.chars();
    if !matches!(chars.next(), Some('f' | 'i')) {
        return false;
    }
    let rest = chars.as_str();
    !rest.is_empty() && rest.chars().all(|character| character.is_ascii_digit())
}

pub fn rig_for(asm: &Asm) -> Rig {
    let mut muscles: Vec<&str> = Vec::new();
    let mut sensors_f: Vec<&str> = Vec::new();
    let mut sensors_i: Vec<&str> = Vec::new();

    let groups: HashSet<&str> = asm
        .header
        .muscle_groups
        .iter()
        .map(|group| group.name.as_str())
        .collect();

    for group in &asm.header.muscle_groups {
        muscles.extend(group.members.iter().map(|member| member.as_str()));
    }

    for item in &asm.text {
        let TextItem::Insn(insn) = &item.item else {
            continue;
        };
        for operand in &insn.operands {
            let AsmOperand::Index(name) = operand else {
                continue;
            };
            let name = name.as_str();
            if is_reserved(name) {
                continue;
            }
            match insn.op {
                OpCode::Fexcite if !groups.contains(name) => muscles.push(name),
                OpCode::Fsense => sensors_f.push(name),
                OpCode::Isense => sensors_i.push(name),
                _ => {}
            }
        }
    }

    let mut seen = HashSet::new();
    let mut rig = Rig::new("fuzz", 1);
    for name in muscles {
        if seen.insert(name) {
            rig = rig.muscle(Name::new(name).expect("the ident parser only produces identifiers"));
        }
    }
    for name in sensors_f {
        if seen.insert(name) {
            rig = rig.sensor_f(Name::new(name).expect("the ident parser only produces identifiers"));
        }
    }
    for name in sensors_i {
        if seen.insert(name) {
            rig = rig.sensor_i(Name::new(name).expect("the ident parser only produces identifiers"));
        }
    }
    rig
}
