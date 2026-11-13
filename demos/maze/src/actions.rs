//! The action references the junction arbiter may load, resolved in memory.

use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_host::{ActionError, ActionResolver};

/// The back-off and the two direction images.
pub struct ActionSet {
    reverse: VerifiedImage,
    left: VerifiedImage,
    right: VerifiedImage,
}

impl ActionSet {
    pub fn new(reverse: VerifiedImage, left: VerifiedImage, right: VerifiedImage) -> Self {
        Self {
            reverse,
            left,
            right,
        }
    }
}

impl ActionResolver for ActionSet {
    fn actions(&self) -> Vec<Box<str>> {
        vec![
            "maze_left.csa".into(),
            "maze_right.csa".into(),
            "reverse.csa".into(),
        ]
    }

    fn resolve(&self, action: &str) -> Result<VerifiedImage, ActionError> {
        match action {
            "reverse.csa" => Ok(self.reverse.clone()),
            "maze_left.csa" => Ok(self.left.clone()),
            "maze_right.csa" => Ok(self.right.clone()),
            other => Err(ActionError::Unknown(other.to_owned())),
        }
    }
}
