use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use clap::arg;

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
}

#[derive(Clone, Copy, ValueEnum)]
enum DigestedDoc {
    Isa,
    Asm,
    Binfmt,
}

fn main() {
    let args = Args::parse();
    match args.commands {
        Some(CliCommands::DocDigest { doc }) => doc_digest(doc),
        None => empty_command_tip(),
    }
}

fn empty_command_tip() {
    println!("use `csp-dev --help` for usage guide. ");
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
