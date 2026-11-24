use std::io::{self, Write};

use serde::Serialize;

use crate::RunReport;

/// The one-line terminal summary: what an operator reads last.
pub fn render(report: &RunReport, out: &mut impl Write) -> io::Result<()> {
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

/// One run, one object, on stdout.
pub fn render_json(report: &RunReport, out: &mut impl Write) -> io::Result<()> {
    #[derive(Serialize)]
    struct Record<'a> {
        schema: u32,
        kind: &'static str,
        #[serde(flatten)]
        report: &'a RunReport,
    }

    let record = Record {
        schema: cyberspine_diag::SCHEMA,
        kind: "run",
        report,
    };
    serde_json::to_writer(&mut *out, &record).map_err(io::Error::other)?;
    out.write_all(b"\n")
}
