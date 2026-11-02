use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use cyberspine_vm::io::read_image;
use cyberspine_vm::{DummyBackend, Host, RigLayout};

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
    Run {
        /// The `.csp` image to run.
        #[arg(short, long)]
        image: PathBuf,

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
        Some(CliCommands::Run { image, ticks }) => {
            if let Err(message) = run_image(&image, ticks) {
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

fn run_image(path: &Path, ticks: u32) -> Result<(), String> {
    let image = read_image(path).map_err(|error| error.to_string())?;

    let layout = RigLayout {
        sensors_f: image.sensors_f.len(),
        sensors_i: image.sensors_i.len(),
        muscles: image.muscles.len(),
    };
    println!(
        "dev rig: {} muscle(s), {} float sensor(s), {} integer sensor(s), all zero",
        layout.muscles, layout.sensors_f, layout.sensors_i
    );

    let mut host = Host::new(DummyBackend::new(layout));
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
