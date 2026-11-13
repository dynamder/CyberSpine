use std::path::PathBuf;

use clap::Parser;
use cyberspine_arbiter::{ArbiterSpec, SpecError as ArbiterSpecError};
use cyberspine_backend::{BackendSpec, SpecError as BackendSpecError};

use crate::run::{OnInterrupt, Options, RunMode, Stop};

/// `cyinevm`: run one or more `.csp` images against a body.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "cyinevm",
    version,
    about = "Run .csp images against a body",
    arg_required_else_help = true
)]
pub struct Args {
    /// A `.csp` image to run. Repeat `-i` to load several into one session.
    #[arg(short, long, required = true, action = clap::ArgAction::Append)]
    pub image: Vec<PathBuf>,

    /// `dylib:<path>` or `rpc:<host>:<port>`; omit for the built-in dummy body.
    #[arg(short, long, value_name = "SPEC", value_parser = parse_backend)]
    pub backend: Option<BackendSpec>,

    /// `dylib:<path>` or `rpc:<host>:<port>`; omit for the built-in single-round arbiter.
    #[arg(long, value_name = "SPEC", value_parser = parse_arbiter)]
    pub interrupt: Option<ArbiterSpec>,

    /// A directory of `.csp` actions an arbiter may load.
    #[arg(long, value_name = "DIR")]
    pub actions: Option<PathBuf>,

    /// What the arbiter falls back to when it cannot decide (`stop` or `continue`).
    #[arg(long, value_enum, default_value_t = OnInterrupt::Stop)]
    pub on_interrupt: OnInterrupt,

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
            on_interrupt: self.on_interrupt,
            interrupt: self.interrupt.clone(),
            actions: self.actions.clone(),
        }
    }
}

fn parse_backend(text: &str) -> Result<BackendSpec, BackendSpecError> {
    BackendSpec::parse(text)
}

fn parse_arbiter(text: &str) -> Result<ArbiterSpec, ArbiterSpecError> {
    ArbiterSpec::parse(text)
}
