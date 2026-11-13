//! What the host did with one ruling, kept as data so a terminal, a screen and
//! a test all read the same stream.

use core::fmt;

use cyberspine_arbiter::{Action, EventId, Verdict};
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
        message: Box<str>,
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

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interrupt {
                instance,
                action,
                pc,
                tick,
                reason,
                ..
            } => write!(
                f,
                "interrupt: instance {instance} ({action}) at pc {pc} tick {tick}: {reason}"
            ),
            Self::Round {
                event,
                source,
                note,
                ..
            } => write!(f, "interrupt {event}: {source}: {note}"),
            Self::Finished {
                event,
                rounds,
                last: Some(last),
                note,
                ..
            } => {
                write!(
                    f,
                    "interrupt {event}: finished after {rounds} round(s), last: {}: {}",
                    last.source, last.note
                )?;
                if !note.is_empty() && note.as_ref() != last.note.as_ref() {
                    write!(f, " · {note}")?;
                }
                Ok(())
            }
            Self::Finished {
                event,
                last: None,
                note,
                ..
            } if !note.is_empty() => write!(f, "interrupt {event}: {note}"),
            Self::Finished { .. } => Ok(()),
            Self::Unknown { event } => {
                write!(f, "warning: the arbiter answered unknown event {event}")
            }
            Self::Action { event, message } => write!(f, "warning: event {event}: {message}"),
            Self::Carry { event, message } => {
                write!(f, "warning: event {event} could not be carried: {message}")
            }
            Self::Verdict { message } => {
                write!(f, "warning: the interrupt verdict did not apply: {message}")
            }
            Self::Failed { message } => write!(f, "warning: the arbiter failed: {message}"),
        }
    }
}
