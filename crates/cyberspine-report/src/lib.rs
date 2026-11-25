//! Turning diagnostics into bytes, in exactly two shapes.
//!
//! Every crate upstream produces [`cyberspine_diag::Diagnostic`] values and none
//! of them formats anything. This crate is the only place that decides what a
//! diagnostic looks like, which is what keeps the human rendering and the JSON
//! contract from drifting apart: they read the same value, and a new field is a
//! change in one file.
//!
//! The JSON shape is a contract. `schema` is bumped only when a field is removed
//! or changes meaning; `code` is stable, `message` is not.

mod format;
mod human;
mod json;
mod source;

pub use format::Format;
pub use json::AnchorJson;
pub use source::SourceCache;

use std::io::{self, Write};

use cyberspine_diag::{Diagnostic, Diagnostics};

/// Renders diagnostics, remembering source text it has already been given or
/// read so one run does not read a file twice.
#[derive(Debug, Default)]
pub struct Renderer {
    format: Format,
    sources: SourceCache,
}

impl Renderer {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            sources: SourceCache::default(),
        }
    }

    /// Hand over text the caller already has.
    ///
    /// The assembler read its `.csa` to parse it; re-reading it to draw a caret
    /// would be a second chance to disagree with what was actually parsed.
    pub fn remember(&mut self, path: impl Into<std::path::PathBuf>, text: impl Into<String>) {
        self.sources.remember(path, text);
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// Render a batch. In human mode a run that found several errors ends with a
    /// count, so a truncated terminal does not read as a complete list.
    ///
    /// The sink is both the batch and the count it refused: taking them apart
    /// would let a caller pair a short list with "nothing was dropped".
    pub fn diagnostics(
        &mut self,
        diagnostics: &Diagnostics,
        out: &mut impl Write,
    ) -> io::Result<()> {
        for diagnostic in diagnostics.items() {
            self.diagnostic(diagnostic, out)?;
        }

        if self.format == Format::Human {
            let errors = diagnostics
                .items()
                .iter()
                .filter(|item| item.is_error())
                .count();
            if errors > 1 {
                writeln!(out)?;
                writeln!(out, "{errors} error(s) shown")?;
            }
        }

        if diagnostics.dropped() > 0 {
            match self.format {
                Format::Human => human::truncated(diagnostics.dropped(), out)?,
                Format::Json => json::truncated(diagnostics.dropped(), out)?,
            }
        }

        Ok(())
    }

    pub fn diagnostic(&mut self, diagnostic: &Diagnostic, out: &mut impl Write) -> io::Result<()> {
        match self.format {
            Format::Human => human::render(&mut self.sources, diagnostic, out),
            Format::Json => json::render(diagnostic, out),
        }
    }
}
