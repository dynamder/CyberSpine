//! Serve the maze body over the wire protocol.
//!
//! The host is the client (`maze -b rpc:<host>:<port>`), so the process that
//! owns the body listens. One process serves one run: the protocol has no reset,
//! and the body keeps its position across connections, which is the honest model
//! of a device that was driven once.

use std::process::ExitCode;

use clap::Parser;
use cyberspine_backend::rpc;
use cyberspine_demo_maze_world::{MANIFEST, MazeBackend};

#[derive(Debug, Parser)]
#[command(name = "maze-serve", version, about = "Serve the maze body over TCP")]
struct Args {
    /// Address to listen on; the port is printed once it is bound.
    #[arg(long, default_value = "127.0.0.1:7777")]
    listen: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let build_id = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

    let (address, thread) = match rpc::spawn(
        &args.listen,
        MazeBackend::new(),
        MANIFEST.to_owned(),
        build_id.to_owned(),
    ) {
        Ok(serving) => serving,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };

    println!("maze body serving on {address} ({build_id})\n  connect with: maze -b rpc:{address}");

    match thread.join() {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(error)) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("error: the serving thread panicked");
            ExitCode::FAILURE
        }
    }
}
