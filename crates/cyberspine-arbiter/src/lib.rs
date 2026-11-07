//! Adjudicators: the host's second plugin boundary (`docs/interrupt.md` §7).
//!
//! An [`Arbiter`] turns one [`InterruptEvent`] into a stream of [`Ruling`]s that
//! ends with exactly one [`Ruling::Finished`]. How many rounds that takes, and
//! what happens between them, is the arbiter's own policy: a fallback rule and an
//! LLM are two rounds of the same stream, not two host features.
//!
//! The host owns the ticket and the instance state. An arbiter only sees the
//! borrowed view in [`InterruptEvent`] and can only answer with a [`Ruling`], so
//! it cannot read or write an instance. `Switch` names an action; resolving the
//! name is the host's job, because the VM knows nothing about actions.
//!
//! Three packagings share this one trait, the same way bodies share
//! `cyberspine_vm::Backend`: linked (implement it), dynamic library, and remote.
//! A body and an arbiter differ in what they answer, not in how they are loaded.

/// The host's name for one interrupt, stable for the life of the ticket.
///
/// It is an arbiter's only way to refer to an event: a [`Decision`] echoes the id
/// it was delivered, and the host rejects an id it never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// Everything an arbiter is allowed to know about one interrupt.
///
/// It carries an id instead of the host's ticket: the ticket cannot be built
/// outside the session, and an arbiter that could hold one could replay it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The actions visible at the moment of the interrupt.
    pub actions: Vec<Box<str>>,
    pub image_hash: [u8; 32],
}

/// One round's answer.
///
/// The marker sits outside the verdict: the three verdicts stay atomic, and
/// "this round did not decide" is not a fourth one. Only `Finished` consumes the
/// ticket; an `Unfinished` round leaves the instance parked under the same id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ruling {
    /// This round did not decide. The ticket stays live and the next round may
    /// still answer it.
    Unfinished { source: Box<str>, note: Box<str> },
    /// The verdict is final.
    Finished(Verdict),
}

/// An atomic verdict, the three answers a finished stream can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    Stop,
    Switch { action: Box<str> },
}

/// One stream element: an event id and the round it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// gets exactly one [`Ruling::Finished`], preceded by any number of
/// [`Ruling::Unfinished`]. `take` returning an empty batch is a normal state, so
/// neither method may block the tick loop.
pub trait Arbiter {
    fn deliver(&mut self, event: InterruptEvent);

    fn take(&mut self) -> Batch;
}
