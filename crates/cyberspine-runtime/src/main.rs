use std::process::ExitCode;

use clap::Parser;
use cyberspine_runtime::{Args, linked, run_external, run_linked};

fn main() -> ExitCode {
    let args = Args::parse();
    let options = args.options();

    let result = match &args.backend {
        Some(spec) => run_external(spec, &args.image, &options),
        None => run_linked::<linked::Official>(&args.image, &options),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
