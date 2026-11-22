use serde::Serialize;

/// How a run ended.
///
/// `Budget` and `Halted` are both successes, and telling them apart is the
/// point: "the action finished" and "the tick budget ran out while it was still
/// going" are different answers to the same question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// `--ticks N` ran out.
    Budget,
    /// Every instance halted and `--until-halt` was satisfied.
    Halted,
    /// Ctrl-C.
    Stopped,
    /// An instance faulted.
    Faulted,
    /// A body, arbiter or file failure ended the run.
    Environment,
}

impl Outcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::Halted => "halted",
            Self::Stopped => "stopped",
            Self::Faulted => "faulted",
            Self::Environment => "environment",
        }
    }
}

/// What was loaded. The report describes the session, not the body's state:
/// a body is opaque below the rig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceReport {
    pub id: u64,
    pub layer: u16,
    pub action: String,
}

/// The terminal state of one run (`docs/diagnostics.md` §5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunReport {
    pub outcome: Outcome,
    pub ticks: u64,
    /// Seconds per tick: the session's fixed step, not wall-clock pacing.
    pub dt: f32,
    pub overruns: u64,
    pub instances: Vec<InstanceReport>,
}
