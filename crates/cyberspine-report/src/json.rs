use std::io::{self, Write};

use cyberspine_diag::{Diagnostic, Site};
use serde::Serialize;

pub const SCHEMA: u32 = cyberspine_diag::SCHEMA;

/// One diagnostic as one JSON line.
///
/// The field names are the contract: an agent matches on `code` and reads
/// `primary`/`labels` for places. `message` is prose and may change between
/// releases; `code` may not.
pub fn render(diagnostic: &Diagnostic, out: &mut impl Write) -> io::Result<()> {
    let record = Record::of(diagnostic);
    serde_json::to_writer(&mut *out, &record).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

#[derive(Debug, Serialize)]
struct Record<'a> {
    schema: u32,
    kind: &'static str,
    level: &'static str,
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary: Option<AnchorJson<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labels: Vec<Label<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<&'a str>,
}

impl<'a> Record<'a> {
    fn of(diagnostic: &'a Diagnostic) -> Self {
        Self {
            schema: SCHEMA,
            kind: "diagnostic",
            level: diagnostic.level.as_str(),
            code: diagnostic.code.as_str(),
            message: &diagnostic.message,
            primary: diagnostic.primary.as_ref().map(AnchorJson::of),
            labels: diagnostic
                .labels
                .iter()
                .map(|label| Label {
                    artifact: &label.anchor.artifact,
                    site: SiteJson::of(label.anchor.site),
                    message: &label.message,
                })
                .collect(),
            notes: diagnostic.notes.iter().map(String::as_str).collect(),
            help: diagnostic.help.as_deref(),
        }
    }
}

/// An anchored place as the JSON contract spells it.
///
/// The diagnostic renderer and the run-report renderer share this so a `site`
/// and a `primary` can never disagree about an artifact or a `kind`.
#[derive(Debug, Serialize)]
pub struct AnchorJson<'a> {
    artifact: &'a str,
    #[serde(flatten)]
    site: SiteJson,
}

impl<'a> AnchorJson<'a> {
    pub fn of(anchor: &'a cyberspine_diag::Anchor) -> Self {
        Self {
            artifact: &anchor.artifact,
            site: SiteJson::of(anchor.site),
        }
    }
}

#[derive(Debug, Serialize)]
struct Label<'a> {
    artifact: &'a str,
    #[serde(flatten)]
    site: SiteJson,
    message: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum SiteJson {
    Source { line: u32, column: u32 },
    Byte { offset: u64 },
    Instruction { pc: u32 },
    Session { instance: u32, tick: u64 },
    Whole,
}

impl SiteJson {
    pub(crate) fn of(site: Site) -> Self {
        match site {
            Site::Source { line, column } => Self::Source { line, column },
            Site::Byte { offset } => Self::Byte { offset },
            Site::Instruction { pc } => Self::Instruction { pc },
            Site::Session { instance, tick } => Self::Session { instance, tick },
            Site::Whole => Self::Whole,
        }
    }
}

/// One record saying the stream is short of the sink's limit.
///
/// A consumer that counts records can use this to tell "no more problems" from
/// "no more room".
pub fn truncated(dropped: usize, out: &mut impl Write) -> io::Result<()> {
    #[derive(Serialize)]
    struct Record {
        schema: u32,
        kind: &'static str,
        dropped: usize,
    }

    let record = Record {
        schema: SCHEMA,
        kind: "truncated",
        dropped,
    };
    serde_json::to_writer(&mut *out, &record).map_err(io::Error::other)?;
    out.write_all(b"\n")
}
