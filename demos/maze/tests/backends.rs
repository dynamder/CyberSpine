//! One body, three packagings: the same image must produce the same tick
//! stream and the same picture whichever way the body is reached.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use cyberspine_backend::{BackendSpec, rpc};
use cyberspine_demo_maze::{
    Body, Error, MANIFEST, MAX_TICKS, SOURCE, Session,
    backend::MazeBackend,
    render::{self, Charset, Style},
};
use cyberspine_vm::LoadError;

use serde_json::Value;

const STYLE: Style = Style {
    charset: Charset::Ascii,
    color: false,
};

/// Walk both sessions in lockstep and prove they saw the same ticks.
fn same_walk(mut linked: Session, mut other: Session) {
    let mut frames = 0usize;
    loop {
        let left = linked.scene().expect("the linked body cannot fail");
        let right = other.scene().expect("the other packaging answers");

        assert_eq!(left.tick, right.tick, "frame {frames}");
        assert_eq!(
            render::frame(linked.grid(), &left, STYLE),
            render::frame(other.grid(), &right, STYLE),
            "the picture at tick {}",
            left.tick
        );
        assert_eq!(left.trail, right.trail, "the trail at tick {}", left.tick);

        if !linked.running() {
            break;
        }
        assert!(other.running(), "both packagings halt at the same tick");

        let dt = f64::from(linked.dt());
        linked.advance(dt).expect("the linked body cannot fail");
        other.advance(dt).expect("the other packaging answers");

        frames += 1;
        assert!(frames <= MAX_TICKS as usize, "the walk never ended");
    }

    assert_eq!(commands(&linked), commands(&other));

    let linked = linked.outcome().expect("the linked body cannot fail");
    let other = other.outcome().expect("the other packaging answers");
    assert_eq!(other, linked);
    assert_eq!(
        (linked.moves, linked.turns, linked.bumps),
        (24, 6, 0),
        "the odometer crosses the boundary unchanged"
    );
}

fn commands(session: &Session) -> Vec<(u64, Vec<Option<f32>>)> {
    session
        .trace()
        .map(|tick| (tick.now, tick.commands.to_vec()))
        .collect()
}

fn serve_maze() -> String {
    let (address, _server) = rpc::spawn(
        "127.0.0.1:0",
        MazeBackend::new(),
        MANIFEST.to_owned(),
        "test".to_owned(),
    )
    .expect("the body binds its address");
    address.to_string()
}

#[test]
fn a_remote_body_walks_and_draws_the_same_maze() {
    let address = serve_maze();
    let linked = Session::embedded().expect("the linked body loads");
    let remote = Session::external(&BackendSpec::Rpc(address)).expect("the remote body answers");

    same_walk(linked, remote);
}

#[test]
fn an_image_for_another_rig_never_reaches_a_remote_tick() {
    let address = serve_maze();
    let body = Body::external(&BackendSpec::Rpc(address)).expect("the remote body answers");

    let swapped = MANIFEST.replace("\"forward\", \"backward\"", "\"backward\", \"forward\"");
    assert_ne!(swapped, MANIFEST);

    let error = match Session::open(SOURCE, &swapped, body) {
        Ok(_) => panic!("an image for another rig must not load"),
        Err(error) => error,
    };

    assert!(
        matches!(error, Error::Load(LoadError::RigMismatch { .. })),
        "{error}"
    );
}

#[test]
fn a_library_walks_and_draws_the_same_maze() {
    let linked = Session::embedded().expect("the linked body loads");
    let loaded =
        Session::external(&BackendSpec::Dylib(library().to_path_buf())).expect("the library loads");

    same_walk(linked, loaded);
}

/// The cdylib half of this package, built on demand: `cargo test` builds the
/// harness, not the shared library. The artifact path comes from
/// `--message-format=json`, which keeps the test independent of the target
/// directory, the profile and the platform's library suffix.
fn library() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();

    PATH.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args([
                "build",
                "-p",
                "cyberspine-demo-maze",
                "--lib",
                "--message-format=json",
                "--offline",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");

        assert!(
            output.status.success(),
            "cargo could not build the demo library:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record["reason"] != "compiler-artifact" {
                continue;
            }
            if !record["package_id"]
                .as_str()
                .unwrap_or_default()
                .contains("cyberspine-demo-maze")
            {
                continue;
            }
            for file in record["filenames"].as_array().into_iter().flatten() {
                let path = PathBuf::from(file.as_str().unwrap_or_default());
                if path.extension().and_then(|ext| ext.to_str())
                    == Some(std::env::consts::DLL_EXTENSION)
                {
                    return path;
                }
            }
        }

        panic!("cargo did not report a shared library for the demo");
    })
}
