use clap::Parser;
use std::path::PathBuf;
#[derive(Debug, Clone, Parser)]
pub struct CliArgs {
    #[arg(short, long)]
    pub asm_file: PathBuf,

    /// Where to write the assembled `.csp`. Defaults to the input with a `.csp` extension.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}
