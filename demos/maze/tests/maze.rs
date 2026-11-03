use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_demo_maze::{
    Body, Error, MANIFEST, MAX_TICKS, SOURCE, Session, assemble,
    render::{self, Charset, Style},
};
use cyberspine_demo_maze_world::{DEFAULT_LAYOUT, MazeBackend};
use cyberspine_vm::{Host, LoadError};

#[test]
fn the_clock_comes_from_the_image() {
    let session = Session::embedded().expect("the demo assembles and loads");
    assert_eq!(session.dt(), 0.25);
}

#[test]
fn the_program_walks_the_maze_to_the_goal() {
    let mut session = Session::embedded().expect("the demo assembles and loads");
    while session.running() && session.ticks() < MAX_TICKS {
        session
            .advance(f64::from(session.dt()))
            .expect("the maze backend cannot fail");
    }

    let outcome = session.outcome().expect("the linked body cannot fail");
    assert!(outcome.faults.is_empty(), "{:?}", outcome.faults);
    assert!(
        outcome.solved(),
        "ended at {:?}: {outcome:?}",
        outcome.position
    );
    assert_eq!((outcome.moves, outcome.turns, outcome.bumps), (24, 6, 0));
    assert!(outcome.ticks <= 40, "took {} ticks", outcome.ticks);
}

#[test]
fn the_assembled_bytes_load_and_run() {
    let rig = cyberspine_manifest::from_json(MANIFEST).expect("the shipped manifest parses");
    let bytes = assemble(SOURCE, &rig)
        .expect("the shipped source assembles")
        .to_bytes();
    let loaded = VerifiedImage::from_bytes(&bytes).expect("the assembler's own output loads");

    let host = Host::new(MazeBackend::new());
    host.load(loaded)
        .expect("the demo backend implements the rig the image declares");
}

#[test]
fn a_manifest_that_disagrees_with_the_backend_is_refused() {
    let swapped = MANIFEST.replace("\"forward\", \"backward\"", "\"backward\", \"forward\"");
    assert_ne!(swapped, MANIFEST);

    let error = match Session::open(SOURCE, &swapped, Body::linked()) {
        Ok(_) => panic!("a reordered rig must not load"),
        Err(error) => error,
    };
    assert!(matches!(error, Error::Load(LoadError::RigMismatch { .. })));
}

#[test]
fn the_board_has_one_row_per_cell_row_and_marks_the_rover() {
    let mut session = Session::embedded().expect("the demo assembles and loads");
    let scene = session.scene().expect("the linked body cannot fail");
    let ascii = render::board(
        session.grid(),
        &scene,
        Style {
            charset: Charset::Ascii,
            color: false,
        },
    );
    assert_eq!(ascii.lines().count(), DEFAULT_LAYOUT.len());
    assert!(ascii.contains("S>"), "{ascii}");

    let unicode = render::board(session.grid(), &scene, Style::plain());
    assert!(unicode.contains('█'));
    assert!(unicode.contains('▶'));
}
