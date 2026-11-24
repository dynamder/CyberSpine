//! A two-round arbiter that pins the stream contract across a boundary.

use cyberspine_arbiter::{
    Arbiter, ArbiterError, Batch, Decision, EventId, InterruptEvent, Ruling, Verdict,
};

/// The reason text that makes this fixture report a failure instead of ruling.
///
/// A real arbiter dies for its own reasons; a test needs one it can ask for, and
/// the interrupt's reason is the only channel an image has.
pub const FAILING_REASON: &str = "fail";

/// Answers every event in two rounds: `Unfinished`, then `Finished(verdict)`.
///
/// The second round is produced on a later `take`, so a test that only looks at
/// one reply cannot mistake the stream for a single decision.
pub struct Scripted {
    verdict: Verdict,
    fresh: Vec<EventId>,
    answered: Vec<EventId>,
    failing: bool,
}

impl Scripted {
    /// `config` selects the terminal verdict: `stop`, or anything else for
    /// `continue`.
    pub fn new(config: Option<&str>) -> Self {
        let verdict = match config {
            Some("stop") => Verdict::Stop,
            _ => Verdict::Continue,
        };
        Self {
            verdict,
            fresh: Vec::new(),
            answered: Vec::new(),
            failing: false,
        }
    }
}

impl Arbiter for Scripted {
    fn deliver(&mut self, event: InterruptEvent) {
        if event.reason.as_ref() == FAILING_REASON {
            self.failing = true;
            return;
        }
        self.fresh.push(event.event);
    }

    fn take(&mut self, _now: u64) -> Batch {
        if std::mem::take(&mut self.failing) {
            return Batch {
                decisions: Vec::new(),
                failure: Some(ArbiterError::new("the fixture was told to fail")),
                ..Batch::default()
            };
        }

        let mut decisions = Vec::new();

        for event in self.answered.drain(..) {
            decisions.push(Decision {
                event,
                ruling: Ruling::finished(Vec::new(), self.verdict),
            });
        }

        for event in self.fresh.drain(..) {
            decisions.push(Decision {
                event,
                ruling: Ruling::unfinished("fixture", "first pass"),
            });
            self.answered.push(event);
        }

        Batch::from(decisions)
    }
}
