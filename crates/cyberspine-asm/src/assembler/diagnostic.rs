//! Located, typed diagnostics for the assembler's parse stage.
//!
//! The parser has two jobs that are easy to conflate: deciding whether a source
//! is legal, and telling the author *which* rule rejected *which* characters.
//! This module owns the second job. Every error carries a stable
//! [`ParseErrorKind`] (so callers can match on it) plus a [`Location`] (so a
//! human can find it). Rendering is delegated to `annotate-snippets`, which
//! produces the familiar `error:` / source-line / caret block.
//!
//! A source line is parsed from a bare `&str`; the byte offset is therefore kept
//! as "bytes remaining in the line" (`SyntaxError::remaining`). That value is
//! invariant under slicing, so an error raised deep inside an operand parser can
//! be located correctly by the outer line loop.

use std::fmt;

use cyberspine_diag::{Anchor, Diagnostic, Site};
use smol_str::SmolStr;
use thiserror::Error;
use winnow::error::{ErrMode, ParserError};

/// A 1-based source position. `line` counts `\n`, `column` counts characters.
///
/// The shared type, not a copy of it: `cyberspine_diag` owns the vocabulary, and a
/// warning from the lint pass has to be locatable exactly like an error from the
/// parser.
pub use cyberspine_diag::Location;

/// Why a source was rejected.
///
/// The variants are deliberately about *source rules*, not about winnow's parse
/// tree. A caller that wants to recover from one class of error can match on
/// this enum instead of re-parsing the message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseErrorKind {
    // -- directives ---------------------------------------------------------
    #[error("unknown directive `.{0}`")]
    UnknownDirective(SmolStr),
    #[error("expected a directive name after `.`, found `{found}`")]
    MissingDirectiveName { found: SmolStr },
    #[error("directive `.{0}` is only allowed before the first instruction")]
    LateDirective(SmolStr),
    #[error("`.{0}` is declared more than once")]
    DuplicateField(&'static str),
    #[error("`.{0}` is missing from the fixed header")]
    MissingField(&'static str),
    #[error("`.{directive}` needs {expected}")]
    MissingFieldValue {
        directive: &'static str,
        expected: &'static str,
    },
    #[error("`.{directive}` needs {expected}, found `{found}`")]
    InvalidFieldValue {
        directive: &'static str,
        expected: &'static str,
        found: SmolStr,
    },
    #[error("`.description` needs at least one line starting with `\\\\`")]
    MissingDescription,
    #[error(
        "`.description` lines must align their `\\\\`: expected column {expected}, found column {found}"
    )]
    DescriptionMisaligned { expected: usize, found: usize },

    // -- instructions -------------------------------------------------------
    #[error("a string literal is not closed before the end of the line")]
    UnterminatedString,
    #[error("unknown mnemonic `{0}`")]
    UnknownMnemonic(SmolStr),
    #[error("`{mnemonic}` is missing operand {index} of {arity}")]
    MissingOperand {
        mnemonic: &'static str,
        index: usize,
        arity: usize,
    },
    #[error("expected a `,` before operand {index} of `{mnemonic}`")]
    MissingComma {
        mnemonic: &'static str,
        index: usize,
    },
    #[error("operand {index} of `{mnemonic}` must be {expected}, found `{found}`")]
    InvalidOperand {
        mnemonic: &'static str,
        index: usize,
        expected: &'static str,
        found: SmolStr,
    },
    #[error("register `{prefix}{index}` is out of range; the file is `{prefix}0`..`{prefix}63`")]
    RegisterOutOfRange { prefix: char, index: u32 },
    #[error("expected a register from the {expected} file, found one from the {found} file")]
    RegisterFileMismatch {
        expected: &'static str,
        found: &'static str,
    },

    // -- line structure -----------------------------------------------------
    #[error("expected an instruction, a label, or a comment, found `{0}`")]
    ExpectedInstruction(SmolStr),
    #[error("unexpected trailing input `{0}`")]
    TrailingTokens(SmolStr),
    #[error("`{0}` is a reserved name and cannot be a label or a declared name")]
    ReservedName(SmolStr),
    /// Only produced when winnow reports an error without a more specific rule
    /// (for example an integer parser rejecting an overflowing literal).
    #[error("syntax error")]
    Unexpected,
}

/// A [`ParseErrorKind`] pinned to a source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub location: Location,
    pub kind: ParseErrorKind,
}

impl ParseError {
    pub fn new(location: Location, kind: ParseErrorKind) -> Self {
        Self { location, kind }
    }
}

/// The one way a source location becomes an anchor: the artifact is the file the
/// caller read, and the site is the location the parser recorded.
pub(crate) fn anchor(file: &str, location: Location) -> Anchor {
    Anchor::new(file, Site::from(location))
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}:{}: {}",
            self.location.line, self.location.column, self.kind
        )
    }
}

impl std::error::Error for ParseError {}

cyberspine_diag::codes! {
    ParseErrorKind: Error {
        Self::UnknownDirective(_) => "asm.unknown-directive",
        Self::MissingDirectiveName { .. } => "asm.bad-directive",
        Self::LateDirective(_) => "asm.late-directive",
        Self::DuplicateField(_) => "asm.duplicate-field",
        Self::MissingField(_) => "asm.missing-field",
        Self::MissingFieldValue { .. } => "asm.bad-field",
        Self::InvalidFieldValue { .. } => "asm.bad-field",
        Self::MissingDescription => "asm.bad-description",
        Self::DescriptionMisaligned { .. } => "asm.bad-description",
        Self::UnterminatedString => "asm.bad-literal",
        Self::UnknownMnemonic(_) => "asm.unknown-mnemonic",
        Self::MissingOperand { .. } => "asm.bad-operand",
        Self::MissingComma { .. } => "asm.bad-operand",
        Self::InvalidOperand { .. } => "asm.bad-operand",
        Self::RegisterOutOfRange { .. } => "asm.bad-register",
        Self::RegisterFileMismatch { .. } => "asm.bad-register",
        Self::ExpectedInstruction(_) => "asm.bad-statement",
        Self::TrailingTokens(_) => "asm.bad-statement",
        Self::ReservedName(_) => "asm.reserved-name",
        Self::Unexpected => "asm.syntax",
    }
}

/// Everything the parse stage rejected in one source file.
///
/// Parsing recovers at line boundaries, so a source with several independent
/// mistakes reports all of them instead of stopping at the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmParseErrors {
    pub errors: Vec<ParseError>,
}

impl AsmParseErrors {
    pub fn new(errors: Vec<ParseError>) -> Self {
        Self { errors }
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Every error as a diagnostic, located in `file`.
    ///
    /// Rendering is not this crate's job: this is the one conversion, and the
    /// report crate turns the result into a caret block or a JSON line.
    pub fn diagnostics(&self, file: &str) -> Vec<Diagnostic> {
        self.errors
            .iter()
            .map(|error| Diagnostic::of(&error.kind).at(anchor(file, error.location)))
            .collect()
    }
}

impl fmt::Display for AsmParseErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AsmParseErrors {}

/// The error type used *inside* winnow parsers.
///
/// It is crate-private because it exposes `remaining`, a parser implementation
/// detail. The outer line loop turns it into a public [`ParseError`] with a line
/// number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub(crate) remaining: usize,
    pub(crate) kind: ParseErrorKind,
}

impl SyntaxError {
    pub(crate) fn at(remaining: &str, kind: ParseErrorKind) -> Self {
        Self {
            remaining: remaining.len(),
            kind,
        }
    }

    /// Pin this error to `line` (which must be the *whole* line the parser was
    /// given) and `line_no` (1-based).
    pub(crate) fn located(self, line: &str, line_no: usize) -> ParseError {
        // `remaining` is always the length of a suffix of `line`, so the
        // subtraction lands on a character boundary. `get` is defensive: even a
        // future misuse falls back to column 1 instead of panicking on input.
        let remaining = self.remaining.min(line.len());
        let consumed = line.len() - remaining;
        let column = line
            .get(..consumed)
            .map_or(1, |prefix| prefix.chars().count() + 1);
        ParseError::new(Location::new(line_no, column), self.kind)
    }
}

impl<'i> ParserError<&'i str> for SyntaxError {
    type Inner = Self;

    fn from_input(input: &&'i str) -> Self {
        Self {
            remaining: input.len(),
            kind: ParseErrorKind::Unexpected,
        }
    }

    fn into_inner(self) -> Result<Self::Inner, Self> {
        Ok(self)
    }
}

pub(crate) type PResult<O> = Result<O, ErrMode<SyntaxError>>;

/// Cut the current line and report `kind` at `remaining`.
pub(crate) fn cut_at<O>(remaining: &str, kind: ParseErrorKind) -> PResult<O> {
    Err(ErrMode::Cut(SyntaxError::at(remaining, kind)))
}

/// Strip winnow's backtrack/cut wrapper. `Incomplete` cannot occur for a
/// complete `&str`, but mapping it to a generic error keeps this function total.
pub(crate) fn into_syntax(err: ErrMode<SyntaxError>) -> SyntaxError {
    match err {
        ErrMode::Backtrack(error) | ErrMode::Cut(error) => error,
        ErrMode::Incomplete(_) => SyntaxError::at("", ParseErrorKind::Unexpected),
    }
}

/// The beginning of the remaining input, capped so one bad line cannot produce
/// a screen-wide message.
pub(crate) fn snippet(input: &str) -> SmolStr {
    let trimmed = input.trim_start();
    let mut out = String::new();
    for (index, c) in trimmed.chars().enumerate() {
        if index == 32 {
            out.push('…');
            break;
        }
        out.push(c);
    }
    SmolStr::new(out)
}

/// One token's worth of the remaining input, capped like [`snippet`].
pub(crate) fn token_snippet(input: &str) -> SmolStr {
    let trimmed = input.trim_start();
    let end = trimmed
        .find(|c: char| c.is_whitespace() || c == ',' || c == '#')
        .unwrap_or(trimmed.len());
    snippet(&trimmed[..end])
}
