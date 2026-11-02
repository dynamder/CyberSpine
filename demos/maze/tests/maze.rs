use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_demo_maze::{
    Error, MANIFEST, MAX_TICKS, SOURCE, Session, assemble,
    backend::{MazeBackend, rig},
    grid::{Cell, DEFAULT_LAYOUT, Grid, GridError},
    render::{self, Charset, Style},
};
use cyberspine_vm::{Host, LoadError};

#[test]
fn the_manifest_and_the_code_declare_the_same_rig() {
    let manifest = cyberspine_rig::from_json(MANIFEST).expect("the shipped manifest parses");
    assert_eq!(manifest, rig());
    assert_eq!(manifest.fingerprint(), rig().fingerprint());
}

#[test]
fn the_clock_comes_from_the_image() {
    let session = Session::embedded().expect("the demo assembles and loads");
    assert_eq!(session.dt(), 0.25);
}

#[test]
fn the_program_walks_the_maze_to_the_goal() {
    let mut session = Session::embedded().expect("the demo assembles and loads");
    while session.running() && session.ticks() < MAX_TICKS {
        session.advance(f64::from(session.dt()));
    }

    let outcome = session.outcome();
    assert!(outcome.faults.is_empty(), "{:?}", outcome.faults);
    assert!(
        outcome.solved(),
        "ended at {:?}: {outcome:?}",
        session.backend().position()
    );
    assert_eq!((outcome.moves, outcome.turns, outcome.bumps), (24, 6, 0));
    assert!(outcome.ticks <= 40, "took {} ticks", outcome.ticks);
}

#[test]
fn the_assembled_bytes_load_and_run() {
    let rig = cyberspine_rig::from_json(MANIFEST).expect("the shipped manifest parses");
    let bytes = assemble(SOURCE, &rig)
        .expect("the shipped source assembles")
        .to_bytes();
    let loaded = VerifiedImage::from_bytes(&bytes).expect("the assembler's own output loads");

    let mut host = Host::new(MazeBackend::new());
    host.load(loaded)
        .expect("the demo backend implements the rig the image declares");
}

#[test]
fn a_manifest_that_disagrees_with_the_backend_is_refused() {
    let swapped = MANIFEST.replace("\"forward\", \"backward\"", "\"backward\", \"forward\"");
    assert_ne!(swapped, MANIFEST);

    let error = match Session::open(SOURCE, &swapped) {
        Ok(_) => panic!("a reordered rig must not load"),
        Err(error) => error,
    };
    assert!(matches!(error, Error::Load(LoadError::RigMismatch { .. })));
}

#[test]
fn a_ragged_layout_is_refused() {
    assert_eq!(
        Grid::parse(&["###", "##"]),
        Err(GridError::Ragged {
            row: 1,
            expected: 3,
            found: 2,
        }),
    );
}

#[test]
fn every_marker_sits_on_a_junction() {
    let grid = Grid::parse(&DEFAULT_LAYOUT).expect("the shipped layout parses");
    for y in 0..grid.height() as i32 {
        for x in 0..grid.width() as i32 {
            if matches!(grid.cell(x, y), Some(Cell::Red | Cell::Blue)) {
                let open = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .filter(|(dx, dy)| grid.is_walkable(x + dx, y + dy))
                    .count();
                assert!(
                    open >= 3,
                    "the marker at ({x},{y}) has only {open} open sides"
                );
            }
        }
    }
}

#[test]
fn the_board_has_one_row_per_cell_row_and_marks_the_rover() {
    let session = Session::embedded().expect("the demo assembles and loads");
    let ascii = render::board(
        session.backend(),
        Style {
            charset: Charset::Ascii,
            color: false,
        },
    );
    assert_eq!(ascii.lines().count(), DEFAULT_LAYOUT.len());
    assert!(ascii.contains("S>"), "{ascii}");

    let unicode = render::board(session.backend(), Style::plain());
    assert!(unicode.contains('█'));
    assert!(unicode.contains('▶'));
}
