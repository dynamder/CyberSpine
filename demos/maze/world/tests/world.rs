//! The world is the body: it declares the rig it implements and reports the
//! start of a run. The app's tests drive it; these tests only pin its contract.

use cyberspine_core::rig::BoundFingerprint;
use cyberspine_demo_maze_world::{
    Cell, DEFAULT_LAYOUT, Grid, GridError, Heading, MANIFEST, MazeBackend, SensorI, rig,
};
use cyberspine_vm::{Backend, Shape, Snapshot};

#[test]
fn the_manifest_and_the_code_declare_the_same_rig() {
    let manifest = cyberspine_manifest::from_json(MANIFEST).expect("the shipped manifest parses");
    assert_eq!(manifest, rig());
    assert_eq!(manifest.fingerprint(), rig().fingerprint());
}

#[test]
fn the_body_declares_the_rig_it_implements() {
    let body = MazeBackend::new();

    assert_eq!(body.fingerprint(), BoundFingerprint::of(&rig()));
    assert_eq!(
        body.shape(),
        Shape {
            muscles: rig().muscles.len(),
            sensors_f: rig().sensors_f.len(),
            sensors_i: rig().sensors_i.len(),
        }
    );
}

#[test]
fn the_body_starts_on_the_maze_start_with_a_zero_odometer() {
    let grid = Grid::parse(&DEFAULT_LAYOUT).expect("the shipped layout parses");
    let (x, y) = grid.start();

    let mut body = MazeBackend::new();
    let mut f = vec![0.0; rig().sensors_f.len()];
    let mut i = vec![0; rig().sensors_i.len()];
    let mut muscles = vec![0.0; rig().muscles.len()];
    body.sample(&mut Snapshot {
        f: &mut f,
        i: &mut i,
        muscles: &mut muscles,
    })
    .expect("a linked body cannot fail");

    let sensor = |sensor: SensorI| i[sensor.index()];
    assert_eq!((sensor(SensorI::X), sensor(SensorI::Y)), (x, y));
    assert_eq!(sensor(SensorI::Heading), Heading::East.index());
    assert_eq!(sensor(SensorI::Moves), 0);
    assert_eq!(sensor(SensorI::Turns), 0);
    assert_eq!(sensor(SensorI::Bumps), 0);
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
