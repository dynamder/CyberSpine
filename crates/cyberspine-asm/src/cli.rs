use clap::Parser;
use std::path::PathBuf;
#[derive(Debug, Clone, Parser)]
pub struct CliArgs {
    #[arg(short, long)]
    pub asm_file: PathBuf,
}
