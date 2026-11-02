//! Readability warnings: what they catch, and where they point.

use cyberspine_asm::assembler::{Location, WarningKind, lint, parse};

const HEADER: &str =
    ".layer 10\n.clock hz = 60\n.fuel 4096\n.max_wait 600.0\n.rig \"r.rig.json\"\n";

fn warnings(body: &str) -> Vec<(Location, WarningKind)> {
    let asm = parse(&format!("{HEADER}{body}")).expect("the fixture parses");
    lint(&asm)
        .into_iter()
        .map(|warning| (warning.location, warning.kind))
        .collect()
}

#[test]
fn an_unused_label_is_reported_where_it_is_written() {
    let found = warnings(".text\nunused:\n    NOP\n    HALT\n");
    assert_eq!(
        found,
        vec![(
            Location::new(7, 1),
            WarningKind::UnusedLabel("unused".into())
        )]
    );
}

#[test]
fn a_targeted_label_is_not_reported() {
    assert_eq!(warnings(".text\nloop:\n    JMP loop\n"), Vec::new());
}

#[test]
fn a_single_member_group_is_reported_at_its_use() {
    let found = warnings(
        ".group brows = brow_l\n.text\n    FMOVI   f1, 1.0\n    FEXCITE brows, f1\n    FLUSH\n    HALT\n",
    );
    assert_eq!(
        found,
        vec![(
            Location::new(9, 5),
            WarningKind::SingleMemberGroup("brows".into())
        )]
    );
}

#[test]
fn warnings_are_ordered_by_position() {
    let found = warnings(".text\nfirst:\nsecond:\n    NOP\n    HALT\n");
    let lines: Vec<usize> = found.iter().map(|(location, _)| location.line).collect();
    assert_eq!(lines, vec![7, 8]);
}
