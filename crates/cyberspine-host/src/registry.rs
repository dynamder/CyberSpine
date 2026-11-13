//! The host's record of what is loaded, addressed both by `InstanceId` and by
//! the arbiter's `InstanceRef`.

use cyberspine_arbiter::{InstanceRef, InstanceView};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Fingerprint;
use cyberspine_vm::InstanceId;

/// One loaded instance. The arbiter never sees this type; it sees the
/// [`InstanceView`] built from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub id: InstanceId,
    pub layer: u16,
    pub action: Box<str>,
    hash: [u8; 32],
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

    pub fn push(&mut self, id: InstanceId, layer: u16, hash: [u8; 32], action: Box<str>) {
        self.instances.push(Loaded {
            id,
            layer,
            action,
            hash,
        });
    }

    pub(crate) fn remove(&mut self, id: InstanceId) {
        self.instances.retain(|loaded| loaded.id != id);
    }

    pub(crate) fn action_of(&self, id: InstanceId) -> Option<&str> {
        self.loaded_by_id(id).map(|loaded| &*loaded.action)
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
