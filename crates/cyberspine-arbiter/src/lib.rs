//! Adjudicators: the host's second plugin boundary (`docs/interrupt.md` §7).
//!
//! An [`Arbiter`] turns one [`InterruptEvent`] into a stream of [`Ruling`]s that
//! ends with exactly one [`Completion::Finished`]. How many rounds that takes,
//! and what happens between them, is the arbiter's own policy: a fallback rule
//! and an LLM are two rounds of the same stream, not two host features.
//!
//! # Two orthogonal axes
//!
//! A ruling does two independent things. [`Action`]s shape the world around the
//! interrupt — load an instance, unload one, silence one. [`Completion`] says
//! whether the ticket's fate is decided. `Switch` is not a third verdict: it is
//! [`Action::Load`] followed by [`Verdict::Stop`], which is what it always was
//! (`docs/interrupt.md` §7.1).
//!
//! The host owns the ticket and the instance state. An arbiter only sees the
//! borrowed view in [`InterruptEvent`] and answers with [`Ruling`]s, so it
//! cannot read or write an instance directly.
//!
//! Three packagings share this one trait, the same way bodies share
//! `cyberspine_vm::Backend`: linked (implement it), dynamic library, and remote.
//! A body and an arbiter differ in what they answer, not in how they are loaded.

use serde::{Deserialize, Serialize};

pub mod abi;
pub mod remote;
pub mod server;
pub mod spec;
pub mod wire;

#[cfg(feature = "dylib")]
pub mod dylib;

#[cfg(feature = "export")]
pub mod export;

#[cfg(feature = "rpc")]
pub mod rpc;

pub use remote::{RemoteArbiter, Transport};
pub use server::Server;
pub use spec::{ArbiterSpec, SpecError};

#[cfg(feature = "rpc")]
pub use rpc::RpcArbiter;

/// The host's name for one interrupt, stable for the life of the ticket.
///
/// It is an arbiter's only way to refer to an event: a [`Decision`] echoes the id
/// it was delivered, and the host rejects an id it never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EventId(u64);

impl EventId {
    pub const fn new(seq: u64) -> Self {
        Self(seq)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for EventId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A stable handle on one loaded instance, as the arbiter sees it.
///
/// The host never reuses a handle, so an action may name an instance across
/// rounds without it silently becoming someone else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InstanceRef(u64);

impl InstanceRef {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for InstanceRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One instance visible at the moment of the interrupt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceView {
    pub reference: InstanceRef,
    pub action: Box<str>,
    pub layer: u16,
}

/// Everything an arbiter is allowed to know about one interrupt.
///
/// It carries an id instead of the host's ticket: the ticket cannot be built
/// outside the session, and an arbiter that could hold one could replay it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptEvent {
    pub event: EventId,
    /// The `INTR` instruction; the resume point is `pc + 1`.
    pub pc: usize,
    pub tick: u64,
    /// The stored reason template — a stable key for a programmatic handler.
    pub format: Box<str>,
    /// The interpolated text — what a human or an LLM reads.
    pub reason: Box<str>,
    /// The action the interrupted instance was running.
    pub action: Box<str>,
    /// Every instance visible at the moment of the interrupt.
    pub instances: Vec<InstanceView>,
    /// The action references the host could load. A hint, not an allowlist:
    /// `Load` is still validated by the host's loader.
    pub actions: Vec<Box<str>>,
    pub image_hash: [u8; 32],
}

/// An atomic operation on the world around the interrupt.
///
/// An action never decides the ticket's fate; that is [`Completion`]'s job.
/// `Unload` of the interrupted instance is therefore not an action — it is
/// [`Verdict::Stop`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Resolve an action reference and load it as a new instance.
    Load { action: Box<str> },
    /// Remove an instance.
    Unload { instance: InstanceRef },
    /// Stop merging the instance's commands. It stays alive and adjudicable;
    /// the body keeps the last command for every muscle it no longer claims.
    Silence { instance: InstanceRef },
    /// Undo [`Action::Silence`].
    Restore { instance: InstanceRef },
}

/// The fate of the interrupted instance. Exactly two atomic outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Release the instance; it resumes after the `INTR`.
    Continue,
    /// Unload the instance. Its commands stop merging, and the body holds the
    /// last published vector (`docs/isa.md` §G10).
    Stop,
}

/// Whether a ruling is the last one for its event.
///
/// The marker sits on the ruling rather than in the verdict: the two verdicts
/// stay atomic, and "this round did not decide" is not a third one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    /// This round did not decide. The ticket stays live; the next round may
    /// still answer it.
    Unfinished,
    /// The verdict is final; the ticket is consumed.
    Finished(Verdict),
}

/// One round's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruling {
    /// Who answered, as the arbiter names it. Observability only — never part
    /// of the replay input.
    pub source: Box<str>,
    /// What the round concluded, for the log. Observability only.
    pub note: Box<str>,
    /// Applied left to right, before the completion. Failures are reported and
    /// skipped; they never change whether the completion is applied.
    pub actions: Vec<Action>,
    pub completion: Completion,
}

impl Ruling {
    /// A round that decides nothing and changes nothing.
    pub fn unfinished(source: impl Into<Box<str>>, note: impl Into<Box<str>>) -> Self {
        Self {
            source: source.into(),
            note: note.into(),
            actions: Vec::new(),
            completion: Completion::Unfinished,
        }
    }

    /// A round that applies `actions` and then settles the ticket.
    pub fn finished(actions: Vec<Action>, verdict: Verdict) -> Self {
        Self {
            source: Box::default(),
            note: Box::default(),
            actions,
            completion: Completion::Finished(verdict),
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.completion, Completion::Finished(_))
    }
}

/// One stream element: an event id and the round it answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub event: EventId,
    pub ruling: Ruling,
}

/// What an arbiter has to say right now.
///
/// An empty batch is normal: it means "no round has finished yet", not "the
/// interrupt was dropped".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub decisions: Vec<Decision>,
    pub failure: Option<ArbiterError>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty() && self.failure.is_none()
    }
}

impl From<Vec<Decision>> for Batch {
    fn from(decisions: Vec<Decision>) -> Self {
        Self {
            decisions,
            failure: None,
        }
    }
}

/// Why an arbiter stopped being able to adjudicate.
///
/// Reporting it obliges the arbiter to have synthesized `Finished(Stop)` for
/// every event it still owed an answer; the host finishes the rest.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ArbiterError(pub Box<str>);

/// A host's adjudicator (`docs/interrupt.md` §7.3).
///
/// The contract is a stream, not a handshake: every delivered event eventually
/// gets exactly one [`Completion::Finished`], preceded by any number of
/// [`Completion::Unfinished`] rounds. `take` returning an empty batch is a
/// normal state, so neither method may block the tick loop.
///
/// `take` is given the host's current tick. A synchronous arbiter ignores it; an
/// asynchronous one uses it for deadlines, and a replaying one uses it to
/// release each recorded round at the tick it was originally applied.
pub trait Arbiter {
    fn deliver(&mut self, event: InterruptEvent);

    fn take(&mut self, now: u64) -> Batch;
}

impl<T: Arbiter + ?Sized> Arbiter for Box<T> {
    fn deliver(&mut self, event: InterruptEvent) {
        (**self).deliver(event);
    }

    fn take(&mut self, now: u64) -> Batch {
        (**self).take(now)
    }
}
