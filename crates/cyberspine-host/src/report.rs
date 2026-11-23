//! What the host did with one ruling, kept as data so a terminal, a screen and
//! a test all read the same stream.

use cyberspine_arbiter::{Action, EventId, Verdict};
use cyberspine_diag::{Anchor, Cause};
use cyberspine_vm::InstanceId;

/// The last `Unfinished` round of an event, as the stderr line quotes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundTrace {
    pub source: Box<str>,
    pub note: Box<str>,
}

/// One observation from the in-flight table. Rounds carry the actions they
/// applied so a caller can show what the arbiter did, not just what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Interrupt {
        event: EventId,
        instance: InstanceId,
        action: Box<str>,
        pc: usize,
        tick: u64,
        reason: Box<str>,
        /// The source position the debug segment maps `pc` to, resolved against
        /// the image's directory (`docs/asm.md` §8).
        site: Option<Anchor>,
    },
    Round {
        event: EventId,
        tick: u64,
        round: usize,
        source: Box<str>,
        note: Box<str>,
        actions: Vec<Action>,
    },
    Finished {
        event: EventId,
        tick: u64,
        rounds: usize,
        last: Option<RoundTrace>,
        note: Box<str>,
        actions: Vec<Action>,
        verdict: Verdict,
    },
    Unknown {
        event: EventId,
    },
    /// The arbiter has taken this event and will never answer it; the host
    /// settled it with the fallback verdict.
    Abandoned {
        event: EventId,
    },
    Action {
        event: EventId,
        message: Box<str>,
    },
    Carry {
        event: EventId,
        message: Box<str>,
    },
    Verdict {
        message: Box<str>,
    },
    Failed {
        cause: Cause,
    },
}

impl Report {
    /// Whether this report has nothing to say on stderr.
    ///
    /// A direct `Finished` with no trail and no note is the built-in arbiter's
    /// ordinary answer; printing "finished after 0 rounds" would be noise.
    pub fn is_silent(&self) -> bool {
        matches!(self, Self::Finished { last: None, note, .. } if note.is_empty())
    }
}
