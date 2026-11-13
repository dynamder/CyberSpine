//! The demo's linked arbiter: an unsigned junction, a back-off, then a turn.
//!
//! Round one silences the asker and loads a one-tick reverse. After a few ticks
//! round two loads a `maze_left` or `maze_right` image and stops the asker. The
//! two images are the same program with a different `turn` initial value, so the
//! arbiter's choice is the direction.

use std::collections::VecDeque;

use cyberspine_arbiter::{
    Action, Arbiter, Batch, Completion, Decision, EventId, InstanceRef, InterruptEvent, Ruling,
    Verdict,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Left,
    Right,
}

impl Turn {
    fn action(self) -> &'static str {
        match self {
            Self::Left => "maze_left.csa",
            Self::Right => "maze_right.csa",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Left => "the unsigned junction: turning left",
            Self::Right => "the unsigned junction: turning right",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Backs the asker off, waits, then hands the body to a left/right image.
pub struct Junction {
    fresh: VecDeque<(EventId, u64, InstanceRef)>,
    holding: VecDeque<(EventId, u64)>,
    retire: VecDeque<EventId>,
    next: Turn,
}

impl Junction {
    pub fn new() -> Self {
        Self {
            fresh: VecDeque::new(),
            holding: VecDeque::new(),
            retire: VecDeque::new(),
            next: Turn::Left,
        }
    }
}

impl Default for Junction {
    fn default() -> Self {
        Self::new()
    }
}

impl Arbiter for Junction {
    fn deliver(&mut self, event: InterruptEvent) {
        if matches!(
            event.action.as_ref(),
            "maze.csa" | "maze_left.csa" | "maze_right.csa"
        ) {
            self.fresh
                .push_back((event.event, event.tick, event.instance));
        } else {
            self.retire.push_back(event.event);
        }
    }

    fn take(&mut self, now: u64) -> Batch {
        let mut decisions = Vec::new();

        for event in self.retire.drain(..) {
            decisions.push(Decision {
                event,
                ruling: finished("not the unsigned junction; stopping", Verdict::Stop),
            });
        }

        for (event, tick, instance) in self.fresh.drain(..) {
            decisions.push(Decision {
                event,
                ruling: Ruling {
                    source: "junction".into(),
                    note: "the unsigned junction: backing off".into(),
                    actions: vec![
                        Action::Silence { instance },
                        Action::Load {
                            action: "reverse.csa".into(),
                        },
                    ],
                    completion: Completion::Unfinished,
                },
            });
            self.holding.push_back((event, tick));
        }

        let mut keep = VecDeque::new();
        for (event, tick) in self.holding.drain(..) {
            if now > tick + 3 {
                let turn = self.next;
                self.next = turn.other();
                decisions.push(Decision {
                    event,
                    ruling: Ruling {
                        source: "junction".into(),
                        note: turn.note().into(),
                        actions: vec![Action::Load {
                            action: turn.action().into(),
                        }],
                        completion: Completion::Finished(Verdict::Stop),
                    },
                });
            } else {
                keep.push_back((event, tick));
            }
        }
        self.holding = keep;

        Batch::from(decisions)
    }
}

fn finished(note: &str, verdict: Verdict) -> Ruling {
    Ruling {
        source: "junction".into(),
        note: note.into(),
        actions: Vec::new(),
        completion: Completion::Finished(verdict),
    }
}
