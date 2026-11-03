//! Run the embedded maze action and draw it while it walks.
//!
//! There is no configuration here on purpose: the action's `.clock` sets the
//! pace, and `is_terminal` only decides how the frames are drawn.

use std::{
    io::{self, IsTerminal},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use cyberspine_demo_maze::{
    MAX_TICKS, Session,
    render::{self, Charset, Style},
};

fn main() -> ExitCode {
    let mut session = match Session::embedded() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };

    let interactive = io::stdout().is_terminal();
    let style = Style {
        charset: Charset::Unicode,
        color: interactive && std::env::var_os("NO_COLOR").is_none(),
    };

    if interactive {
        print!("\x1b[2J");
    }
    draw(&session, style, interactive);

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
        draw(&session, style, interactive);
    }

    let outcome = session.outcome();
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

    let (x, y) = session.backend().position();
    eprintln!(
        "  stopped at ({x},{y}) after {} ticks without reaching the goal",
        outcome.ticks
    );
    ExitCode::FAILURE
}

/// Redraw in place on a terminal; write the frames one after another in a pipe.
fn draw(session: &Session, style: Style, interactive: bool) {
    let frame = render::frame(session.backend(), session.ticks(), style);
    if !interactive {
        print!("{frame}");
        return;
    }
    print!("\x1b[H");
    for line in frame.lines() {
        println!("{line}\x1b[K");
    }
}
