use std::{error::Error, fs};

use clap::Parser;
use cyberspine_asm::{assembler, cli::CliArgs};

fn main() -> Result<(), Box<dyn Error>> {
    let cli_args = CliArgs::parse();
    let mut asm_str = fs::read_to_string(&cli_args.asm_file)?;

    let asm = assembler::parse(&mut asm_str)?;
    let image = assembler::generate_image(asm)?;

    //TODO: write the image to file
    Ok(())
}
