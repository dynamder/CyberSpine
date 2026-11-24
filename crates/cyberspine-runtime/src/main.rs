use std::process::ExitCode;

use clap::Parser;
use cyberspine_diag::Diagnostics;
use cyberspine_run_report::{Format, Renderer};

use cyberspine_runtime::{Args, linked, run_external, run_linked};

fn main() -> ExitCode {
    let args = Args::parse();
    let mut options = match args.options() {
        Ok(options) => options,
        Err(error) => error.exit(),
    };
    // The startup summary is human text; on a JSON stream it would be the one
    // line a reader cannot parse. `--json` therefore implies `--quiet`.
    if args.json {
        options.quiet = true;
    }

    let diagnostics_format = if args.json {
        Format::Json
    } else {
        Format::Human
    };
    let mut renderer = Renderer::new(diagnostics_format);
    let mut sink = Diagnostics::new(false);

    // The interrupt/ruling stream is rendered where it happens: long runs show
    // progress, and JSON mode gets one `kind: "event"` line per event.
    let mut events_failed = false;
    let result = {
        let mut on_event = |report: &cyberspine_runtime::Report| {
            if renderer
                .event(report, &mut std::io::stderr().lock())
                .is_err()
            {
                events_failed = true;
            }
        };
        match &args.backend {
            Some(spec) => run_external(spec, &args.image, &options, &mut sink, &mut on_event),
            None => run_linked::<linked::Official>(&args.image, &options, &mut sink, &mut on_event),
        }
    };
    let (report, error) = result;
    let class = match &error {
        Some(error) => {
            sink.push_all(error.diagnostics());
            error.class()
        }
        None => 0,
    };

    // Diagnostics always go to stderr: stdout is the product (the trace or the
    // report), and mixing the two makes a run impossible to parse. `--json`
    // changes the format, not the stream.
    let diagnostics_failed = renderer
        .diagnostics(sink.items(), sink.dropped(), &mut std::io::stderr().lock())
        .is_err();

    // The run report has its own format flag, independent of `--json`: a caller
    // may want a machine-readable report and human diagnostics, or the reverse.
    // It is written for every exit code, a failed run most of all.
    if let Some(format) = args.report
        && Renderer::new(format.format())
            .run(&report, &mut std::io::stdout().lock())
            .is_err()
    {
        return ExitCode::from(5);
    }

    // A stream nobody can read is an environment failure; the run still gets its
    // terminal report, which is written above.
    if diagnostics_failed || events_failed {
        return ExitCode::from(5);
    }

    ExitCode::from(class)
}
