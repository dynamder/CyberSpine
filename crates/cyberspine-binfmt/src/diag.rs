//! Where the loader's refusals meet the shared diagnostic vocabulary.
//!
//! Every [`LoadError`] is an error, and its `code` is the rule that rejected the
//! image — the same identifier the doc comment on the variant carries, so the
//! checklist in `docs/binfmt.md` §9 and the diagnostics a caller can filter by are
//! one list rather than two.

use alloc::format;
use alloc::string::ToString;
use core::fmt;

use cyberspine_diag::{DiagnosticKind, Level};

use crate::load::LoadError;

impl DiagnosticKind for LoadError {
    fn level(&self) -> Level {
        Level::Error
    }

    fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "C0",
            Self::TooShort { .. } => "C1",
            Self::BadMagic { .. } => "C1",
            Self::FileLenMismatch { .. } => "C1",
            Self::UnsupportedFormatVersion { .. } => "C2",
            Self::UnsupportedIsaVersion { .. } => "C2",
            Self::ReservedNotZero { .. } => "C3",
            Self::ZeroHz => "C4",
            Self::InvalidMaxWait { .. } => "C4",
            Self::CountsExceedFileLen { .. } => "C5",
            Self::ContentHashMismatch { .. } => "C6",
            Self::TrailingBytes { .. } => "C7",
            // The mirror of C7: the tables ran past the end instead of stopping
            // short of it. Both are "the declared layout and the bytes disagree",
            // which C1 is the rule for.
            Self::Truncated { .. } => "C1",
            Self::CodeLenZero => "C8",
            Self::MissingTrailingHalt => "C8",
            Self::EmptyName { .. } => "M1",
            Self::NameNotAnIdentifier { .. } => "M1",
            Self::UnterminatedName { .. } => "M1",
            Self::DuplicateName { .. } => "M2",
            Self::SensorFCountBelowMuscleCount { .. } => "M3",
            Self::ReadbackNameMismatch { .. } => "M4",
            Self::ReadbackSourceMismatch { .. } => "M4",
            Self::UnknownSensorSource { .. } => "M5",
            Self::SensorSourceNotAllowedHere { .. } => "M5",
            Self::DuplicateBuiltinDt { .. } => "M5",
            Self::DuplicateBuiltinTick { .. } => "M5",
            Self::MuscleSourceNotAllowedHere { .. } => "M6",
            Self::UnterminatedReason { .. } => "M7",
            Self::ReasonNotUtf8 { .. } => "M7",
            Self::EmptyReason { .. } => "M7",
            Self::ReasonTooLong { .. } => "M7",
            Self::InvalidReason { .. } => "M7",
            Self::InvalidInstruction { .. } => "I1",
            Self::NonCanonicalInstruction { .. } => "I2",
            Self::SensorIndexOutOfRange { .. } => "I3",
            Self::VarIndexOutOfRange { .. } => "I4",
            Self::MuscleIndexOutOfRange { .. } => "I5",
            Self::ReasonIndexOutOfRange { .. } => "I9",
            Self::JumpTargetOutOfRange { .. } => "I6",
            Self::WaitNotFiniteOrNegative { .. } => "I7",
            Self::WaitExceedsMaxWait { .. } => "I7",
        }
    }
}

/// Something the loader accepted, with a caveat.
///
/// A warning never makes a load fail by itself. It exists for the cases where the
/// image is *readable* but the loader is not the authority that produced it — a
/// newer format minor — which is exactly the situation where refusing would
/// punish a working image and accepting silently would hide the mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// The image declares a format minor this loader does not know (§10).
    NewerMinor { declared: u8, known: u8 },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewerMinor { declared, known } => f.write_str(&format!(
                "the image declares format minor {declared}, this loader knows {known}; \
                 a minor change must not alter parsing, so the rest of the checks decide"
            )),
        }
    }
}

impl DiagnosticKind for Warning {
    fn level(&self) -> Level {
        Level::Warning
    }

    fn code(&self) -> &'static str {
        match self {
            Self::NewerMinor { .. } => "C2",
        }
    }
}

impl Warning {
    /// The message a caller renders. Kept for the same reason `LoadError` has one.
    pub fn message(&self) -> alloc::string::String {
        self.to_string()
    }
}
