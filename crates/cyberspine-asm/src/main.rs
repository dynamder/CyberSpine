use std::{fs, path::Path, process::ExitCode};

use clap::Parser;
use cyberspine_asm::{
    assembler,
    cli::CliArgs,
    store::{self, WriteError},
};

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

    refuse_to_overwrite_source(&cli_args.asm_file, &output)?;

    store::write_image(&image, &output).map_err(|error| match error {
        // The loader rejecting our own output is an assembler bug, not a
        // property of the user's source, so say so instead of blaming the input.
        WriteError::NotCanonical { source } => format!(
            "{}: internal error: refusing to write an image the loader rejects: {source}\n",
            cli_args.asm_file.display()
        ),
        other => format!("error: {other}\n"),
    })?;

    Ok(())
}

/// `-o` pointed at the source would replace a `.csa` with binary, and the atomic
/// rename makes that loss permanent. Comparing the *resolved* paths also catches
/// a relative alias or a symlink between the two.
fn refuse_to_overwrite_source(source: &Path, output: &Path) -> Result<(), String> {
    if !output.exists() {
        return Ok(());
    }

    let (Ok(source_real), Ok(output_real)) = (fs::canonicalize(source), fs::canonicalize(output))
    else {
        return Ok(());
    };

    if source_real == output_real {
        return Err(format!(
            "error: `{}` is the source file; refusing to overwrite it with the assembled image\n",
            output.display()
        ));
    }

    Ok(())
}
