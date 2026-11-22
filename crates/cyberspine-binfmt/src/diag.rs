//! Where the loader's refusals meet the shared diagnostic vocabulary.
//!
//! Every [`LoadError`] is an error, and its code is the rule that rejected the
//! image — the same identifier the variant's doc comment carries, so the checklist
//! in `docs/binfmt.md` §9 and the codes a caller can filter by are one list rather
//! than two.
//!
//! The loader reports in image coordinates only. It never consults the debug
//! segment to explain a refusal: at load time the debug segment is itself still a
//! suspect (C9), and using it to locate another error would be circular. Turning a
//! `pc` into a source line is the host's job, after the image has been accepted.

use alloc::format;
use alloc::string::ToString;
use core::fmt;

use crate::load::LoadError;

cyberspine_diag::codes! {
    LoadError: Error {
        Self::TooLarge { .. } => "binfmt.C0",
        Self::TooShort { .. } => "binfmt.C1",
        Self::BadMagic { .. } => "binfmt.C1",
        Self::FileLenMismatch { .. } => "binfmt.C1",
        Self::UnsupportedFormatVersion { .. } => "binfmt.C2",
        Self::UnsupportedIsaVersion { .. } => "binfmt.C2",
        Self::ReservedNotZero { .. } => "binfmt.C3",
        Self::ZeroHz => "binfmt.C4",
        Self::InvalidMaxWait { .. } => "binfmt.C4",
        Self::CountsExceedFileLen { .. } => "binfmt.C5",
        Self::ContentHashMismatch { .. } => "binfmt.C6",
        Self::TrailingBytes { .. } => "binfmt.C7",
        Self::Debug(_) => "binfmt.C9",
        // The mirror of C7: the tables ran past the end instead of stopping short
        // of it. Both are "the declared layout and the bytes disagree", which C1
        // is the rule for.
        Self::Truncated { .. } => "binfmt.C1",
        Self::CodeLenZero => "binfmt.C8",
        Self::MissingTrailingHalt => "binfmt.C8",
        Self::EmptyName { .. } => "binfmt.M1",
        Self::NameNotAnIdentifier { .. } => "binfmt.M1",
        Self::UnterminatedName { .. } => "binfmt.M1",
        Self::DuplicateName { .. } => "binfmt.M2",
        Self::SensorFCountBelowMuscleCount { .. } => "binfmt.M3",
        Self::ReadbackNameMismatch { .. } => "binfmt.M4",
        Self::ReadbackSourceMismatch { .. } => "binfmt.M4",
        Self::UnknownSensorSource { .. } => "binfmt.M5",
        Self::SensorSourceNotAllowedHere { .. } => "binfmt.M5",
        Self::DuplicateBuiltinDt { .. } => "binfmt.M5",
        Self::DuplicateBuiltinTick { .. } => "binfmt.M5",
        Self::MuscleSourceNotAllowedHere { .. } => "binfmt.M6",
        Self::UnterminatedReason { .. } => "binfmt.M7",
        Self::ReasonNotUtf8 { .. } => "binfmt.M7",
        Self::EmptyReason { .. } => "binfmt.M7",
        Self::ReasonTooLong { .. } => "binfmt.M7",
        Self::InvalidReason { .. } => "binfmt.M7",
        Self::InvalidInstruction { .. } => "binfmt.I1",
        Self::NonCanonicalInstruction { .. } => "binfmt.I2",
        Self::SensorIndexOutOfRange { .. } => "binfmt.I3",
        Self::VarIndexOutOfRange { .. } => "binfmt.I4",
        Self::MuscleIndexOutOfRange { .. } => "binfmt.I5",
        Self::ReasonIndexOutOfRange { .. } => "binfmt.I9",
        Self::JumpTargetOutOfRange { .. } => "binfmt.I6",
        Self::WaitNotFiniteOrNegative { .. } => "binfmt.I7",
        Self::WaitExceedsMaxWait { .. } => "binfmt.I7",
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

cyberspine_diag::codes! {
    Warning: Warning {
        Self::NewerMinor { .. } => "binfmt.C2",
    }
}

impl Warning {
    /// The message a caller renders. Kept for the same reason `LoadError` has one.
    pub fn message(&self) -> alloc::string::String {
        self.to_string()
    }
}
