use smol_str::SmolStr;
use winnow::{
    Parser,
    ascii::{space0, space1},
    combinator::preceded,
    error::ErrMode,
    token::take_while,
};

use crate::assembler::{
    Line, comments,
    diagnostic::{
        AsmParseErrors, Location, PResult, ParseError, ParseErrorKind, SyntaxError, cut_at,
        into_syntax, snippet, token_snippet,
    },
    ident,
    text::quoted_string,
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
    /// The author's description from `.description`. It is metadata only:
    /// `generate_image` never reads it and nothing about it reaches the image.
    pub description: Option<String>,
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
    Description,
    Rig(SmolStr),
    Group(GroupDef),
    Varf { name: SmolStr, value: f32 },
    VarI { name: SmolStr, value: i32 },
    Text,
}

fn name_arg<'i>(input: &mut Line<'i>) -> PResult<SmolStr> {
    ident.map(SmolStr::new).parse_next(input)
}

/// One value token: everything up to whitespace or a comment.
///
/// Parsing the lexeme first and converting second is what keeps `.clock hz = 1.5`
/// from being read as `1` followed by the trailing input `.5` — the error then
/// says the value is not a `u32`, which is what the author wrote wrong.
fn token<'i>(input: &mut Line<'i>) -> PResult<&'i str> {
    take_while(1.., |c: char| {
        !matches!(c, ' ' | '\t' | '\r' | '\n' | ',' | '#')
    })
    .parse_next(input)
}

fn number<'i, T>(input: &mut Line<'i>) -> PResult<T>
where
    T: std::str::FromStr,
{
    let text = token(input)?;
    text.parse()
        .map_err(|_| ErrMode::Backtrack(SyntaxError::at(input, ParseErrorKind::Unexpected)))
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
        // A `Cut` is the parser saying "this is not a value-shaped token at all"
        // and it already has a precise kind (`UnterminatedString`); re-kinding it
        // as `InvalidFieldValue` would lose that.
        Err(ErrMode::Cut(error)) => Err(ErrMode::Cut(error)),
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
            HeaderStmt::Layer(argument(input, "layer", "a u16", number::<u16>)?)
        }
        "clock" => {
            space_after(input, "clock")?;
            let _ = argument(input, "clock", "`hz`", "hz")?;
            expect_eq(input, "clock")?;
            HeaderStmt::Clock {
                hz: argument(input, "clock", "a u32", number::<u32>)?,
            }
        }
        "fuel" => {
            space_after(input, "fuel")?;
            HeaderStmt::Fuel(argument(input, "fuel", "a u32", number::<u32>)?)
        }
        "max_wait" => {
            space_after(input, "max_wait")?;
            HeaderStmt::MaxWait(argument(input, "max_wait", "an f32", number::<f32>)?)
        }
        "description" => {
            let rest = input.trim_start();
            if !rest.is_empty() && !rest.starts_with('#') {
                return cut_at(
                    rest,
                    ParseErrorKind::InvalidFieldValue {
                        directive: "description",
                        expected: "a newline followed by `\\\\` lines",
                        found: token_snippet(rest),
                    },
                );
            }
            HeaderStmt::Description
        }
        "rig" => {
            space_after(input, "rig")?;
            HeaderStmt::Rig(argument(
                input,
                "rig",
                "a quoted manifest path",
                quoted_string,
            )?)
        }
        "group" => HeaderStmt::Group(group(input)?),
        "var_f" => {
            space_after(input, "var_f")?;
            let name = argument(input, "var_f", "a name", name_arg)?;
            expect_eq(input, "var_f")?;
            let value = argument(input, "var_f", "an f32", number::<f32>)?;
            HeaderStmt::Varf { name, value }
        }
        "var_i" => {
            space_after(input, "var_i")?;
            let name = argument(input, "var_i", "a name", name_arg)?;
            expect_eq(input, "var_i")?;
            let value = argument(input, "var_i", "an i32", number::<i32>)?;
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

/// One `.description` line: optional horizontal whitespace, then exactly `\\`.
/// Returns the 1-based character column of the marker and the content after it.
fn description_line(line: &str) -> Option<(usize, &str)> {
    let indent = line
        .char_indices()
        .find(|(_, character)| !matches!(character, ' ' | '\t'))
        .map_or(line.len(), |(offset, _)| offset);
    let content = line[indent..].strip_prefix("\\\\")?;
    Some((line[..indent].chars().count() + 1, content))
}

/// Consume the lines of one `.description` block. `index` points at the first
/// line after the directive, and is advanced past every line that belongs to
/// the block; `consumed` follows the same lines for the shared cursor.
///
/// Misaligned lines are reported but still collected, so one block can report
/// every independent mistake. A line that is not a `\\` line ends the block.
fn description_block(
    lines: &[&str],
    index: &mut usize,
    consumed: &mut usize,
    errors: &mut Vec<ParseError>,
) -> (String, usize) {
    let mut value = String::new();
    let mut expected_column = None;
    let mut count = 0usize;

    while let Some(raw_line) = lines.get(*index) {
        let line = raw_line.trim_end_matches('\n').trim_end_matches('\r');
        let Some((column, content)) = description_line(line) else {
            break;
        };

        match expected_column {
            None => expected_column = Some(column),
            Some(expected) if expected != column => errors.push(ParseError::new(
                Location::new(*index + 1, column),
                ParseErrorKind::DescriptionMisaligned {
                    expected,
                    found: column,
                },
            )),
            Some(_) => {}
        }

        if count > 0 {
            value.push('\n');
        }
        value.push_str(content);

        count += 1;
        *consumed += raw_line.len();
        *index += 1;
    }

    (value, count)
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
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut consumed = 0usize;

    let mut parsing_fixed_header = ParsingFixedHeader::default();
    let mut description: Option<String> = None;
    let mut rig: Option<SmolStr> = None;
    let mut muscle_groups: Vec<GroupDef> = Vec::new();
    let mut vars_f: Vec<(SmolStr, f32)> = Vec::new();
    let mut vars_i: Vec<(SmolStr, i32)> = Vec::new();
    let mut errors = Vec::new();
    let mut header_end_line = None;

    let mut index = 0usize;
    while index < lines.len() {
        let raw_line = lines[index];
        let line_no = index + 1;
        let trimmed = raw_line.trim();

        // Blank and comment-only lines are not statements; skip them.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            consumed += raw_line.len();
            index += 1;
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
                index += 1;

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
                    HeaderStmt::Description => {
                        let (block, lines_read) =
                            description_block(&lines, &mut index, &mut consumed, &mut errors);
                        if description.is_some() {
                            errors.push(ParseError::new(
                                location,
                                ParseErrorKind::DuplicateField("description"),
                            ));
                        } else if lines_read == 0 {
                            errors.push(ParseError::new(
                                location,
                                ParseErrorKind::MissingDescription,
                            ));
                        } else {
                            description = Some(block);
                        }
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
                index += 1;
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
                    description,
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
