use std::fmt::Write as _;

use crate::Scene;
use crate::backend::{RIG_NAME, RIG_VERSION};
use crate::grid::{Cell, Grid, Heading};

const INDENT: &str = "  ";

const RESET: &str = "\x1b[0m";
const WALL_COLOR: &str = "\x1b[90m";
const START_COLOR: &str = "\x1b[2;37m";
const GOAL_COLOR: &str = "\x1b[1;33m";
const RED_COLOR: &str = "\x1b[1;31m";
const BLUE_COLOR: &str = "\x1b[1;34m";
const TRAIL_COLOR: &str = "\x1b[2;36m";
const ROBOT_COLOR: &str = "\x1b[1;92m";

const UNICODE_ARROWS: [char; 4] = ['▲', '▶', '▼', '◀'];
const ASCII_ARROWS: [char; 4] = ['^', '>', 'v', '<'];

/// Glyph set for the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    Unicode,
    Ascii,
}

/// Glyphs and colour policy for one drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub charset: Charset,
    pub color: bool,
}

impl Style {
    pub const fn plain() -> Self {
        Self {
            charset: Charset::Unicode,
            color: false,
        }
    }
}

/// Header, senses, board and legend. Everything drawn here comes from the rig;
/// the tick and the map come from the caller because the body publishes neither.
pub fn frame(grid: &Grid, scene: &Scene, style: Style) -> String {
    let observation = &scene.observation;
    let (x, y) = (observation.x, observation.y);
    let separator = match style.charset {
        Charset::Unicode => "·",
        Charset::Ascii => "|",
    };

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{INDENT}{RIG_NAME} v{RIG_VERSION}  {separator}  tick {:>3}  {separator}  ({x},{y}) {}  {separator}  {} moves  {} turns  {} bumps",
        scene.tick,
        observation.heading.name(),
        observation.odometer.moves,
        observation.odometer.turns,
        observation.odometer.bumps,
    );
    let _ = writeln!(
        out,
        "{INDENT}senses  red {}  blue {}  goal {}  wall {}",
        u8::from(observation.marker_red),
        u8::from(observation.marker_blue),
        u8::from(observation.at_goal),
        u8::from(observation.wall_ahead),
    );
    out.push('\n');
    out.push_str(&board(grid, scene, style));
    out.push('\n');
    let _ = writeln!(out, "{INDENT}{}", legend(style));
    out
}

/// One two-column cell per grid cell: terrain on the left, occupant on the
/// right. A robot standing on a marker therefore reads `R▶`, not just `▶`.
pub fn board(grid: &Grid, scene: &Scene, style: Style) -> String {
    let mut out = String::new();

    for y in 0..grid.height() {
        out.push_str(INDENT);
        for x in 0..grid.width() {
            let (x, y) = (x as i32, y as i32);
            let cell = grid.cell(x, y).expect("the board covers the grid");

            if cell == Cell::Wall {
                put(&mut out, wall(style.charset), Some(WALL_COLOR), style.color);
                continue;
            }

            put_char(&mut out, terrain(cell), terrain_color(cell), style.color);
            match occupant(scene, x, y, style.charset) {
                Some((glyph, color)) => put_char(&mut out, glyph, Some(color), style.color),
                None => out.push(' '),
            }
        }
        out.push('\n');
    }

    out
}

fn occupant(scene: &Scene, x: i32, y: i32, charset: Charset) -> Option<(char, &'static str)> {
    let observation = &scene.observation;
    if (observation.x, observation.y) == (x, y) {
        return Some((arrow(observation.heading, charset), ROBOT_COLOR));
    }
    if scene.trail.contains(&(x, y)) {
        return Some((trail(charset), TRAIL_COLOR));
    }
    None
}

fn terrain(cell: Cell) -> char {
    match cell {
        Cell::Wall => '#',
        Cell::Floor => ' ',
        Cell::Start => 'S',
        Cell::Goal => '@',
        Cell::Red => 'R',
        Cell::Blue => 'B',
    }
}

fn terrain_color(cell: Cell) -> Option<&'static str> {
    match cell {
        Cell::Wall | Cell::Floor => None,
        Cell::Start => Some(START_COLOR),
        Cell::Goal => Some(GOAL_COLOR),
        Cell::Red => Some(RED_COLOR),
        Cell::Blue => Some(BLUE_COLOR),
    }
}

fn wall(charset: Charset) -> &'static str {
    match charset {
        Charset::Unicode => "██",
        Charset::Ascii => "##",
    }
}

fn trail(charset: Charset) -> char {
    match charset {
        Charset::Unicode => '·',
        Charset::Ascii => '.',
    }
}

fn arrow(heading: Heading, charset: Charset) -> char {
    let arrows = match charset {
        Charset::Unicode => &UNICODE_ARROWS,
        Charset::Ascii => &ASCII_ARROWS,
    };
    arrows[heading.index() as usize]
}

fn legend(style: Style) -> String {
    let robot = match style.charset {
        Charset::Unicode => "▲▶▼◀",
        Charset::Ascii => "^>v<",
    };
    format!(
        "{} wall   {robot} robot   {} walked   S start   R turn left   B turn right   @ goal",
        wall(style.charset),
        trail(style.charset),
    )
}

fn put(out: &mut String, text: &str, color: Option<&str>, enabled: bool) {
    match (color, enabled) {
        (Some(color), true) => {
            out.push_str(color);
            out.push_str(text);
            out.push_str(RESET);
        }
        _ => out.push_str(text),
    }
}

fn put_char(out: &mut String, glyph: char, color: Option<&str>, enabled: bool) {
    match (color, enabled) {
        (Some(color), true) => {
            out.push_str(color);
            out.push(glyph);
            out.push_str(RESET);
        }
        _ => out.push(glyph),
    }
}
