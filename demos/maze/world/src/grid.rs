use thiserror::Error;

/// One grid cell: the kind of floor the runner can stand on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Wall,
    Floor,
    Start,
    Goal,
    Red,
    Blue,
}

impl Cell {
    pub fn from_char(glyph: char) -> Option<Self> {
        match glyph {
            '#' => Some(Self::Wall),
            '.' => Some(Self::Floor),
            'S' => Some(Self::Start),
            '*' => Some(Self::Goal),
            'R' => Some(Self::Red),
            'B' => Some(Self::Blue),
            _ => None,
        }
    }

    pub const fn walkable(self) -> bool {
        !matches!(self, Self::Wall)
    }
}

/// A grid heading. `index()` is the value the `heading` sensor reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    North,
    East,
    South,
    West,
}

impl Heading {
    pub const fn left(self) -> Self {
        match self {
            Self::North => Self::West,
            Self::West => Self::South,
            Self::South => Self::East,
            Self::East => Self::North,
        }
    }

    pub const fn right(self) -> Self {
        match self {
            Self::North => Self::East,
            Self::East => Self::South,
            Self::South => Self::West,
            Self::West => Self::North,
        }
    }

    pub const fn delta(self) -> (i32, i32) {
        match self {
            Self::North => (0, -1),
            Self::East => (1, 0),
            Self::South => (0, 1),
            Self::West => (-1, 0),
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    /// The inverse of [`Heading::index`], for a value read back from a body.
    pub const fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::North),
            1 => Some(Self::East),
            2 => Some(Self::South),
            3 => Some(Self::West),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
        }
    }
}

/// `#` wall, `.` floor, `S` start, `*` goal, `R` turn left, `B` turn right.
///
/// The unsigned junction at `(4,11)` has no marker and the cell straight ahead
/// is a wall, so the colour rule cannot decide there. The left branch is the way
/// on and the right branch is a dead end. The start and `(2,11)` are marker-less
/// junctions where going straight is the rule.
pub const DEFAULT_LAYOUT: [&str; 15] = [
    "#####################",
    "############.########",
    "##########..B...*####",
    "########.###.########",
    "######..B...R..######",
    "####.###.###.########",
    "####.###.###.########",
    "##..B...R..##########",
    "####.###.############",
    "##.#.###.############",
    "##.#.################",
    "#S...################",
    "#.##.################",
    "#.##.################",
    "#####################",
];

/// A rectangular cell map parsed from ASCII rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    cells: Vec<Cell>,
    width: usize,
    height: usize,
    start: (i32, i32),
    goal: (i32, i32),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GridError {
    #[error("the maze has no rows")]
    Empty,
    #[error("row {row} has {found} cells, expected {expected}")]
    Ragged {
        row: usize,
        expected: usize,
        found: usize,
    },
    #[error("row {row}, column {column}: `{found}` is not a maze cell")]
    UnknownCell {
        row: usize,
        column: usize,
        found: char,
    },
    #[error("the maze has no start")]
    NoStart,
    #[error("the maze has more than one start")]
    DuplicateStart,
    #[error("the maze has no goal")]
    NoGoal,
    #[error("the maze has more than one goal")]
    DuplicateGoal,
}

impl Grid {
    /// Parse rows of `#` `.` `S` `*` `R` `B`; `S` and `*` are required.
    pub fn parse(rows: &[&str]) -> Result<Self, GridError> {
        let Some(first) = rows.first() else {
            return Err(GridError::Empty);
        };

        let height = rows.len();
        let width = first.chars().count();
        let mut cells = Vec::with_capacity(width * height);
        let mut start = None;
        let mut goal = None;

        for (y, row) in rows.iter().enumerate() {
            let found = row.chars().count();
            if found != width {
                return Err(GridError::Ragged {
                    row: y,
                    expected: width,
                    found,
                });
            }
            for (x, glyph) in row.chars().enumerate() {
                let cell = Cell::from_char(glyph).ok_or(GridError::UnknownCell {
                    row: y,
                    column: x,
                    found: glyph,
                })?;
                match cell {
                    Cell::Start if start.is_some() => return Err(GridError::DuplicateStart),
                    Cell::Start => start = Some((x as i32, y as i32)),
                    Cell::Goal if goal.is_some() => return Err(GridError::DuplicateGoal),
                    Cell::Goal => goal = Some((x as i32, y as i32)),
                    _ => {}
                }
                cells.push(cell);
            }
        }

        Ok(Self {
            cells,
            width,
            height,
            start: start.ok_or(GridError::NoStart)?,
            goal: goal.ok_or(GridError::NoGoal)?,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn start(&self) -> (i32, i32) {
        self.start
    }

    pub fn goal(&self) -> (i32, i32) {
        self.goal
    }

    pub fn cell(&self, x: i32, y: i32) -> Option<Cell> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.cells[y * self.width + x])
    }

    pub fn is_walkable(&self, x: i32, y: i32) -> bool {
        self.cell(x, y).is_some_and(Cell::walkable)
    }
}
