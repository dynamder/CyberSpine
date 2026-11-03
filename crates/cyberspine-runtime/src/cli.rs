use std::path::PathBuf;

use clap::Parser;
use cyberspine_backend::{BackendSpec, SpecError};

use crate::run::{Options, RunMode, Stop};

/// `cyinevm`: run a `.csp` image against a body.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "cyinevm",
    version,
    about = "Run a .csp image against a body",
    arg_required_else_help = true
)]
pub struct Args {
    /// The `.csp` image to run.
    #[arg(short, long)]
    pub image: PathBuf,

    /// `dylib:<path>` or `rpc:<host>:<port>`; omit for the built-in dummy body.
    #[arg(short, long, value_name = "SPEC", value_parser = parse_backend)]
    pub backend: Option<BackendSpec>,

    /// Run exactly N ticks and exit; 0 loads without ticking.
    #[arg(long, value_name = "N", conflicts_with = "until_halt")]
    pub ticks: Option<u64>,

    /// Stop when every instance has halted (this stops driving the body).
    #[arg(long, conflicts_with = "ticks")]
    pub until_halt: bool,

    /// Tick as fast as possible; requires `--ticks`.
    #[arg(long, requires = "ticks")]
    pub unpaced: bool,

    /// Print one line per tick to stdout.
    #[arg(long)]
    pub trace: bool,

    /// Suppress the startup summary; the dummy warning and faults still print.
    #[arg(short, long)]
    pub quiet: bool,
}

impl Args {
    pub fn options(&self) -> Options {
        let stop = match (self.ticks, self.until_halt) {
            (Some(ticks), false) => Stop::Ticks(ticks),
            (None, true) => Stop::UntilHalt,
            (None, false) => Stop::Never,
            (Some(_), true) => unreachable!("clap rejects --ticks with --until-halt"),
        };

        let mode = if self.unpaced {
            match stop {
                Stop::Ticks(ticks) => RunMode::UnpacedTicks(ticks),
                Stop::UntilHalt | Stop::Never => {
                    unreachable!("clap requires --ticks for --unpaced")
                }
            }
        } else {
            RunMode::RealTime(stop)
        };

        Options {
            mode,
            trace: self.trace,
            quiet: self.quiet,
        }
    }
}

fn parse_backend(text: &str) -> Result<BackendSpec, SpecError> {
    BackendSpec::parse(text)
}
