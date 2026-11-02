use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use cyberspine_vm::io::{LoadFileError, read_image};
use cyberspine_vm::{DummyBackend, Host, LoadError};
use thiserror::Error;

#[derive(Parser)]
#[command(arg_required_else_help = true)]
struct Args {
    #[command(subcommand)]
    commands: CliCommands,
}

#[derive(Subcommand)]
enum CliCommands {
    /// Print a one-line digest of a normative document.
    DocDigest {
        #[arg(long)]
        doc: DigestedDoc,
    },

    /// Run a `.csp` image against a zero rig and print its command stream.
    ///
    /// A development aid, not a runtime: every sensor reports zero and nothing
    /// is displayed, so it answers "what does this image do?" and nothing more.
    /// The image is only run against the rig it was assembled for, exactly as the
    /// host would refuse it anywhere else.
    Run {
        /// The `.csp` image to run.
        #[arg(short, long)]
        image: PathBuf,

        /// The manifest the image was assembled for.
        #[arg(short, long)]
        rig: PathBuf,

        /// How many ticks to run.
        #[arg(short, long, default_value_t = 8)]
        ticks: u32,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum DigestedDoc {
    Isa,
    Asm,
    Binfmt,
}

/// Everything `csp-dev run` can fail at, with the path it failed on.
///
/// Same shape as `cyinea`: an error names the file it came from, `main` returns
/// an `ExitCode`, and nothing calls `process::exit`.
#[derive(Debug, Error)]
enum Error {
    #[error("cannot read `{path}`: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Manifest {
        path: PathBuf,
        source: cyberspine_manifest::ManifestError,
    },
    #[error("{path}: {source}")]
    Image {
        path: PathBuf,
        source: LoadFileError,
    },
    #[error("{path}: {source}")]
    Load { path: PathBuf, source: LoadError },
    #[error("the image faulted:\n{0}")]
    Faulted(String),
}

fn main() -> ExitCode {
    match Args::parse().commands {
        CliCommands::DocDigest { doc } => {
            doc_digest(doc);
            ExitCode::SUCCESS
        }
        CliCommands::Run { image, rig, ticks } => match run_image(&image, &rig, ticks) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_image(path: &Path, rig_path: &Path, ticks: u32) -> Result<(), Error> {
    let manifest = std::fs::read_to_string(rig_path).map_err(|source| Error::Read {
        path: rig_path.to_path_buf(),
        source,
    })?;
    let rig = cyberspine_manifest::from_json(&manifest).map_err(|source| Error::Manifest {
        path: rig_path.to_path_buf(),
        source,
    })?;

    println!(
        "dev rig: {} — {} muscle(s), {} float sensor(s), {} integer sensor(s), all zero",
        rig.name,
        rig.muscles.len(),
        rig.sensors_f.len(),
        rig.sensors_i.len()
    );

    let image = read_image(path).map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;

    let mut host = Host::new(DummyBackend::new(rig));
    host.set_tracing(true);
    host.load(image).map_err(|source| Error::Load {
        path: path.to_path_buf(),
        source,
    })?;

    for _ in 0..ticks {
        host.tick();
    }

    for tick in host.trace() {
        println!("tick {:>4}: {:?}", tick.now, tick.commands);
    }
    println!("{} instance(s) left", host.instances());

    let faults = host.take_faults();
    if faults.is_empty() {
        return Ok(());
    }
    let mut message = String::new();
    for record in faults {
        message.push_str(&format!(
            "  instance {}: {}\n",
            record.instance, record.fault
        ));
    }
    Err(Error::Faulted(message))
}

fn doc_digest(doc: DigestedDoc) {
    match doc {
        DigestedDoc::Isa => {
            println!("Instruction set, bytecode, vm model and runtime convention. See docs/isa.md")
        }
        DigestedDoc::Asm => println!("Assembly code design, the .csa files. See docs/asm.md"),
        DigestedDoc::Binfmt => {
            println!("Format for the executable image, the .csp files. See docs/binfmt.md")
        }
    }
}
