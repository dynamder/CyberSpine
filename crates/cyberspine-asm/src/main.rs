use std::{fs, process::ExitCode};

use clap::Parser;
use cyberspine_asm::{assembler, cli::CliArgs};

fn main() -> ExitCode {
    let cli_args = CliArgs::parse();

    match run(&cli_args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprint!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli_args: &CliArgs) -> Result<(), String> {
    let asm_str = fs::read_to_string(&cli_args.asm_file).map_err(|error| {
        format!(
            "error: cannot read `{}`: {error}\n",
            cli_args.asm_file.display()
        )
    })?;

    let asm =
        assembler::parse(&asm_str).map_err(|errors| errors.render(&cli_args.asm_file, &asm_str))?;

    let image = assembler::generate_image(&asm)
        .map_err(|error| format!("{}: error: {error}\n", cli_args.asm_file.display()))?;

    let output = cli_args
        .output
        .clone()
        .unwrap_or_else(|| cli_args.asm_file.with_extension("csp"));

    fs::write(&output, image.to_bytes())
        .map_err(|error| format!("error: cannot write `{}`: {error}\n", output.display()))?;

    Ok(())
}
