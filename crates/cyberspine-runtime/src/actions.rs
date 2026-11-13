//! Turning an action reference into a verified image.
//!
//! An action reference is an opaque string that the arbiter got in an
//! [`InterruptEvent`](cyberspine_arbiter::InterruptEvent) and echoed back in
//! [`Action::Load`](cyberspine_arbiter::Action::Load). This is the one place
//! that decides what such a string means: a directory of images, joined by file
//! name.
//!
//! The reference is not a path. A name with a separator or a `..` is refused
//! before it is joined, so an arbiter cannot name a file outside the directory —
//! a hygiene rule, not a security boundary: an arbiter is trusted as much as the
//! body is (`docs/interrupt.md` §7.6).

use std::path::PathBuf;

use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_vm::io::{LoadFileError, read_image};

/// What a host can load on an arbiter's behalf.
pub trait ActionResolver {
    /// The references a `Load` may name, sorted.
    fn actions(&self) -> Vec<Box<str>>;

    fn resolve(&self, action: &str) -> Result<VerifiedImage, ActionError>;
}

/// A flat directory of `.csp` images.
pub struct ActionDirectory {
    root: PathBuf,
}

impl ActionDirectory {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ActionError> {
        let root = root.into();
        let metadata = std::fs::metadata(&root).map_err(|source| ActionError::Io {
            path: root.clone(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(ActionError::NotADirectory(root));
        }
        Ok(Self { root })
    }

    fn path_of(&self, action: &str) -> Result<PathBuf, ActionError> {
        let is_name = !action.is_empty()
            && action != "."
            && action != ".."
            && !action.contains('/')
            && !action.contains('\\');
        if !is_name {
            return Err(ActionError::Outside(action.to_owned()));
        }
        Ok(self.root.join(action))
    }
}

impl ActionResolver for ActionDirectory {
    fn actions(&self) -> Vec<Box<str>> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut names: Vec<Box<str>> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("csp") {
                    return None;
                }
                let name = path.file_name()?.to_string_lossy().into_owned();
                Some(name.into_boxed_str())
            })
            .collect();
        names.sort();
        names
    }

    fn resolve(&self, action: &str) -> Result<VerifiedImage, ActionError> {
        let path = self.path_of(action)?;
        Ok(read_image(&path)?)
    }
}

/// Why an action reference could not become an image.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("`{0}` is not a single file name inside the action directory")]
    Outside(String),
    #[error("`{0}` is not a directory")]
    NotADirectory(PathBuf),
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Load(#[from] LoadFileError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberspine_testkit as testkit;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("csp-actions-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the scratch directory is created");
        root
    }

    #[test]
    fn a_directory_resolves_the_images_it_lists() {
        let root = scratch("lists");
        let image = testkit::action(vec![testkit::halt()], &testkit::rig(1));
        std::fs::write(root.join("walk.csp"), image.to_bytes()).expect("the image is written");
        std::fs::write(root.join("notes.txt"), b"not an image").expect("the note is written");

        let resolver = ActionDirectory::open(&root).expect("the directory opens");

        assert_eq!(resolver.actions(), vec!["walk.csp".into()]);
        assert!(resolver.resolve("walk.csp").is_ok());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_reference_cannot_leave_the_directory() {
        let root = scratch("outside");
        let resolver = ActionDirectory::open(&root).expect("the directory opens");

        for name in ["../walk.csp", "a/walk.csp", "a\\walk.csp", "..", ""] {
            assert!(
                matches!(resolver.resolve(name), Err(ActionError::Outside(_))),
                "`{name}` is not a name"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_action_is_a_load_error() {
        let root = scratch("missing");
        let resolver = ActionDirectory::open(&root).expect("the directory opens");

        assert!(matches!(
            resolver.resolve("absent.csp"),
            Err(ActionError::Load(_))
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let root = scratch("file");
        let file = root.join("walk.csp");
        std::fs::write(&file, b"x").expect("the file is written");

        assert!(matches!(
            ActionDirectory::open(&file),
            Err(ActionError::NotADirectory(_))
        ));

        let _ = std::fs::remove_dir_all(&root);
    }
}
