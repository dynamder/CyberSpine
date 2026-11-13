use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_demo_maze::{
    Body, Error, MANIFEST, SOURCE, Session, assemble,
    render::{self, Charset, Style},
};
use cyberspine_demo_maze_world::{DEFAULT_LAYOUT, MazeBackend};
use cyberspine_host::Report;
use cyberspine_vm::{Host, LoadError};

#[test]
fn the_clock_comes_from_the_image() {
    let session = Session::embedded().expect("the demo assembles and loads");
    assert_eq!(session.dt(), 0.25);
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

#[test]
fn the_ruling_log_sits_below_the_board_without_moving_it() {
    let mut session = Session::embedded().expect("the demo assembles and loads");
    let scene = session.scene().expect("the linked body cannot fail");
    let before = render::frame(session.grid(), &scene, Style::plain());
    let board_before = render::board(session.grid(), &scene, Style::plain());

    while !session
        .reports()
        .iter()
        .any(|report| matches!(report, Report::Finished { .. }))
        && session.ticks() < 20
    {
        session
            .advance(f64::from(session.dt()))
            .expect("the linked body cannot fail");
    }

    let scene = session.scene().expect("the linked body cannot fail");
    let after = render::frame(session.grid(), &scene, Style::plain());
    let board_after = render::board(session.grid(), &scene, Style::plain());

    assert_eq!(
        before.lines().count(),
        after.lines().count(),
        "the frame height is fixed"
    );
    assert_eq!(
        board_before.lines().count(),
        board_after.lines().count(),
        "the ruling log does not move the board"
    );

    let lines: Vec<&str> = after.lines().collect();
    let tail = &lines[lines.len() - 3..];
    assert!(tail[0].contains("no progress"), "{tail:?}");
    assert!(tail[1].contains("backing off"), "{tail:?}");
    assert!(tail[2].contains("turning left"), "{tail:?}");
}
