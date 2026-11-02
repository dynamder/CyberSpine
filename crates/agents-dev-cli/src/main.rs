use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use cyberspine_vm::io::read_image;
use cyberspine_vm::{DummyBackend, Host};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    commands: Option<CliCommands>,
}

#[derive(Subcommand)]
enum CliCommands {
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

fn main() {
    match Args::parse().commands {
        Some(CliCommands::DocDigest { doc }) => doc_digest(doc),
        Some(CliCommands::Run { image, rig, ticks }) => {
            if let Err(message) = run_image(&image, &rig, ticks) {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
        }
        None => empty_command_tip(),
    }
}

fn empty_command_tip() {
    println!("use `csp-dev --help` for usage guide.");
}

fn run_image(path: &Path, rig_path: &Path, ticks: u32) -> Result<(), String> {
    let manifest = std::fs::read_to_string(rig_path)
        .map_err(|error| format!("cannot read `{}`: {error}", rig_path.display()))?;
    let rig = cyberspine_rig::from_json(&manifest)
        .map_err(|error| format!("{}: {error}", rig_path.display()))?;

    println!(
        "dev rig: {} — {} muscle(s), {} float sensor(s), {} integer sensor(s), all zero",
        rig.name,
        rig.muscles.len(),
        rig.sensors_f.len(),
        rig.sensors_i.len()
    );

    let image = read_image(path).map_err(|error| error.to_string())?;

    let mut host = Host::new(DummyBackend::new(rig));
    host.set_tracing(true);
    host.load(image).map_err(|error| error.to_string())?;

    for _ in 0..ticks {
        host.tick();
    }

    for tick in host.trace() {
        println!("tick {:>4}: {:?}", tick.now, tick.commands);
    }
    println!("{} instance(s) left", host.instances());
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
