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

use std::{fmt, ops::Range, path::Path};

use annotate_snippets::{AnnotationKind, Level, Origin, Renderer, Snippet, renderer::DecorStyle};
use smol_str::SmolStr;
use thiserror::Error;
use winnow::error::{ErrMode, ParserError};

/// A 1-based source position. `line` counts `\n`, `column` counts characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

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

    /// Render this error against `source` with a source line and a caret.
    pub fn render(&self, path: &Path, source: &str) -> String {
        render_errors(std::slice::from_ref(self), path, source)
    }
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

    /// Render every error against `source`, separated by a blank line.
    pub fn render(&self, path: &Path, source: &str) -> String {
        let mut out = render_errors(&self.errors, path, source);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if self.errors.len() > 1 {
            out.push('\n');
            out.push_str(&format!("{} errors emitted\n", self.errors.len()));
        }
        out
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

/// Byte offset of `location` in `source`; `None` when the line does not exist.
fn byte_offset(source: &str, location: Location) -> Option<usize> {
    let mut offset = 0usize;
    for (index, line) in source.split_inclusive('\n').enumerate() {
        if index + 1 == location.line {
            let chars = location.column.saturating_sub(1);
            let byte = line
                .char_indices()
                .nth(chars)
                .map_or(line.len(), |(index, _)| index);
            return Some(offset + byte);
        }
        offset += line.len();
    }
    None
}

/// Byte range to annotate. Normally one character; a missing token at the end of
/// a line becomes a zero-length span so the reported column is exact.
fn span_at(source: &str, location: Location) -> Option<Range<usize>> {
    let start = byte_offset(source, location)?;

    if start >= source.len() || matches!(source[start..].chars().next(), Some('\n' | '\r')) {
        return Some(start..start);
    }

    let end = start + source[start..].chars().next()?.len_utf8();
    Some(start..end)
}

fn render_errors(errors: &[ParseError], path: &Path, source: &str) -> String {
    let mut out = String::new();

    for (index, error) in errors.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&render_one(
            Level::ERROR,
            &error.kind.to_string(),
            error.location,
            path,
            source,
        ));
    }

    out
}

/// Render one diagnostic at `location`, with the source line and a caret.
///
/// Shared by the parse stage (which has a `ParseErrorKind`) and the generation
/// stage (which has an `ImageGenError` and, sometimes, no line at all — that case
/// is handled by the caller, not here).
pub fn render_error(title: &str, location: Location, path: &Path, source: &str) -> String {
    render_one(Level::ERROR, title, location, path, source)
}

/// A readability diagnostic: same rendering, warning level.
pub fn render_warning(title: &str, location: Location, path: &Path, source: &str) -> String {
    render_one(Level::WARNING, title, location, path, source)
}

fn render_one(level: Level, title: &str, location: Location, path: &Path, source: &str) -> String {
    let path = path.display().to_string();
    let renderer = Renderer::plain().decor_style(DecorStyle::Unicode);
    let title = level.primary_title(title);

    let group = match span_at(source, location) {
        Some(span) => title.element(
            Snippet::source(source)
                .path(path.clone())
                .line_start(1)
                .annotation(AnnotationKind::Primary.span(span)),
        ),
        // The line is past the end of the source (for example a missing header
        // field at EOF). Keep the file:line:column location even though there is
        // no line to quote.
        None => title.element(
            Origin::path(path)
                .line(location.line)
                .char_column(location.column),
        ),
    };

    let mut out = renderer.render(&[group]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
