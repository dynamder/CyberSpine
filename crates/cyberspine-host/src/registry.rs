//! The host's record of what is loaded, addressed both by `InstanceId` and by
//! the arbiter's `InstanceRef`.

use std::path::Path;

use cyberspine_arbiter::{InstanceRef, InstanceView};
use cyberspine_core::binfmt::DebugInfo;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Fingerprint;
use cyberspine_diag::{Anchor, Site};
use cyberspine_vm::InstanceId;

/// One loaded instance. The arbiter never sees this type; it sees the
/// [`InstanceView`] built from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub id: InstanceId,
    pub layer: u16,
    pub action: Box<str>,
    hash: [u8; 32],
    /// The image's debug segment, when it is a debug build (`docs/asm.md` §8).
    /// Kept beside the instance because the image itself lives in the session,
    /// which is where the host cannot reach for a rendering-only lookup.
    debug: Option<DebugInfo>,
}

/// Every instance this host has loaded, in load order.
///
/// The handle an arbiter uses is the one it was given in `InterruptEvent`;
/// this table is the only place that maps it back to the session's id.
#[derive(Debug, Clone)]
pub struct Registry {
    names: Vec<String>,
    rig: Fingerprint,
    instances: Vec<Loaded>,
}

impl Registry {
    pub fn new(image: &VerifiedImage) -> Self {
        Self {
            names: image
                .muscles()
                .iter()
                .map(|muscle| image.name(muscle.name).to_owned())
                .collect(),
            rig: image.header().rig,
            instances: Vec::new(),
        }
    }

    /// The muscle names, in the image's order, for the host's trace.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn rig(&self) -> Fingerprint {
        self.rig
    }

    /// The instances a run summary prints.
    pub fn loaded(&self) -> &[Loaded] {
        &self.instances
    }

    pub fn push(
        &mut self,
        id: InstanceId,
        layer: u16,
        hash: [u8; 32],
        action: Box<str>,
        debug: Option<DebugInfo>,
    ) {
        self.instances.push(Loaded {
            id,
            layer,
            action,
            hash,
            debug,
        });
    }

    /// The debug segment of the instance's image, when it has one.
    pub fn debug_of(&self, id: InstanceId) -> Option<&DebugInfo> {
        self.loaded_by_id(id)
            .and_then(|loaded| loaded.debug.as_ref())
    }

    pub(crate) fn remove(&mut self, id: InstanceId) {
        self.instances.retain(|loaded| loaded.id != id);
    }

    /// The image path the instance was loaded from, as the user spelled it.
    pub fn action_of(&self, id: InstanceId) -> Option<&str> {
        self.loaded_by_id(id).map(|loaded| &*loaded.action)
    }

    /// Where instruction `pc` of `id` came from, when the image is a debug build
    /// that covers it.
    ///
    /// This is the only place a `pc` becomes a source position: the debug segment
    /// stores the source path relative to the image, so resolving it — not just
    /// printing it — belongs where the image path is known.
    pub fn source_of(&self, id: InstanceId, pc: usize) -> Option<Anchor> {
        let loaded = self.loaded_by_id(id)?;
        let debug = loaded.debug.as_ref()?;
        let line = debug.line(pc)?;
        Some(Anchor::new(
            resolve_debug_path(&loaded.action, debug.source()),
            Site::Source {
                line: line.line,
                column: u32::from(line.column),
            },
        ))
    }

    pub(crate) fn id_of(&self, reference: InstanceRef) -> Option<InstanceId> {
        self.instances
            .iter()
            .find(|loaded| loaded.id.get() == reference.get())
            .map(|loaded| loaded.id)
    }

    pub(crate) fn loaded_by_id(&self, id: InstanceId) -> Option<&Loaded> {
        self.instances.iter().find(|loaded| loaded.id == id)
    }

    /// The interrupted instance's action and the instances visible beside it.
    pub(crate) fn snapshot(&self, id: InstanceId) -> (Box<str>, Vec<InstanceView>, [u8; 32]) {
        let action = self.action_of(id).unwrap_or_default().into();
        let instances = self
            .instances
            .iter()
            .map(|loaded| InstanceView {
                reference: InstanceRef::new(loaded.id.get()),
                action: loaded.action.clone(),
                layer: loaded.layer,
            })
            .collect();
        let hash = self
            .loaded_by_id(id)
            .map(|loaded| loaded.hash)
            .unwrap_or([0; 32]);
        (action, instances, hash)
    }
}

/// The source path a debug segment records is relative to the image's
/// directory; a reader needs a path it can open from where it is.
fn resolve_debug_path(image: &str, source: &str) -> Box<str> {
    let image = Path::new(image);
    match image.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            dir.join(source).display().to_string().into_boxed_str()
        }
        _ => source.into(),
    }
}
