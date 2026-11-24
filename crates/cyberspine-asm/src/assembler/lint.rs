//! Readability warnings: what the source can fix, and what is not an error.
//!
//! This is a pass over the instruction section's own text — no codegen, no rig —
//! so everything it reports is located exactly. See [`Warning`].
//!
//! It produces [`cyberspine_diag::Diagnostic`] values and nothing else: rendering
//! belongs to `cyberspine-report`, so a warning and an error reach a terminal
//! through exactly one path.

use cyberspine_core::isa::OpCode;
use cyberspine_diag::Diagnostic;
use smol_str::SmolStr;

use super::Asm;
use super::diagnostic::Location;
use super::text::{AsmOperand, TextItem};

/// A readability problem the source can fix but that is not an error.
///
/// Same shape as a [`ParseError`](super::ParseError): a stable kind for tools, a
/// [`Location`] for a human. They are reported by the CLI at warning level and
/// never stop an assembly unless `--deny-warnings` asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub location: Location,
    pub kind: WarningKind,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WarningKind {
    #[error("label `{0}` is never a jump target")]
    UnusedLabel(SmolStr),
    #[error("group `{0}` has a single member; name the muscle directly")]
    SingleMemberGroup(SmolStr),
}

/// `level: line:column: kind`, the same shape [`ParseError`](super::ParseError)
/// renders, because both are meant to be read at a terminal.
impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "line {}:{}: {}",
            self.location.line, self.location.column, self.kind
        )
    }
}

cyberspine_diag::codes! {
    WarningKind: Warning {
        Self::UnusedLabel(_) => "asm.unused-label",
        Self::SingleMemberGroup(_) => "asm.single-member-group",
    }
}

impl Warning {
    /// This warning as a diagnostic, located in `file`.
    pub fn diagnostic(&self, file: &str) -> Diagnostic {
        Diagnostic::of(&self.kind).at(super::diagnostic::anchor(file, self.location))
    }
}

/// Readability diagnostics over a parsed source: no codegen, no rig.
///
/// Scope is deliberately narrow — only what can be decided from the instruction
/// section's own text, and therefore located exactly.
pub fn lint(asm: &Asm) -> Vec<Warning> {
    let mut labels: Vec<(&SmolStr, Location)> = Vec::new();
    let mut targeted: Vec<&str> = Vec::new();
    let mut groups: Vec<(&str, usize)> = asm
        .header
        .muscle_groups
        .iter()
        .map(|declared| {
            let group = &declared.item;
            (group.name.as_str(), group.members.len())
        })
        .collect();
    let mut warnings = Vec::new();

    for located in &asm.text {
        match &located.item {
            TextItem::Label(name) => labels.push((name, located.location)),
            TextItem::Insn(insn) => {
                for operand in &insn.operands {
                    match operand {
                        AsmOperand::Target(name) => targeted.push(name.as_str()),
                        AsmOperand::Index(name) if insn.op == OpCode::Fexcite => {
                            if let Some((_, members)) =
                                groups.iter().find(|(group, _)| *group == name.as_str())
                                && *members == 1
                            {
                                warnings.push(Warning {
                                    location: located.location,
                                    kind: WarningKind::SingleMemberGroup(name.clone()),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    for (name, location) in labels {
        if !targeted.iter().any(|target| *target == name.as_str()) {
            warnings.push(Warning {
                location,
                kind: WarningKind::UnusedLabel(name.clone()),
            });
        }
    }

    groups.clear();
    warnings.sort_by_key(|warning| (warning.location.line, warning.location.column));
    warnings
}
