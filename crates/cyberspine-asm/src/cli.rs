use std::path::PathBuf;

use clap::Parser;

/// `cyinea` (cyine + a): assemble a `.csa` source into a `.csp` image.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "cyinea",
    version,
    about = "Assemble a .csa source into a .csp image"
)]
pub struct CliArgs {
    /// The `.csa` source file to assemble.
    #[arg(short, long)]
    pub asm_file: PathBuf,

    /// Where to write the assembled `.csp`. Defaults to the input with a `.csp` extension.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}
