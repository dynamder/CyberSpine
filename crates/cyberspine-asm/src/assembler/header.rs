use smol_str::SmolStr;
use winnow::{
    Parser,
    ascii::{dec_int, dec_uint, float, space0, space1},
    combinator::preceded,
    error::ErrMode,
};

use crate::assembler::{
    Line, comments,
    diagnostic::{
        AsmParseErrors, Location, PResult, ParseError, ParseErrorKind, SyntaxError, cut_at,
        into_syntax, snippet, token_snippet,
    },
    ident,
};

#[derive(Debug, Clone, PartialEq)]
pub struct GroupDef {
    pub name: SmolStr,
    pub members: Vec<SmolStr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixedHeader {
    pub layer: u16,
    pub clock_hz: u32,
    pub fuel: u32,
    pub max_wait: f32,
}

/// The fixed header while it is still being assembled.
///
/// A missing or repeated field is a diagnostic, not a panic: the parser keeps
/// the first value and reports every problem at the end of the declaration
/// section.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsingFixedHeader {
    layer: Option<u16>,
    clock_hz: Option<u32>,
    fuel: Option<u32>,
    max_wait: Option<f32>,
}

impl ParsingFixedHeader {
    pub fn new() -> Self {
        ParsingFixedHeader {
            layer: None,
            clock_hz: None,
            fuel: None,
            max_wait: None,
        }
    }

    pub fn layer(&mut self, value: u16) -> Result<(), ParseErrorKind> {
        if self.layer.is_some() {
            return Err(ParseErrorKind::DuplicateField("layer"));
        }

        self.layer = Some(value);
        Ok(())
    }

    pub fn clock_hz(&mut self, value: u32) -> Result<(), ParseErrorKind> {
        if self.clock_hz.is_some() {
            return Err(ParseErrorKind::DuplicateField("clock_hz"));
        }

        self.clock_hz = Some(value);
        Ok(())
    }

    pub fn fuel(&mut self, value: u32) -> Result<(), ParseErrorKind> {
        if self.fuel.is_some() {
            return Err(ParseErrorKind::DuplicateField("fuel"));
        }

        self.fuel = Some(value);
        Ok(())
    }

    pub fn max_wait(&mut self, value: f32) -> Result<(), ParseErrorKind> {
        if self.max_wait.is_some() {
            return Err(ParseErrorKind::DuplicateField("max_wait"));
        }

        self.max_wait = Some(value);
        Ok(())
    }

    /// Build the fixed header, or return **every** field that is still missing.
    ///
    /// `docs/asm.md` §3.1 requires all four scalars in v1; missing fields are
    /// reported together so one run shows the complete set.
    pub fn finish(self) -> Result<FixedHeader, Vec<ParseErrorKind>> {
        let mut missing = Vec::new();
        if self.layer.is_none() {
            missing.push(ParseErrorKind::MissingField("layer"));
        }
        if self.clock_hz.is_none() {
            missing.push(ParseErrorKind::MissingField("clock_hz"));
        }
        if self.fuel.is_none() {
            missing.push(ParseErrorKind::MissingField("fuel"));
        }
        if self.max_wait.is_none() {
            missing.push(ParseErrorKind::MissingField("max_wait"));
        }

        if !missing.is_empty() {
            return Err(missing);
        }

        match (self.layer, self.clock_hz, self.fuel, self.max_wait) {
            (Some(layer), Some(clock_hz), Some(fuel), Some(max_wait)) => Ok(FixedHeader {
                layer,
                clock_hz,
                fuel,
                max_wait,
            }),
            _ => Err(missing),
        }
    }
}

impl Default for ParsingFixedHeader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsmHeader {
    pub fixed: FixedHeader,
    /// The manifest path from `.rig`, resolved against the source file by the
    /// caller. `None` means the manifest has to come from `--rig`.
    pub rig: Option<SmolStr>,
    pub muscle_groups: Vec<GroupDef>,
    pub vars_f: Vec<(SmolStr, f32)>,
    pub vars_i: Vec<(SmolStr, i32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeaderStmt {
    Layer(u16),
    Clock { hz: u32 },
    Fuel(u32),
    MaxWait(f32),
    Rig(SmolStr),
    Group(GroupDef),
    Varf { name: SmolStr, value: f32 },
    VarI { name: SmolStr, value: i32 },
    Text,
}

/// A double-quoted string. Only `\\` and `\"` are escapes; a backslash before
/// anything else is reported, so a Windows path has to be written `\\`.
fn quoted_path<'i>(input: &mut Line<'i>) -> PResult<SmolStr> {
    let text = *input;
    let Some(rest) = text.strip_prefix('"') else {
        return cut_at(text, ParseErrorKind::Unexpected);
    };

    let mut out = String::new();
    let mut chars = rest.char_indices();
    while let Some((offset, character)) = chars.next() {
        match character {
            '"' => {
                *input = &rest[offset + 1..];
                return Ok(SmolStr::new(out));
            }
            '\\' => match chars.next() {
                Some((_, escaped @ ('\\' | '"'))) => out.push(escaped),
                _ => return cut_at(&rest[offset..], ParseErrorKind::Unexpected),
            },
            other => out.push(other),
        }
    }
    cut_at(text, ParseErrorKind::Unexpected)
}

fn name_arg<'i>(input: &mut Line<'i>) -> PResult<SmolStr> {
    ident.map(SmolStr::new).parse_next(input)
}

fn eq(input: &mut Line<'_>) -> PResult<()> {
    (space0, '=', space0).void().parse_next(input)
}

/// Parse one directive argument, turning a bare winnow failure into a located,
/// directive-aware diagnostic.
fn argument<'i, T, P>(
    input: &mut Line<'i>,
    directive: &'static str,
    expected: &'static str,
    mut parser: P,
) -> PResult<T>
where
    P: Parser<Line<'i>, T, ErrMode<SyntaxError>>,
{
    let start = *input;
    match parser.parse_next(input) {
        Ok(value) => Ok(value),
        Err(_) => {
            let kind = if start.trim_start().is_empty() {
                ParseErrorKind::MissingFieldValue {
                    directive,
                    expected,
                }
            } else {
                ParseErrorKind::InvalidFieldValue {
                    directive,
                    expected,
                    found: token_snippet(start),
                }
            };
            cut_at(start, kind)
        }
    }
}

fn space_after(input: &mut Line<'_>, directive: &'static str) -> PResult<()> {
    let start = *input;
    let parsed: PResult<&str> = space1.parse_next(input);
    match parsed {
        Ok(_) => Ok(()),
        Err(_) => {
            let kind = if start.trim_start().is_empty() {
                ParseErrorKind::MissingFieldValue {
                    directive,
                    expected: "a space before its value",
                }
            } else {
                ParseErrorKind::InvalidFieldValue {
                    directive,
                    expected: "a space before its value",
                    found: token_snippet(start),
                }
            };
            cut_at(start, kind)
        }
    }
}

fn expect_eq(input: &mut Line<'_>, directive: &'static str) -> PResult<()> {
    let start = *input;
    match eq.parse_next(input) {
        Ok(()) => Ok(()),
        Err(_) => {
            let kind = if start.trim_start().is_empty() {
                ParseErrorKind::MissingFieldValue {
                    directive,
                    expected: "`=`",
                }
            } else {
                ParseErrorKind::InvalidFieldValue {
                    directive,
                    expected: "`=`",
                    found: token_snippet(start),
                }
            };
            cut_at(start, kind)
        }
    }
}

fn header_stmt<'i>(input: &mut Line<'i>) -> PResult<HeaderStmt> {
    let start = *input;
    let dir = match preceded('.', ident).parse_next(input) {
        Ok(dir) => dir,
        Err(_) => {
            return cut_at(
                start,
                ParseErrorKind::MissingDirectiveName {
                    found: token_snippet(start),
                },
            );
        }
    };

    let stmt = match dir.to_ascii_lowercase().as_str() {
        "layer" => {
            space_after(input, "layer")?;
            HeaderStmt::Layer(argument(input, "layer", "a u16", dec_uint)?)
        }
        "clock" => {
            space_after(input, "clock")?;
            let _ = argument(input, "clock", "`hz`", "hz")?;
            expect_eq(input, "clock")?;
            HeaderStmt::Clock {
                hz: argument(input, "clock", "a u32", dec_uint)?,
            }
        }
        "fuel" => {
            space_after(input, "fuel")?;
            HeaderStmt::Fuel(argument(input, "fuel", "a u32", dec_uint)?)
        }
        "max_wait" => {
            space_after(input, "max_wait")?;
            HeaderStmt::MaxWait(argument(input, "max_wait", "an f32", float)?)
        }
        "rig" => {
            space_after(input, "rig")?;
            HeaderStmt::Rig(argument(
                input,
                "rig",
                "a quoted manifest path",
                quoted_path,
            )?)
        }
        "muscle" | "sensor_f" | "sensor_i" => {
            return cut_at(start, ParseErrorKind::RemovedDirective(SmolStr::new(dir)));
        }
        "group" => HeaderStmt::Group(group(input)?),
        "var_f" => {
            space_after(input, "var_f")?;
            let name = argument(input, "var_f", "a name", name_arg)?;
            expect_eq(input, "var_f")?;
            let value = argument(input, "var_f", "an f32", float)?;
            HeaderStmt::Varf { name, value }
        }
        "var_i" => {
            space_after(input, "var_i")?;
            let name = argument(input, "var_i", "a name", name_arg)?;
            expect_eq(input, "var_i")?;
            let value = argument(input, "var_i", "an i32", dec_int)?;
            HeaderStmt::VarI { name, value }
        }
        "text" => HeaderStmt::Text,
        _ => return cut_at(start, ParseErrorKind::UnknownDirective(SmolStr::new(dir))),
    };

    finish_header_line(input)?;
    Ok(stmt)
}

fn group<'i>(input: &mut Line<'i>) -> PResult<GroupDef> {
    space_after(input, "group")?;
    let name = argument(input, "group", "a group name", name_arg)?;
    expect_eq(input, "group")?;

    let first = argument(input, "group", "a muscle name", name_arg)?;
    let mut members = vec![first];

    // A manually driven loop (instead of `repeat`) keeps a dangling comma from
    // being silently accepted as "end of members".
    loop {
        let separator: PResult<()> = (space0, ',', space0).void().parse_next(input);
        if separator.is_err() {
            break;
        }
        members.push(argument(input, "group", "a muscle name", name_arg)?);
    }

    Ok(GroupDef { name, members })
}

fn finish_header_line(input: &mut Line<'_>) -> PResult<()> {
    space0.parse_next(input)?;

    if input.is_empty() {
        return Ok(());
    }
    if input.starts_with('#') {
        return comments.parse_next(input);
    }

    let start = *input;
    cut_at(start, ParseErrorKind::TrailingTokens(snippet(start)))
}

/// Parse one declaration line (leading whitespace included).
fn parse_header_line(input: &mut Line<'_>) -> PResult<HeaderStmt> {
    space0.parse_next(input)?;
    header_stmt(input)
}

/// Parses the declaration section and advances `*input` past it.
///
/// The cursor stops after the optional `.text` marker, or before the first line
/// that cannot be a declaration (an instruction or a label), so
/// [`crate::assembler::text::text`] can continue on the same cursor
/// (`docs/asm.md` §2 / §7).
///
/// Errors are collected line by line; a malformed declaration does not stop the
/// parser from checking the rest of the header and the instruction section.
pub fn header(input: &mut &str) -> Result<AsmHeader, AsmParseErrors> {
    let src = *input;
    let mut consumed = 0usize;

    let mut parsing_fixed_header = ParsingFixedHeader::default();
    let mut rig: Option<SmolStr> = None;
    let mut muscle_groups: Vec<GroupDef> = Vec::new();
    let mut vars_f: Vec<(SmolStr, f32)> = Vec::new();
    let mut vars_i: Vec<(SmolStr, i32)> = Vec::new();
    let mut errors = Vec::new();
    let mut header_end_line = None;

    for (index, raw_line) in src.split_inclusive('\n').enumerate() {
        let line_no = index + 1;
        let trimmed = raw_line.trim();

        // Blank and comment-only lines are not statements; skip them.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            consumed += raw_line.len();
            continue;
        }

        // The declaration section ends at the first instruction or label. That
        // line stays in the cursor for the text parser.
        if !trimmed.starts_with('.') {
            header_end_line = Some(line_no);
            break;
        }

        let line = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        let mut cursor = line;
        match parse_header_line(&mut cursor) {
            Ok(stmt) => {
                consumed += raw_line.len();

                if matches!(stmt, HeaderStmt::Text) {
                    header_end_line = Some(line_no);
                    break;
                }

                let location = Location::new(line_no, directive_column(line));
                match stmt {
                    HeaderStmt::Layer(value) => {
                        push_field(parsing_fixed_header.layer(value), location, &mut errors)
                    }
                    HeaderStmt::Clock { hz } => {
                        push_field(parsing_fixed_header.clock_hz(hz), location, &mut errors)
                    }
                    HeaderStmt::Fuel(value) => {
                        push_field(parsing_fixed_header.fuel(value), location, &mut errors)
                    }
                    HeaderStmt::MaxWait(value) => {
                        push_field(parsing_fixed_header.max_wait(value), location, &mut errors)
                    }
                    HeaderStmt::Rig(path) => {
                        let result = match rig.replace(path) {
                            Some(_) => Err(ParseErrorKind::DuplicateField("rig")),
                            None => Ok(()),
                        };
                        push_field(result, location, &mut errors);
                    }
                    HeaderStmt::Group(group_def) => muscle_groups.push(group_def),
                    HeaderStmt::Varf { name, value } => vars_f.push((name, value)),
                    HeaderStmt::VarI { name, value } => vars_i.push((name, value)),
                    // `.text` was handled above so it can end the loop before
                    // the declaration tables are updated.
                    HeaderStmt::Text => {}
                }
            }
            Err(err) => {
                consumed += raw_line.len();
                errors.push(into_syntax(err).located(line, line_no));
            }
        }
    }

    *input = &src[consumed..];

    let end_line = header_end_line.unwrap_or_else(|| src.lines().count().max(1));

    match parsing_fixed_header.finish() {
        Ok(fixed) => {
            if errors.is_empty() {
                Ok(AsmHeader {
                    fixed,
                    rig,
                    muscle_groups,
                    vars_f,
                    vars_i,
                })
            } else {
                sort_errors(&mut errors);
                Err(AsmParseErrors::new(errors))
            }
        }
        Err(missing) => {
            for kind in missing {
                errors.push(ParseError::new(Location::new(end_line, 1), kind));
            }
            sort_errors(&mut errors);
            Err(AsmParseErrors::new(errors))
        }
    }
}

fn push_field(
    result: Result<(), ParseErrorKind>,
    location: Location,
    errors: &mut Vec<ParseError>,
) {
    if let Err(kind) = result {
        errors.push(ParseError::new(location, kind));
    }
}

fn sort_errors(errors: &mut [ParseError]) {
    errors.sort_by_key(|error| (error.location.line, error.location.column));
}

fn directive_column(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count() + 1
}
