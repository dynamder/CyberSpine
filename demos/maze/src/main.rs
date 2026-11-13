//! Run the maze action against a body and draw it while it walks.
//!
//! The default body is compiled into this binary. `--backend` names the other
//! two packagings of the very same body: a shared library built beside this
//! binary (`dylib:self`), or a process serving the wire protocol
//! (`rpc:<host>:<port>`, started with `maze-serve`).

use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use cyberspine_backend::BackendSpec;
use cyberspine_demo_maze::{
    MAX_TICKS, Scene, Session,
    render::{self, Charset, Style},
};

/// `maze`: walk the embedded action and draw it.
#[derive(Debug, Parser)]
#[command(name = "maze", version, about = "Walk the maze demo")]
struct Args {
    /// `dylib:<path>` or `rpc:<host>:<port>`; `dylib:self` loads the shared
    /// library built beside this binary.
    #[arg(short, long, value_name = "SPEC", value_parser = parse_backend)]
    backend: Option<BackendSpec>,

    /// Walk without drawing, then print the summary.
    #[arg(long)]
    headless: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let session = match &args.backend {
        None => Session::embedded(),
        Some(spec) => Session::external(spec),
    };
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };

    if args.headless {
        headless(session)
    } else {
        animate(session)
    }
}

/// The demo's own shorthand: the library Cargo emitted from this package, next
/// to the binary that is asking for it. The real spelling is still
/// `dylib:<path>`.
fn parse_backend(text: &str) -> Result<BackendSpec, String> {
    if text == "dylib:self" {
        return Ok(BackendSpec::Dylib(self_library()?));
    }
    BackendSpec::parse(text).map_err(|error| error.to_string())
}

fn self_library() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|error| format!("cannot locate this binary: {error}"))?;
    let directory = exe
        .parent()
        .ok_or_else(|| "this binary has no directory".to_owned())?;
    let path = directory.join(format!(
        "{}cyberspine_demo_maze_dylib{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX,
    ));

    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "`dylib:self` expects `{}`; build it with `cargo build -p cyberspine-demo-maze-dylib`",
            path.display()
        ))
    }
}

fn animate(mut session: Session) -> ExitCode {
    // One tick per frame: the viewer records where it has seen the runner, and a
    // frame that ran several ticks would leave gaps in that trail.
    session.set_max_ticks_per_frame(1);

    let interactive = io::stdout().is_terminal();
    let style = Style {
        charset: Charset::Unicode,
        color: interactive && std::env::var_os("NO_COLOR").is_none(),
    };

    if interactive {
        print!("\x1b[2J");
    }
    if let Err(exit) = draw(&mut session, style, interactive) {
        return exit;
    }

    let frame = Duration::from_secs_f32(session.dt());
    let mut last = Instant::now();
    while session.running() && session.ticks() < MAX_TICKS {
        // Frame pace is the demo's business; how many ticks that buys is the
        // host's. A slow frame therefore falls behind in wall-clock terms but
        // never changes what a tick contains.
        thread::sleep(frame);
        if let Err(error) = session.advance(last.elapsed().as_secs_f64()) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
        last = Instant::now();
        if let Err(exit) = draw(&mut session, style, interactive) {
            return exit;
        }
    }

    finish(&mut session)
}

fn headless(mut session: Session) -> ExitCode {
    let dt = f64::from(session.dt());
    let mut printed = 0;
    while session.running() && session.ticks() < MAX_TICKS {
        if let Err(error) = session.advance(dt) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
        printed = report(&session, printed);
    }
    finish(&mut session)
}

/// Print the rulings this frame produced, oldest first.
fn report(session: &Session, printed: usize) -> usize {
    for report in &session.reports()[printed..] {
        if !report.is_silent() {
            eprintln!("{report}");
        }
    }
    session.reports().len()
}

fn draw(session: &mut Session, style: Style, interactive: bool) -> Result<(), ExitCode> {
    let scene = match session.scene() {
        Ok(scene) => scene,
        Err(error) => {
            eprintln!("error: {error}");
            return Err(ExitCode::FAILURE);
        }
    };
    show(session, &scene, style, interactive);
    Ok(())
}

/// Redraw in place on a terminal; write the frames one after another in a pipe.
fn show(session: &Session, scene: &Scene, style: Style, interactive: bool) {
    let frame = render::frame(session.grid(), scene, style);
    if !interactive {
        print!("{frame}");
        return;
    }
    print!("\x1b[H");
    for line in frame.lines() {
        println!("{line}\x1b[K");
    }
}

fn finish(session: &mut Session) -> ExitCode {
    let outcome = match session.outcome() {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };

    for fault in &outcome.faults {
        eprintln!("  fault  instance {}  {}", fault.instance, fault.fault);
    }

    if outcome.solved() {
        println!(
            "  goal reached in {} ticks: {} moves, {} turns, {} bumps",
            outcome.ticks, outcome.moves, outcome.turns, outcome.bumps
        );
        return ExitCode::SUCCESS;
    }

    let (x, y) = outcome.position;
    eprintln!(
        "  stopped at ({x},{y}) after {} ticks without reaching the goal",
        outcome.ticks
    );
    ExitCode::FAILURE
}
