//! Recording and replaying the ruling stream.
//!
//! The record is the VM's external input: for every round the host applied, the
//! event it answered, the tick it applied it at, the [`Action`]s the round
//! carried and its [`Completion`]. `source` and `note` are deliberately absent —
//! who adjudicated is observability, not replay input.
//!
//! JSON Lines, one ruling per line, in the order the host applied them.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use cyberspine_arbiter::{
    Action, Arbiter, Batch, Completion, Decision, EventId, InterruptEvent, Ruling,
};
use serde::{Deserialize, Serialize};

/// One applied round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub event: u64,
    pub tick: u64,
    #[serde(default)]
    pub actions: Vec<Action>,
    pub completion: Completion,
}

impl Record {
    pub fn new(event: EventId, tick: u64, ruling: &Ruling) -> Self {
        Self {
            event: event.get(),
            tick,
            actions: ruling.actions.clone(),
            completion: ruling.completion,
        }
    }
}

/// A decision log being written.
pub struct Log {
    path: PathBuf,
    out: BufWriter<File>,
}

impl Log {
    pub fn create(path: &Path) -> Result<Self, LogError> {
        let file = File::create(path).map_err(|source| LogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            out: BufWriter::new(file),
        })
    }

    pub fn write(&mut self, record: &Record) -> Result<(), LogError> {
        serde_json::to_writer(&mut self.out, record).map_err(|source| LogError::Encode {
            path: self.path.clone(),
            source,
        })?;
        writeln!(self.out).map_err(|source| LogError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

/// An arbiter that answers from a recorded stream.
///
/// A recorded round is released only when the host's clock has reached the tick
/// it was applied at, so a round that originally landed later lands later again.
/// An event with no recorded round is never answered: the instance stays parked,
/// exactly as it did on the original run.
pub struct Replay {
    path: PathBuf,
    script: HashMap<u64, VecDeque<Record>>,
    live: Vec<EventId>,
    applied: usize,
    total: usize,
    orphans: usize,
}

impl Replay {
    pub fn load(path: &Path) -> Result<Self, LogError> {
        let file = File::open(path).map_err(|source| LogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = BufReader::new(file);

        let mut script: HashMap<u64, VecDeque<Record>> = HashMap::new();
        let mut total = 0usize;
        for (index, line) in reader.lines().enumerate() {
            let line = line.map_err(|source| LogError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let record: Record =
                serde_json::from_str(&line).map_err(|source| LogError::Decode {
                    path: path.to_path_buf(),
                    line: index + 1,
                    source,
                })?;
            total += 1;
            script.entry(record.event).or_default().push_back(record);
        }

        Ok(Self {
            path: path.to_path_buf(),
            script,
            live: Vec::new(),
            applied: 0,
            total,
            orphans: 0,
        })
    }

    /// Rounds in the file that this run never reached.
    pub fn unused(&self) -> usize {
        self.total - self.applied
    }

    /// Interrupts this run raised that the file says nothing about.
    pub fn orphans(&self) -> usize {
        self.orphans
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        if self.unused() > 0 {
            eprintln!(
                "warning: {} recorded round(s) in {} were never applied; the run diverged",
                self.unused(),
                self.path.display()
            );
        }
        if self.orphans > 0 {
            eprintln!(
                "warning: {} interrupt(s) had no record in {}; they stayed parked",
                self.orphans,
                self.path.display()
            );
        }
    }
}

impl Arbiter for Replay {
    fn deliver(&mut self, event: InterruptEvent) {
        if self.script.contains_key(&event.event.get()) {
            self.live.push(event.event);
        } else {
            self.orphans += 1;
        }
    }

    fn take(&mut self, now: u64) -> Batch {
        let mut decisions = Vec::new();
        let mut finished = Vec::new();

        for event in &self.live {
            let Some(rounds) = self.script.get_mut(&event.get()) else {
                continue;
            };
            while let Some(record) = rounds.front() {
                if record.tick > now {
                    break;
                }
                let record = rounds.pop_front().expect("a front exists");
                self.applied += 1;
                let done = matches!(record.completion, Completion::Finished(_));
                decisions.push(Decision {
                    event: *event,
                    ruling: Ruling {
                        source: "replay".into(),
                        note: Box::default(),
                        actions: record.actions,
                        completion: record.completion,
                    },
                });
                if done {
                    finished.push(*event);
                    break;
                }
            }
        }

        self.live.retain(|event| !finished.contains(event));
        Batch::from(decisions)
    }
}

/// Why a decision log could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: the record does not encode: {source}")]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path}: line {line} is not a decision record: {source}")]
    Decode {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberspine_arbiter::Verdict;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("csp-log-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the scratch directory is created");
        root
    }

    fn event(seq: u64) -> InterruptEvent {
        InterruptEvent {
            event: EventId::new(seq),
            instance: cyberspine_arbiter::InstanceRef::new(0),
            pc: 0,
            tick: 0,
            format: "x".into(),
            reason: "x".into(),
            action: "a.csp".into(),
            instances: Vec::new(),
            actions: Vec::new(),
            image_hash: [0; 32],
        }
    }

    fn ruling(completion: Completion) -> Ruling {
        Ruling {
            source: "test".into(),
            note: "noise".into(),
            actions: Vec::new(),
            completion,
        }
    }

    #[test]
    fn a_written_log_reads_back_as_the_same_stream() {
        let root = scratch("roundtrip");
        let path = root.join("run.jsonl");

        let mut log = Log::create(&path).expect("the log is created");
        log.write(&Record::new(
            EventId::new(3),
            5,
            &ruling(Completion::Unfinished),
        ))
        .expect("the round is written");
        log.write(&Record::new(
            EventId::new(3),
            9,
            &ruling(Completion::Finished(Verdict::Stop)),
        ))
        .expect("the round is written");
        drop(log);

        let mut replay = Replay::load(&path).expect("the log is read");
        replay.deliver(event(3));

        let early = replay.take(4);
        assert!(early.decisions.is_empty(), "a later round is not released");
        assert_eq!(replay.unused(), 2);

        let first = replay.take(5);
        assert_eq!(first.decisions.len(), 1);
        assert!(matches!(
            first.decisions[0].ruling.completion,
            Completion::Unfinished
        ));

        let second = replay.take(9);
        assert_eq!(second.decisions.len(), 1);
        assert!(matches!(
            second.decisions[0].ruling.completion,
            Completion::Finished(Verdict::Stop)
        ));
        assert_eq!(replay.unused(), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unrecorded_interrupt_stays_parked() {
        let root = scratch("orphan");
        let path = root.join("run.jsonl");
        Log::create(&path).expect("the log is created");

        let mut replay = Replay::load(&path).expect("the log is read");
        replay.deliver(event(7));

        assert!(replay.take(100).decisions.is_empty());
        assert_eq!(replay.orphans(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}
