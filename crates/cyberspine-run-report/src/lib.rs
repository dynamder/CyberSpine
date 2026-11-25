//! The whole VM output in one format: diagnostics, the interrupt stream, and
//! the terminal run report.
//!
//! This is deliberately a separate crate from [`cyberspine_report`]: `cyinea`
//! renders diagnostics but never runs an image, so it must not link the host,
//! the arbiter, or the plugin boundary.

mod event;
mod render;
mod run;

pub use cyberspine_report::Format;
pub use run::{InstanceReport, Outcome, RunReport};

use std::io::{self, Write};

use cyberspine_diag::{Diagnostic, Diagnostics};
use cyberspine_host::Report;

/// Renders everything `cyinevm` writes through one format and one set of
/// remembered sources.
#[derive(Debug, Default)]
pub struct Renderer {
    diagnostics: cyberspine_report::Renderer,
}

impl Renderer {
    pub fn new(format: Format) -> Self {
        Self {
            diagnostics: cyberspine_report::Renderer::new(format),
        }
    }

    /// Hand over text the caller already has, for the diagnostic renderer.
    pub fn remember(&mut self, path: impl Into<std::path::PathBuf>, text: impl Into<String>) {
        self.diagnostics.remember(path, text);
    }

    pub fn format(&self) -> Format {
        self.diagnostics.format()
    }

    pub fn diagnostics(
        &mut self,
        diagnostics: &Diagnostics,
        out: &mut impl Write,
    ) -> io::Result<()> {
        self.diagnostics.diagnostics(diagnostics, out)
    }

    pub fn diagnostic(&mut self, diagnostic: &Diagnostic, out: &mut impl Write) -> io::Result<()> {
        self.diagnostics.diagnostic(diagnostic, out)
    }

    /// One interrupt or ruling, as it happens.
    pub fn event(&mut self, report: &Report, out: &mut impl Write) -> io::Result<()> {
        match self.format() {
            Format::Human => event::render(report, out),
            Format::Json => event::render_json(report, out),
        }
    }

    /// The terminal state of a run, once it has stopped.
    pub fn run(&mut self, report: &RunReport, out: &mut impl Write) -> io::Result<()> {
        match self.format() {
            Format::Human => render::render(report, out),
            Format::Json => render::render_json(report, out),
        }
    }
}
