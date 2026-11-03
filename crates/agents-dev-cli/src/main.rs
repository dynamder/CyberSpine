use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use cyberspine_backend::{BackendSpec, SpecError};
use cyberspine_vm::io::{LoadFileError, read_image};
use cyberspine_vm::{Backend, BackendError, Host, LoadError};
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

    /// Run a `.csp` image against a body and print its command stream.
    ///
    /// A development aid, not a runtime: it answers "what does this image do?"
    /// and nothing more. The image is only run against the rig it was assembled
    /// for, exactly as the host would refuse it anywhere else.
    Run {
        /// The `.csp` image to run.
        #[arg(short, long)]
        image: PathBuf,

        /// `dummy:<manifest>`, `dylib:<path>` or `rpc:<host>:<port>`.
        #[arg(short, long, value_parser = parse_backend)]
        backend: BackendSpec,

        /// How many ticks to run.
        #[arg(short, long, default_value_t = 8)]
        ticks: u32,
    },

    /// Print the identity and shape a body reports.
    Inspect {
        /// `dummy:<manifest>`, `dylib:<path>` or `rpc:<host>:<port>`.
        #[arg(short, long, value_parser = parse_backend)]
        backend: BackendSpec,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum DigestedDoc {
    Isa,
    Asm,
    Binfmt,
}

fn parse_backend(text: &str) -> Result<BackendSpec, SpecError> {
    BackendSpec::parse(text)
}

/// Everything `csp-dev` can fail at, with the path it failed on.
///
/// Same shape as `cyinea`: an error names the file it came from, `main` returns
/// an `ExitCode`, and nothing calls `process::exit`.
#[derive(Debug, Error)]
enum Error {
    #[error("{path}: {source}")]
    Image {
        path: PathBuf,
        source: LoadFileError,
    },
    #[error("{path}: {source}")]
    Load { path: PathBuf, source: LoadError },
    #[error("the backend failed: {0}")]
    Backend(#[from] BackendError),
    #[error("the image faulted:\n{0}")]
    Faulted(String),
}

fn main() -> ExitCode {
    match Args::parse().commands {
        CliCommands::DocDigest { doc } => {
            doc_digest(doc);
            ExitCode::SUCCESS
        }
        CliCommands::Inspect { backend } => report(inspect(&backend)),
        CliCommands::Run {
            image,
            backend,
            ticks,
        } => report(run_image(&image, &backend, ticks)),
    }
}

fn report(result: Result<(), Error>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_image(path: &Path, backend: &BackendSpec, ticks: u32) -> Result<(), Error> {
    let body = backend.open()?;
    let shape = body.shape();
    println!(
        "body: {} — {} muscle(s), {} float sensor(s), {} integer sensor(s)",
        body.fingerprint(),
        shape.muscles,
        shape.sensors_f,
        shape.sensors_i
    );

    let image = read_image(path).map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;

    let (mut host, _) = Host::new(body)
        .load(image)
        .map_err(|(_, source)| Error::Load {
            path: path.to_path_buf(),
            source,
        })?;
    host.set_tracing(true);

    for _ in 0..ticks {
        host.tick()?;
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

fn inspect(backend: &BackendSpec) -> Result<(), Error> {
    let body = backend.open()?;
    let shape = body.shape();

    println!("{backend:?}");
    println!("fingerprint: {}", body.fingerprint());
    println!("muscles:     {}", shape.muscles);
    println!("sensors_f:   {}", shape.sensors_f);
    println!("sensors_i:   {}", shape.sensors_i);
    Ok(())
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
