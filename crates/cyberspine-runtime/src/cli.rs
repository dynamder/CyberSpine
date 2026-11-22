use std::path::PathBuf;

use clap::{CommandFactory, Parser, error::ErrorKind};
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

    /// Replay a decision log instead of consulting an arbiter.
    #[arg(long, value_name = "PATH", conflicts_with = "interrupt")]
    pub replay: Option<PathBuf>,

    /// Record every applied ruling as JSON Lines.
    #[arg(long, value_name = "PATH")]
    pub decisions: Option<PathBuf>,

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
    #[arg(long, requires = "ticks", conflicts_with = "until_halt")]
    pub unpaced: bool,

    /// Print one line per tick to stdout.
    #[arg(long)]
    pub trace: bool,

    /// Suppress the startup summary; the dummy warning and faults still print.
    #[arg(short, long)]
    pub quiet: bool,

    /// Emit diagnostics as JSON Lines on stderr instead of a terminal rendering.
    #[arg(long)]
    pub json: bool,

    /// Print the run's terminal state on stdout (`human` or `json`).
    ///
    /// The report answers "did it finish, and why": ticks, outcome, instances.
    /// It owns stdout, so it cannot be combined with `--trace`.
    #[arg(long, value_enum, value_name = "FORMAT", conflicts_with = "trace")]
    pub report: Option<ReportFormat>,
}

/// How `--report` renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReportFormat {
    Human,
    Json,
}

impl ReportFormat {
    pub fn format(self) -> cyberspine_report::Format {
        match self {
            Self::Human => cyberspine_report::Format::Human,
            Self::Json => cyberspine_report::Format::Json,
        }
    }
}

impl Args {
    /// Resolve the flags into a run mode.
    ///
    /// Every rejection is a `clap::Error`, never a panic: this binary is meant
    /// to be driven by scripts, and a caller that gets exit code 101 learns
    /// nothing about what it did wrong. The `conflicts_with`/`requires`
    /// attributes catch these combinations too, but they are not the guarantee
    /// — this function is, because it must be total.
    pub fn options(&self) -> Result<Options, clap::Error> {
        let stop = match (self.ticks, self.until_halt) {
            (Some(ticks), false) => Stop::Ticks(ticks),
            (None, true) => Stop::UntilHalt,
            (None, false) => Stop::Never,
            (Some(_), true) => {
                return Err(Self::command().error(
                    ErrorKind::ArgumentConflict,
                    "--ticks and --until-halt are mutually exclusive",
                ));
            }
        };

        let mode = if self.unpaced {
            match stop {
                Stop::Ticks(ticks) => RunMode::UnpacedTicks(ticks),
                Stop::UntilHalt | Stop::Never => {
                    return Err(Self::command().error(
                        ErrorKind::MissingRequiredArgument,
                        "--unpaced requires --ticks and cannot be combined with --until-halt",
                    ));
                }
            }
        } else {
            RunMode::RealTime(stop)
        };

        Ok(Options {
            mode,
            trace: self.trace,
            quiet: self.quiet,
            on_interrupt: self.on_interrupt,
            interrupt: self.interrupt.clone(),
            actions: self.actions.clone(),
            replay: self.replay.clone(),
            decisions: self.decisions.clone(),
        })
    }
}

fn parse_backend(text: &str) -> Result<BackendSpec, BackendSpecError> {
    BackendSpec::parse(text)
}

fn parse_arbiter(text: &str) -> Result<ArbiterSpec, ArbiterSpecError> {
    ArbiterSpec::parse(text)
}
