use std::io::{self, Write};

use annotate_snippets::renderer::DecorStyle;
use annotate_snippets::{AnnotationKind, Level, Renderer, Snippet};
use cyberspine_diag::{Diagnostic, Site};

use crate::source::{SourceCache, span_at};

/// A terminal rendering: a source frame when there is text to draw, a one-line
/// location otherwise.
pub fn render(
    sources: &mut SourceCache,
    diagnostic: &Diagnostic,
    out: &mut impl Write,
) -> io::Result<()> {
    // `annotate-snippets` renders the level and the id itself, as
    // `error[asm.unknown-mnemonic]: message` — the same shape the plain branch
    // below writes by hand.
    let title = format!(
        "{}[{}]: {}",
        diagnostic.level, diagnostic.code, diagnostic.message
    );

    let framed = match &diagnostic.primary {
        Some(anchor) => match anchor.site {
            Site::Source { line, column } => sources
                .get(&anchor.artifact)
                .and_then(|text| span_at(text, line, column).map(|span| (text, span, anchor))),
            _ => None,
        },
        None => None,
    };

    match framed {
        Some((text, span, anchor)) => {
            let renderer = Renderer::plain().decor_style(DecorStyle::Unicode);
            let level = match diagnostic.level {
                cyberspine_diag::Level::Error => Level::ERROR,
                cyberspine_diag::Level::Warning => Level::WARNING,
                cyberspine_diag::Level::Note => Level::INFO,
            };
            let snippet = Snippet::source(text)
                .path(anchor.artifact.as_ref())
                .line_start(1)
                .annotation(AnnotationKind::Primary.span(span));
            let titled = level
                .primary_title(diagnostic.message.as_str())
                .id(diagnostic.code.as_str());
            let block = renderer.render(&[titled.element(snippet)]);
            if block.ends_with('\n') {
                write!(out, "{block}")?;
            } else {
                writeln!(out, "{block}")?;
            }
        }
        None => {
            writeln!(out, "{title}")?;
            if let Some(anchor) = &diagnostic.primary {
                match anchor.site {
                    Site::Source { line, column } => {
                        writeln!(out, "  --> {}:{line}:{column}", anchor.artifact)?;
                    }
                    Site::Whole => writeln!(out, "  --> {}", anchor.artifact)?,
                    site => writeln!(out, "  --> {}:{site}", anchor.artifact)?,
                }
            }
        }
    }

    for label in &diagnostic.labels {
        writeln!(out, "  {}: {}", label.anchor, label.message)?;
    }
    for note in &diagnostic.notes {
        writeln!(out, "  note: {note}")?;
    }
    if let Some(help) = &diagnostic.help {
        writeln!(out, "  help: {help}")?;
    }

    Ok(())
}

/// The one-line terminal summary: what an operator reads last.
pub fn render_run(report: &crate::RunReport, out: &mut impl Write) -> io::Result<()> {
    writeln!(
        out,
        "run: {} after {} tick(s) of {:.3} ms, {} instance(s), {} overrun(s)",
        report.outcome.as_str(),
        report.ticks,
        f64::from(report.dt) * 1000.0,
        report.instances.len(),
        report.overruns
    )
}
