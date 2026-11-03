//! The body compiled into `cyinevm`.
//!
//! A linked body is a type, not a string: `Body` and `Policy` are associated
//! types, so "a linked body driven under the wrong identity policy" is not a
//! state this module can represent. The official binary uses [`Official`]; a
//! build with a real body replaces that alias.

use cyberspine_core::binfmt::image::SensorSource;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::{Name, Rig};
use cyberspine_vm::{Backend, BackendError, DummyBackend, Host, Permissive, RigPolicy};

/// What a linked body tells the operator when it is selected without a spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Silent,
    Warn(&'static str),
}

/// A body compiled into the runtime.
pub trait LinkedBody {
    type Body: Backend;
    type Policy: RigPolicy;

    const NOTICE: Notice;

    fn open(image: &VerifiedImage) -> Result<Self::Body, BackendError>;

    fn host(body: Self::Body) -> Host<Self::Body, Self::Policy> {
        Host::with_policy(body)
    }
}

/// The official linked body: a zero body whose shape is read from the image.
pub struct Dummy;

impl LinkedBody for Dummy {
    type Body = DummyBackend;
    type Policy = Permissive;

    const NOTICE: Notice =
        Notice::Warn("no --backend given; using the built-in dummy body (rig check disabled)");

    fn open(image: &VerifiedImage) -> Result<DummyBackend, BackendError> {
        dummy_from_image(image)
    }
}

/// The body `cyinevm` uses when no `--backend` is given. Replace this alias to
/// link a real body; `Dummy` is the official choice.
pub type Official = Dummy;

/// Build a zero body with the image's shape.
///
/// Its rig cannot carry the image's identity: the image has a fingerprint, not
/// the manifest text, and `BoundFingerprint` deliberately has no constructor
/// from bytes. The dummy therefore runs under [`Permissive`] and the notice says
/// the rig check is disabled.
fn dummy_from_image(image: &VerifiedImage) -> Result<DummyBackend, BackendError> {
    let mut rig = Rig::new("dummy", 0);

    for muscle in image.muscles() {
        rig = rig.muscle(name(image.name(muscle.name))?);
    }
    for sensor in image.sensors_f() {
        if sensor.source == SensorSource::Backend {
            rig = rig.sensor_f(name(image.name(sensor.name))?);
        }
    }
    for sensor in image.sensors_i() {
        if sensor.source == SensorSource::Backend {
            rig = rig.sensor_i(name(image.name(sensor.name))?);
        }
    }

    Ok(DummyBackend::new(rig))
}

fn name(text: &str) -> Result<Name, BackendError> {
    Name::new(text).map_err(BackendError::new)
}
