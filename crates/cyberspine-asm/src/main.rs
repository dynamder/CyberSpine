use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use cyberspine_asm::{
    assembler::{self, Asm, ImageGenError, diagnostic},
    cli::CliArgs,
    store::{self, WriteError},
};
use cyberspine_core::rig::Rig;

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

    let rig = load_rig(cli_args, &asm)?;

    let image = assembler::generate_image(&asm, &rig).map_err(|error| {
        let title = error.to_string();
        match error.location() {
            Some(location) => {
                diagnostic::render_error(&title, location, &cli_args.asm_file, &asm_str)
            }
            // A bug in the assembler, not in the source: say which one it is so
            // the report does not read like a user error.
            None if matches!(error, ImageGenError::Internal { .. }) => format!(
                "{}: error: {title}\nthis is an assembler bug, please report it\n",
                cli_args.asm_file.display()
            ),
            None => format!("{}: error: {title}\n", cli_args.asm_file.display()),
        }
    })?;

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

/// `docs/asm.md` §3.2: `--rig` overrides the `.rig` directive, and a relative
/// path is resolved against the `.csa`'s directory rather than the cwd.
fn load_rig(cli_args: &CliArgs, asm: &Asm) -> Result<Rig, String> {
    let declared = asm.header.rig.as_ref().map(PathBuf::from);
    let path = cli_args.rig.clone().or(declared).ok_or_else(|| {
        format!(
            "{}: error: no rig manifest; add a `.rig` directive or pass `--rig`\n",
            cli_args.asm_file.display()
        )
    })?;

    let path = match cli_args.asm_file.parent() {
        Some(base) if !path.is_absolute() => base.join(path),
        _ => path,
    };

    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "error: cannot read the rig manifest `{}`: {error}\n",
            path.display()
        )
    })?;

    cyberspine_rig::from_json(&text)
        .map_err(|error| format!("{}: error: {error}\n", path.display()))
}

/// `-o` pointed at the source would replace a `.csa` with binary, and the atomic
/// rename makes that loss permanent. Comparing the *resolved* paths also catches
/// a relative alias or a symlink between the two.
fn refuse_to_overwrite_source(source: &Path, output: &Path) -> Result<(), String> {
    if !output.exists() {
        return Ok(());
    }

    // A path that exists but cannot be resolved is not a reason to skip the
    // check: failing open here is what would let `-o` destroy the source.
    let resolve = |path: &Path| {
        fs::canonicalize(path)
            .map_err(|error| format!("error: cannot resolve `{}`: {error}\n", path.display()))
    };
    let source_real = resolve(source)?;
    let output_real = resolve(output)?;

    if source_real == output_real {
        return Err(format!(
            "error: `{}` is the source file; refusing to overwrite it with the assembled image\n",
            output.display()
        ));
    }

    Ok(())
}
