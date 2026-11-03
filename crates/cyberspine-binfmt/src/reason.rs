//! Reason templates: the text an `INTR` hands to the host (`docs/isa.md` §4.8).
//!
//! A template is UTF-8 text with placeholders that name a register or an
//! instance variable. The assembler stores the canonical form — variables by
//! index, not by name — and the host renders it against a frozen instance.
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

/// How a placeholder names a variable: by index in the stored form, by name in
/// the source form (`docs/asm.md` §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarRef {
    Index(u32),
    Name(String),
}

/// The value a placeholder reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    F(u8),
    I(u8),
    Vf(VarRef),
    Vi(VarRef),
}

/// One piece of a template: literal text or a value to interpolate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Value { value: Value, precision: Option<u8> },
}

/// A parsed reason template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    segments: Vec<Segment>,
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

impl Template {
    /// Parse a source or stored template.
    ///
    /// The two forms differ only in how a variable is named; the grammar around
    /// them is identical, so one parser covers both and the caller decides
    /// whether names are acceptable ([`Template::check_stored`] says no).
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
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
                segments.push(parse_placeholder(&text[index + 1..close])?);
                index = close + 1;
                run_start = index;
            } else if rest.starts_with('}') {
                return Err(TemplateError::UnmatchedBrace { at: index });
            } else {
                index += 1;
            }
        }
        push_text(&mut segments, &text[run_start..]);

        Ok(Self { segments })
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Resolve every variable name through the two tables.
    ///
    /// The assembler calls this once per `INTR`; the stored image therefore
    /// never carries a name.
    pub fn resolve_vars(
        &mut self,
        var_f: impl Fn(&str) -> Option<u32>,
        var_i: impl Fn(&str) -> Option<u32>,
    ) -> Result<(), TemplateError> {
        for segment in &mut self.segments {
            let Segment::Value { value, .. } = segment else {
                continue;
            };
            match value {
                Value::Vf(VarRef::Name(name)) => {
                    let index = var_f(name)
                        .ok_or_else(|| TemplateError::UnknownVar { name: name.clone() })?;
                    *value = Value::Vf(VarRef::Index(index));
                }
                Value::Vi(VarRef::Name(name)) => {
                    let index = var_i(name)
                        .ok_or_else(|| TemplateError::UnknownVar { name: name.clone() })?;
                    *value = Value::Vi(VarRef::Index(index));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The checks the loader applies to a stored template: no names, and every
    /// index inside the table it addresses.
    pub fn check_stored(&self, var_f: u32, var_i: u32) -> Result<(), TemplateError> {
        for segment in &self.segments {
            let Segment::Value { value, .. } = segment else {
                continue;
            };
            match value {
                Value::Vf(VarRef::Name(name)) | Value::Vi(VarRef::Name(name)) => {
                    return Err(TemplateError::NameInStored { name: name.clone() });
                }
                Value::Vf(VarRef::Index(index)) if *index >= var_f => {
                    return Err(TemplateError::StoredVarIndexOutOfRange {
                        index: *index,
                        count: var_f,
                    });
                }
                Value::Vi(VarRef::Index(index)) if *index >= var_i => {
                    return Err(TemplateError::StoredVarIndexOutOfRange {
                        index: *index,
                        count: var_i,
                    });
                }
                _ => {}
            }
        }
        Ok(())
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
                        Value::Vf(VarRef::Index(var)) => out.push_str(&format!("vf:{var}")),
                        Value::Vi(VarRef::Index(var)) => out.push_str(&format!("vi:{var}")),
                        Value::Vf(VarRef::Name(name)) => {
                            out.push_str("vf:");
                            out.push_str(name);
                        }
                        Value::Vi(VarRef::Name(name)) => {
                            out.push_str("vi:");
                            out.push_str(name);
                        }
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
                    Value::Vf(VarRef::Index(var)) => match view.vf(*var) {
                        Some(value) => push_float(&mut out, value, *precision),
                        None => out.push_str("<missing>"),
                    },
                    Value::Vi(VarRef::Index(var)) => match view.vi(*var) {
                        Some(value) => out.push_str(&format!("{value}")),
                        None => out.push_str("<missing>"),
                    },
                    Value::Vf(VarRef::Name(name)) | Value::Vi(VarRef::Name(name)) => {
                        out.push_str("<unresolved:");
                        out.push_str(name);
                        out.push('>');
                    }
                },
            }
        }
        out
    }
}

fn push_text(segments: &mut Vec<Segment>, text: &str) {
    if !text.is_empty() {
        segments.push(Segment::Text(text.to_string()));
    }
}

fn parse_placeholder(inner: &str) -> Result<Segment, TemplateError> {
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

fn parse_var(text: &str) -> Result<VarRef, TemplateError> {
    if text.is_empty() {
        return Err(TemplateError::VarNameInvalid {
            text: text.to_string(),
        });
    }
    if text.bytes().all(|byte| byte.is_ascii_digit()) {
        return text.parse::<u32>().map(VarRef::Index).map_err(|_| {
            TemplateError::VarIndexTooLarge {
                text: text.to_string(),
            }
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
    Ok(VarRef::Name(text.to_string()))
}

fn push_float(out: &mut String, value: f32, precision: Option<u8>) {
    match precision {
        Some(precision) => out.push_str(&format!("{:.*}", usize::from(precision), value)),
        None => out.push_str(&format!("{value}")),
    }
}
