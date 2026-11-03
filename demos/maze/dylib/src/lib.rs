//! The maze body as a shared library.
//!
//! This crate produces one artifact and nothing else: the `cdylib` a host opens
//! with `dylib:<path>`. The body itself is the `world` crate's `MazeBackend`;
//! the macro below is the whole difference between the two packagings.

use cyberspine_demo_maze_world::{MANIFEST, MazeBackend};

cyberspine_backend::export! {
    body: MazeBackend,
    rig: MANIFEST,
    create: |_config| MazeBackend::new(),
}
