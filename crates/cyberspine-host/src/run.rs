//! A loading session plus the table of what it loaded.
//!
//! Every driver — `cyinevm`, the maze demo, a test — has to do the same three
//! things around a [`Session`]: hand it an image, remember which action that
//! image came from, and pump the arbiter after every tick. [`Run`] owns the pair
//! so the bookkeeping exists once.

use cyberspine_arbiter::Arbiter;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_vm::{Backend, BackendError, Host, InstanceId, LoadError, RigPolicy, Session};

use crate::Interrupts;
use crate::decisions::LogError;
use crate::registry::{Identity, Registry};
use crate::report::Report;

/// The host-side half of a run: the ticking session and the registry naming
/// what it holds. The arbiter stays separate because a run may not have one.
#[derive(Debug)]
pub struct Run<B: Backend, P: RigPolicy> {
    session: Session<B, P>,
    registry: Registry,
}

impl<B: Backend, P: RigPolicy> Run<B, P> {
    /// Load the first image and start the registry that describes it.
    ///
    /// The image is consumed by the session, so its identity is read here,
    /// before the hand-off. On failure the host comes back untouched.
    pub fn start(
        host: Host<B, P>,
        image: VerifiedImage,
        action: Box<str>,
    ) -> Result<(Self, InstanceId), (Host<B, P>, LoadError)> {
        let mut registry = Registry::new(&image);
        let identity = Identity::of(&image);
        let (session, id) = host.load(image)?;
        registry.push(id, identity, action);
        Ok((Self { session, registry }, id))
    }

    /// Load another action and record it.
    pub fn load(
        &mut self,
        image: VerifiedImage,
        action: Box<str>,
    ) -> Result<InstanceId, LoadError> {
        let identity = Identity::of(&image);
        let id = self.session.load(image)?;
        self.registry.push(id, identity, action);
        Ok(id)
    }

    /// Advance the clock, then let the arbiter answer whatever the tick raised.
    pub fn advance<A: Arbiter>(
        &mut self,
        seconds: f64,
        interrupts: &mut Interrupts<A>,
    ) -> Result<Vec<Report>, RunError> {
        self.session.advance(seconds)?;
        Ok(self.pump(interrupts)?)
    }

    /// Let the arbiter answer the tick that just ran.
    pub fn pump<A: Arbiter>(
        &mut self,
        interrupts: &mut Interrupts<A>,
    ) -> Result<Vec<Report>, LogError> {
        interrupts.pump(&mut self.session, &mut self.registry)
    }

    pub fn session(&self) -> &Session<B, P> {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session<B, P> {
        &mut self.session
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// Why one step could not complete: the body refused the tick, or the decision
/// log could not be written after it.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Log(#[from] LogError),
}
