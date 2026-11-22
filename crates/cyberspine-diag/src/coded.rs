use alloc::format;
use alloc::string::String;
use core::fmt;

use crate::{Code, Level};

/// An error enum that can name itself in the shared vocabulary.
///
/// `code()` has no default: a new variant that has not been given a stable
/// identifier is a compile error in the `match` the [`codes!`] macro generates.
pub trait Coded: fmt::Display {
    const LEVEL: Level;
    fn code(&self) -> Code;
}

/// Map an error enum onto the code table, and derive the table from the mapping.
///
/// This is the one place a producer writes the variant → identifier pairs. It
/// generates both the [`Coded`] implementation and `Type::CODES`, so the
/// uniqueness test and the documentation table read the same list the compiler
/// checks for exhaustiveness.
///
/// ```ignore
/// codes! {
///     LoadError: Error {
///         Self::TooShort { .. } => "binfmt.C1",
///         Self::ContentHashMismatch { .. } => "binfmt.C6",
///     }
/// }
/// ```
///
/// The level is per table because it is per type: every variant of a loader
/// error is an error, every variant of a lint warning is a warning.
#[macro_export]
macro_rules! codes {
    ($ty:ident : $level:ident { $($arm:pat => $code:literal),* $(,)? }) => {
        impl $crate::Coded for $ty {
            const LEVEL: $crate::Level = $crate::Level::$level;

            fn code(&self) -> $crate::Code {
                match self {
                    $( $arm => $crate::Code::new($code), )*
                }
            }
        }

        impl $ty {
            /// Every code this type can report, in declaration order.
            pub const CODES: &'static [$crate::Code] =
                &[ $( $crate::Code::new($code) ),* ];
        }
    };
}

/// Check one crate's table: namespaced, and all in one namespace.
///
/// Codes are coarse — several variants legitimately share one (`binfmt.C1` is
/// "the declared layout and the bytes disagree", in six shapes) — so uniqueness
/// is not the invariant. Owning exactly one namespace is: a table that mixes
/// `asm.` and `binfmt.` means a crate is reporting for another one.
///
/// Called from each crate's own tests. There is deliberately no registry across
/// crates: the dependency graph already keeps `asm`, `binfmt` and `exec` from
/// sharing a code space, so the only invariant worth testing is local.
pub fn table_is_sane(codes: &[Code]) -> Result<(), String> {
    let Some(first) = codes.first() else {
        return Err(String::from("the code table is empty"));
    };
    let namespace = first.namespace();

    for code in codes {
        let text = code.as_str();
        if text.is_empty() {
            return Err(String::from("an empty code in the table"));
        }
        if !text.contains('.') {
            return Err(format!("`{text}` is not `namespace.name`"));
        }
        if code.namespace() != namespace {
            return Err(format!(
                "`{text}` is in namespace `{}`, but the table is `{namespace}`",
                code.namespace()
            ));
        }
    }

    Ok(())
}
