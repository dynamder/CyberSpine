//! Reason templates: the text an `INTR` hands to the host (`docs/isa.md` §4.8).
//!
//! A template is UTF-8 text with placeholders that name a register or an
//! instance variable. It comes in two forms and the form is the type parameter:
//! a [`SourceTemplate`] names its variables (`{vf:phase}`) and is what a source
//! file writes, a [`StoredTemplate`] indexes them (`{vf:3}`) and is what an
//! image stores. Neither can be built in the other's shape — a stored template
//! cannot hold a name, and a source template refuses an index at parse time —
//! so the assembler cannot leak a name into an image and the loader cannot
//! render a template it has not range-checked.
//!
//! The grammar is deliberately tiny: a placeholder is a *value*, never an
//! expression. Arithmetic belongs in the program, before the `INTR`.
//!
//! ```text
//! {f12}  {i3}  {vf:phase}  {vi:state}  {f3:.2}  {{  }}
//! ```

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The longest reason template the format accepts, in bytes.
///
/// A rendered reason is bounded by this plus a handful of formatted values per
/// placeholder, so it always fits one arbiter event frame (`docs/backend.md`
/// caps a frame at 1 MiB).
pub const MAX_REASON_LEN: usize = 4096;

/// A template in the form a source file writes: variables by name.
pub type SourceTemplate = Template<String>;

/// A template in the form an image stores: variables by index.
pub type StoredTemplate = Template<u32>;

/// A parsed reason template; `Var` is how it refers to an instance variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template<Var> {
    segments: Vec<Segment<Var>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment<Var> {
    Text(String),
    Value {
        value: Value<Var>,
        precision: Option<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value<Var> {
    F(u8),
    I(u8),
    Vf(Var),
    Vi(Var),
}

/// Everything a template can get wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("a reason template cannot be empty")]
    Empty,
    #[error("the reason template is {len} bytes, above the {MAX_REASON_LEN}-byte limit")]
    TooLong { len: usize },
    #[error("the `{{` at byte {at} is not closed")]
    UnclosedBrace { at: usize },
    #[error("the `}}` at byte {at} is not an escaped brace")]
    UnmatchedBrace { at: usize },
    #[error("`{{{text}}}` is not a placeholder")]
    UnknownPlaceholder { text: String },
    #[error("register `{text}` is not in 0..=63")]
    RegisterOutOfRange { text: String },
    #[error("variable index `{text}` does not fit a u32")]
    VarIndexTooLarge { text: String },
    #[error("`{text}` is not a variable name")]
    VarNameInvalid { text: String },
    #[error("precision `{text}` is not in 0..=9")]
    BadPrecision { text: String },
    #[error("precision is only defined for floating-point values")]
    PrecisionOnInteger,
    #[error("a stored template names variables by index, not `{name}`")]
    NameInStored { name: String },
    #[error("a source template names variables, not index `{text}`")]
    IndexInSource { text: String },
    #[error("stored variable index {index} is out of range (count {count})")]
    StoredVarIndexOutOfRange { index: u32, count: u32 },
    #[error("variable `{name}` is not declared")]
    UnknownVar { name: String },
}

/// What the host reads out of one frozen instance to render a template.
///
/// `vf` / `vi` return `None` when the index is out of range. A verified image
/// cannot contain such an index (the loader checks every template), so the
/// renderer treats `None` as a diagnostic rather than as a state of its own.
pub trait ReasonView {
    fn f(&self, reg: u8) -> f32;
    fn i(&self, reg: u8) -> i32;
    fn vf(&self, var: u32) -> Option<f32>;
    fn vi(&self, var: u32) -> Option<i32>;
}

impl Template<String> {
    /// Parse the form a source file writes.
    ///
    /// A variable is named here; an index is [`TemplateError::IndexInSource`],
    /// because only [`Template::resolve_vars`] may decide what an index means.
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        Ok(Self {
            segments: parse_segments(text, parse_source_var)?,
        })
    }

    /// Rewrite every variable name through the two tables, producing the form
    /// the image stores.
    ///
    /// The assembler calls this once per `INTR`; the stored image therefore
    /// never carries a name.
    pub fn resolve_vars(
        self,
        var_f: impl Fn(&str) -> Option<u32>,
        var_i: impl Fn(&str) -> Option<u32>,
    ) -> Result<StoredTemplate, TemplateError> {
        let mut segments = Vec::with_capacity(self.segments.len());
        for segment in self.segments {
            segments.push(match segment {
                Segment::Text(text) => Segment::Text(text),
                Segment::Value { value, precision } => Segment::Value {
                    value: match value {
                        Value::F(reg) => Value::F(reg),
                        Value::I(reg) => Value::I(reg),
                        Value::Vf(name) => {
                            Value::Vf(var_f(&name).ok_or(TemplateError::UnknownVar { name })?)
                        }
                        Value::Vi(name) => {
                            Value::Vi(var_i(&name).ok_or(TemplateError::UnknownVar { name })?)
                        }
                    },
                    precision,
                },
            });
        }
        Ok(Template { segments })
    }
}

impl Template<u32> {
    /// Parse the form an image stores and check every index against its table.
    ///
    /// Both checks a reader cares about happen here: a name is
    /// [`TemplateError::NameInStored`], an index past its table is
    /// [`TemplateError::StoredVarIndexOutOfRange`]. The result is therefore a
    /// template the renderer cannot find out of range.
    pub fn parse(text: &str, var_f: u32, var_i: u32) -> Result<Self, TemplateError> {
        let template = Self {
            segments: parse_segments(text, parse_stored_var)?,
        };
        template.check_indices(var_f, var_i)?;
        Ok(template)
    }

    /// The canonical stored form: variables by index, braces escaped.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Text(text) => {
                    for character in text.chars() {
                        match character {
                            '{' => out.push_str("{{"),
                            '}' => out.push_str("}}"),
                            other => out.push(other),
                        }
                    }
                }
                Segment::Value { value, precision } => {
                    out.push('{');
                    match value {
                        Value::F(reg) => out.push_str(&format!("f{reg}")),
                        Value::I(reg) => out.push_str(&format!("i{reg}")),
                        Value::Vf(var) => out.push_str(&format!("vf:{var}")),
                        Value::Vi(var) => out.push_str(&format!("vi:{var}")),
                    }
                    if let Some(precision) = precision {
                        out.push_str(&format!(":.{precision}"));
                    }
                    out.push('}');
                }
            }
        }
        out
    }

    /// Interpolate the template against one frozen instance.
    ///
    /// Infallible on purpose: the value was checked when the image loaded, and
    /// a missing variable is rendered as a diagnostic marker rather than
    /// panicking inside `Session::tick`.
    pub fn render(&self, view: &dyn ReasonView) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Text(text) => out.push_str(text),
                Segment::Value { value, precision } => match value {
                    Value::F(reg) => push_float(&mut out, view.f(*reg), *precision),
                    Value::I(reg) => out.push_str(&format!("{}", view.i(*reg))),
                    Value::Vf(var) => match view.vf(*var) {
                        Some(value) => push_float(&mut out, value, *precision),
                        None => out.push_str("<missing>"),
                    },
                    Value::Vi(var) => match view.vi(*var) {
                        Some(value) => out.push_str(&format!("{value}")),
                        None => out.push_str("<missing>"),
                    },
                },
            }
        }
        out
    }

    fn check_indices(&self, var_f: u32, var_i: u32) -> Result<(), TemplateError> {
        for segment in &self.segments {
            let Segment::Value { value, .. } = segment else {
                continue;
            };
            match value {
                Value::Vf(var) if *var >= var_f => {
                    return Err(TemplateError::StoredVarIndexOutOfRange {
                        index: *var,
                        count: var_f,
                    });
                }
                Value::Vi(var) if *var >= var_i => {
                    return Err(TemplateError::StoredVarIndexOutOfRange {
                        index: *var,
                        count: var_i,
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn push_text<Var>(segments: &mut Vec<Segment<Var>>, text: &str) {
    if !text.is_empty() {
        segments.push(Segment::Text(text.to_string()));
    }
}

fn parse_segments<Var>(
    text: &str,
    parse_var: fn(&str) -> Result<Var, TemplateError>,
) -> Result<Vec<Segment<Var>>, TemplateError> {
    if text.is_empty() {
        return Err(TemplateError::Empty);
    }
    if text.len() > MAX_REASON_LEN {
        return Err(TemplateError::TooLong { len: text.len() });
    }

    let mut segments = Vec::new();
    let mut run_start = 0usize;
    let mut index = 0usize;

    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with("{{") {
            push_text(&mut segments, &text[run_start..index]);
            segments.push(Segment::Text("{".to_string()));
            index += 2;
            run_start = index;
        } else if rest.starts_with("}}") {
            push_text(&mut segments, &text[run_start..index]);
            segments.push(Segment::Text("}".to_string()));
            index += 2;
            run_start = index;
        } else if let Some(after_brace) = rest.strip_prefix('{') {
            push_text(&mut segments, &text[run_start..index]);
            let offset = after_brace
                .find('}')
                .ok_or(TemplateError::UnclosedBrace { at: index })?;
            let close = index + 1 + offset;
            segments.push(parse_placeholder(&text[index + 1..close], parse_var)?);
            index = close + 1;
            run_start = index;
        } else if rest.starts_with('}') {
            return Err(TemplateError::UnmatchedBrace { at: index });
        } else {
            let character = rest.chars().next().expect("index < text.len()");
            index += character.len_utf8();
        }
    }
    push_text(&mut segments, &text[run_start..]);

    Ok(segments)
}

fn parse_placeholder<Var>(
    inner: &str,
    parse_var: fn(&str) -> Result<Var, TemplateError>,
) -> Result<Segment<Var>, TemplateError> {
    if inner.is_empty() {
        return Err(TemplateError::UnknownPlaceholder {
            text: String::new(),
        });
    }

    let (head, precision) = split_precision(inner)?;
    let value = if let Some(rest) = head.strip_prefix("vf:") {
        Value::Vf(parse_var(rest)?)
    } else if let Some(rest) = head.strip_prefix("vi:") {
        Value::Vi(parse_var(rest)?)
    } else if let Some(rest) = head.strip_prefix('f') {
        Value::F(parse_reg(rest)?)
    } else if let Some(rest) = head.strip_prefix('i') {
        Value::I(parse_reg(rest)?)
    } else {
        return Err(TemplateError::UnknownPlaceholder {
            text: inner.to_string(),
        });
    };

    if precision.is_some() && matches!(value, Value::I(_) | Value::Vi(_)) {
        return Err(TemplateError::PrecisionOnInteger);
    }

    Ok(Segment::Value { value, precision })
}

/// Split `name` into the value part and an optional `:.N` precision suffix.
fn split_precision(inner: &str) -> Result<(&str, Option<u8>), TemplateError> {
    let Some(at) = inner.find(":.") else {
        return Ok((inner, None));
    };
    let head = &inner[..at];
    let text = &inner[at + 2..];
    let precision = text
        .parse::<u8>()
        .ok()
        .filter(|value| *value <= 9)
        .ok_or_else(|| TemplateError::BadPrecision {
            text: text.to_string(),
        })?;
    Ok((head, Some(precision)))
}

fn parse_reg(text: &str) -> Result<u8, TemplateError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TemplateError::RegisterOutOfRange {
            text: text.to_string(),
        });
    }
    text.parse::<u8>()
        .ok()
        .filter(|reg| *reg < 64)
        .ok_or_else(|| TemplateError::RegisterOutOfRange {
            text: text.to_string(),
        })
}

/// A variable in a source template: an identifier, never a number.
fn parse_source_var(text: &str) -> Result<String, TemplateError> {
    if text.is_empty() {
        return Err(TemplateError::VarNameInvalid {
            text: String::new(),
        });
    }
    if text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TemplateError::IndexInSource {
            text: text.to_string(),
        });
    }
    let mut bytes = text.bytes();
    let first = bytes.next().expect("checked non-empty");
    if !(first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(TemplateError::VarNameInvalid {
            text: text.to_string(),
        });
    }
    Ok(text.to_string())
}

/// A variable in a stored template: an index into the table it addresses.
fn parse_stored_var(text: &str) -> Result<u32, TemplateError> {
    if text.is_empty() {
        return Err(TemplateError::VarNameInvalid {
            text: text.to_string(),
        });
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TemplateError::NameInStored {
            name: text.to_string(),
        });
    }
    text.parse::<u32>()
        .map_err(|_| TemplateError::VarIndexTooLarge {
            text: text.to_string(),
        })
}

fn push_float(out: &mut String, value: f32, precision: Option<u8>) {
    match precision {
        Some(precision) => out.push_str(&format!("{:.*}", usize::from(precision), value)),
        None => out.push_str(&format!("{value}")),
    }
}
