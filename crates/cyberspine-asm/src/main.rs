use std::{error::Error, fs};

use clap::Parser;
use cyberspine_asm::{assembler, cli::CliArgs};

fn main() -> Result<(), Box<dyn Error>> {
    let cli_args = CliArgs::parse();
    let asm_str = fs::read_to_string(&cli_args.asm_file)?;

    let asm = assembler::parse(&asm_str)?;
    let image = assembler::generate_image(&asm)?;

    let output = cli_args
        .output
        .clone()
        .unwrap_or_else(|| cli_args.asm_file.with_extension("csp"));
    fs::write(output, image.to_bytes())?;

    Ok(())
}
