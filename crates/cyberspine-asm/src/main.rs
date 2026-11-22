use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use cyberspine_asm::{
    assembler::{self, Asm},
    cli::CliArgs,
};
use cyberspine_core::binfmt::Image;
use cyberspine_core::rig::Rig;
use cyberspine_diag::{Anchor, Code, Diagnostic, Diagnostics};
use cyberspine_report::{Format, Renderer};
use cyberspine_store::{self as store, WriteError};

/// The image was produced and written.
const EXIT_OK: u8 = 0;
/// The input was rejected: bad source, bad manifest, a `-o` that would eat it.
const EXIT_REJECTED: u8 = 3;
/// The environment failed: a file could not be read or written.
const EXIT_ENVIRONMENT: u8 = 5;

// Codes for the failures that come from the CLI's own work rather than from an
// error enum. They live here, next to the only code that reports them.
const MISSING_RIG: Code = Code::new("asm.missing-rig");
const UNREADABLE_RIG: Code = Code::new("asm.unreadable-rig");
const BAD_RIG: Code = Code::new("asm.bad-rig");
const UNREADABLE_SOURCE: Code = Code::new("asm.unreadable-source");
const UNWRITABLE_OUTPUT: Code = Code::new("asm.unwritable-output");
const SOURCE_OVERWRITE: Code = Code::new("asm.source-overwrite");
const DENIED_WARNINGS: Code = Code::new("asm.denied-warnings");
const INTERNAL: Code = Code::new("asm.internal");

fn main() -> ExitCode {
    let cli = CliArgs::parse();
    let format = if cli.json {
        Format::Json
    } else {
        Format::Human
    };
    let mut sink = Diagnostics::new(cli.deny_warnings);

    let source = match fs::read_to_string(&cli.asm_file) {
        Ok(text) => Some(text),
        Err(error) => {
            sink.push(
                Diagnostic::error(
                    UNREADABLE_SOURCE,
                    format!("cannot read the source: {error}"),
                )
                .at(Anchor::whole(cli.asm_file.display().to_string())),
            );
            None
        }
    };

    let class = match &source {
        Some(text) => run(&cli, text, &mut sink),
        None => EXIT_ENVIRONMENT,
    };

    let mut renderer = Renderer::new(format);
    if let Some(text) = &source {
        renderer.remember(&cli.asm_file, text.clone());
    }
    // Diagnostics always go to stderr; `--json` changes the format, not the
    // stream. stdout stays free for whatever the tool produces (`cyinea`
    // produces nothing there today, which keeps a pipeline readable).
    let _ = renderer.diagnostics(sink.items(), &mut std::io::stderr().lock());

    ExitCode::from(class)
}

fn run(cli: &CliArgs, source: &str, sink: &mut Diagnostics) -> u8 {
    let path = &cli.asm_file;

    let asm = match assembler::parse(source) {
        Ok(asm) => asm,
        Err(errors) => {
            sink.push_all(errors.diagnostics(&path.display().to_string()));
            return EXIT_REJECTED;
        }
    };

    let artifact = path.display().to_string();
    for warning in assembler::lint(&asm) {
        sink.push(warning.diagnostic(&artifact));
    }

    // The only point that decides whether the run continues: an error always
    // stops it, a warning stops it only under `--deny-warnings`.
    if sink.failed() {
        deny_warnings(sink);
        return EXIT_REJECTED;
    }

    let rig = match load_rig(cli, &asm, sink) {
        Ok(rig) => rig,
        Err(class) => return class,
    };

    let output = cli
        .output
        .clone()
        .unwrap_or_else(|| cli.asm_file.with_extension("csp"));

    if let Err(class) = refuse_to_overwrite_source(&cli.asm_file, &output, sink) {
        return class;
    }

    let Some(image) = build_image(cli, &asm, &rig, &output, sink) else {
        return EXIT_REJECTED;
    };

    match store::write_image(&image, &output) {
        Ok(()) => EXIT_OK,
        Err(error) => {
            let diagnostic = match error {
                WriteError::NotCanonical { source } => {
                    Diagnostic::error(INTERNAL, source.to_string())
                        .at(Anchor::whole(output.display().to_string()))
                        .note("this is an assembler bug, please report it")
                }
                other => Diagnostic::error(UNWRITABLE_OUTPUT, other.to_string())
                    .at(Anchor::whole(output.display().to_string())),
            };
            sink.push(diagnostic);
            EXIT_ENVIRONMENT
        }
    }
}

fn deny_warnings(sink: &mut Diagnostics) {
    if sink.has_errors() || !sink.has_warnings() {
        return;
    }
    let count = sink.items().len();
    sink.push(Diagnostic::error(
        DENIED_WARNINGS,
        format!("{count} warning(s) treated as errors"),
    ));
}

fn build_image(
    cli: &CliArgs,
    asm: &Asm,
    rig: &Rig,
    output: &Path,
    sink: &mut Diagnostics,
) -> Option<Image> {
    let built = if cli.strip {
        assembler::generate_image(asm, rig)
    } else {
        let source = debug_source(&cli.asm_file, output);
        assembler::generate_image_debug(asm, rig, &source)
    };

    match built {
        Ok(image) => Some(image),
        Err(error) => {
            sink.push(error.diagnostic(&cli.asm_file.display().to_string()));
            None
        }
    }
}

/// `docs/asm.md` §3.2: `--rig` overrides the `.rig` directive, and a relative
/// path is resolved against the `.csa`'s directory rather than the cwd.
fn load_rig(cli: &CliArgs, asm: &Asm, sink: &mut Diagnostics) -> Result<Rig, u8> {
    let declared = asm.header.rig.as_ref().map(PathBuf::from);
    let Some(path) = cli.rig.clone().or(declared) else {
        sink.push(Diagnostic::error(
            MISSING_RIG,
            "no rig manifest; add a `.rig` directive or pass `--rig`",
        ));
        return Err(EXIT_REJECTED);
    };

    let path = match cli.asm_file.parent() {
        Some(base) if !path.is_absolute() => base.join(path),
        _ => path,
    };
    let artifact = path.display().to_string();

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            sink.push(
                Diagnostic::error(
                    UNREADABLE_RIG,
                    format!("cannot read the rig manifest: {error}"),
                )
                .at(Anchor::whole(artifact)),
            );
            return Err(EXIT_REJECTED);
        }
    };

    match cyberspine_manifest::from_json(&text) {
        Ok(rig) => Ok(rig),
        Err(error) => {
            sink.push(Diagnostic::error(BAD_RIG, error.to_string()).at(Anchor::whole(artifact)));
            Err(EXIT_REJECTED)
        }
    }
}

/// The path the debug segment records: the source relative to the image's
/// directory, so a renderer resolves it next to the `.csp` no matter where the
/// pair is moved (`docs/asm.md` §8). Separators are normalized to `/` so the same
/// tree assembles to the same bytes on every platform; a source on another root
/// stays absolute, which is all it can be.
fn debug_source(source: &Path, image: &Path) -> String {
    let base = image
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let source = absolute(source);
    let base = absolute(base);
    relative_to(&source, &base)
        .unwrap_or(source)
        .to_string_lossy()
        .replace('\\', "/")
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `source` relative to `base`, when one exists. A `None` means the two share no
/// root above the base (another drive on Windows).
fn relative_to(source: &Path, base: &Path) -> Option<PathBuf> {
    let mut source_parts = source.components().peekable();
    let mut base_parts = base.components().peekable();

    while let (Some(from_source), Some(from_base)) = (source_parts.peek(), base_parts.peek()) {
        if from_source != from_base {
            break;
        }
        source_parts.next();
        base_parts.next();
    }

    // Walking up from `base` cannot reach a path that starts at a root or a
    // drive prefix: the two diverge above it.
    if matches!(
        source_parts.peek(),
        Some(Component::Prefix(_) | Component::RootDir)
    ) {
        return None;
    }

    let mut relative = PathBuf::new();
    for _ in base_parts {
        relative.push("..");
    }
    for part in source_parts {
        relative.push(part);
    }
    Some(relative)
}

/// `-o` pointed at the source would replace a `.csa` with binary, and the atomic
/// rename makes that loss permanent. Comparing the *resolved* paths also catches
/// a relative alias or a symlink between the two.
fn refuse_to_overwrite_source(
    source: &Path,
    output: &Path,
    sink: &mut Diagnostics,
) -> Result<(), u8> {
    if !output.exists() {
        return Ok(());
    }

    // A path that exists but cannot be resolved is not a reason to skip the
    // check: failing open here is what would let `-o` destroy the source.
    let resolve = |path: &Path| fs::canonicalize(path);
    let source_real = match resolve(source) {
        Ok(real) => real,
        Err(error) => {
            sink.push(
                Diagnostic::error(
                    UNREADABLE_SOURCE,
                    format!("cannot resolve the source: {error}"),
                )
                .at(Anchor::whole(source.display().to_string())),
            );
            return Err(EXIT_REJECTED);
        }
    };
    let output_real = match resolve(output) {
        Ok(real) => real,
        Err(error) => {
            sink.push(
                Diagnostic::error(
                    UNWRITABLE_OUTPUT,
                    format!("cannot resolve the output: {error}"),
                )
                .at(Anchor::whole(output.display().to_string())),
            );
            return Err(EXIT_REJECTED);
        }
    };

    if source_real == output_real {
        sink.push(
            Diagnostic::error(
                SOURCE_OVERWRITE,
                "refusing to overwrite the source file with the assembled image",
            )
            .at(Anchor::whole(output.display().to_string())),
        );
        return Err(EXIT_REJECTED);
    }

    Ok(())
}
