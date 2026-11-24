use std::io::{self, Write};

use cyberspine_arbiter::{Action, Verdict};
use cyberspine_host::{Report, RoundTrace};
use cyberspine_report::AnchorJson;
use serde::Serialize;

/// The interrupt/ruling stream as it happens.
///
/// These are not diagnostics: an interrupt is normal traffic, and a ruling is the
/// host answering it. They still go through the same renderer so that `--json`
/// describes the whole run in one vocabulary instead of half of it.
pub fn render(report: &Report, out: &mut impl Write) -> io::Result<()> {
    match report {
        Report::Interrupt {
            instance,
            action,
            pc,
            tick,
            reason,
            site,
            ..
        } => {
            write!(out, "interrupt: instance {instance} ({action}) at pc {pc}")?;
            if let Some(site) = site {
                write!(out, " ({site})")?;
            }
            writeln!(out, " tick {tick}: {reason}")
        }
        Report::Round {
            event,
            source,
            note,
            ..
        } => writeln!(out, "interrupt {event}: {source}: {note}"),
        Report::Finished {
            event,
            rounds,
            last: Some(last),
            note,
            ..
        } => {
            write!(
                out,
                "interrupt {event}: finished after {rounds} round(s), last: {}: {}",
                last.source, last.note
            )?;
            if !note.is_empty() && note.as_ref() != last.note.as_ref() {
                write!(out, " · {note}")?;
            }
            writeln!(out)
        }
        Report::Finished {
            event,
            last: None,
            note,
            ..
        } if !note.is_empty() => writeln!(out, "interrupt {event}: {note}"),
        Report::Finished { .. } => Ok(()),
        Report::Unknown { event } => {
            writeln!(out, "warning: the arbiter answered unknown event {event}")
        }
        Report::Abandoned { event } => writeln!(
            out,
            "warning: the arbiter will never answer event {event}; the fallback verdict was applied"
        ),
        Report::Action { event, message } => writeln!(out, "warning: event {event}: {message}"),
        Report::Carry { event, message } => {
            writeln!(
                out,
                "warning: event {event} could not be carried: {message}"
            )
        }
        Report::Verdict { message } => {
            writeln!(
                out,
                "warning: the interrupt verdict did not apply: {message}"
            )
        }
        Report::Failed { cause } => writeln!(out, "warning: the arbiter failed: {cause}"),
    }
}

/// One event, one JSON line.
pub fn render_json(report: &Report, out: &mut impl Write) -> io::Result<()> {
    #[derive(Serialize)]
    struct Record<'a> {
        schema: u32,
        kind: &'static str,
        #[serde(flatten)]
        event: EventJson<'a>,
    }

    let record = Record {
        schema: cyberspine_diag::SCHEMA,
        kind: "event",
        event: EventJson::of(report),
    };
    serde_json::to_writer(&mut *out, &record).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

#[derive(Debug, Serialize)]
#[serde(tag = "what", rename_all = "snake_case")]
enum EventJson<'a> {
    Interrupt {
        event: u64,
        instance: u64,
        action: &'a str,
        pc: usize,
        tick: u64,
        reason: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        site: Option<AnchorJson<'a>>,
    },
    Round {
        event: u64,
        tick: u64,
        round: usize,
        source: &'a str,
        note: &'a str,
        actions: &'a [Action],
    },
    Finished {
        event: u64,
        tick: u64,
        rounds: usize,
        last: Option<LastJson<'a>>,
        note: &'a str,
        actions: &'a [Action],
        verdict: Verdict,
    },
    Unknown {
        event: u64,
    },
    Abandoned {
        event: u64,
    },
    Action {
        event: u64,
        message: &'a str,
    },
    Carry {
        event: u64,
        message: &'a str,
    },
    Verdict {
        message: &'a str,
    },
    Failed {
        code: &'a str,
        message: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        help: Option<&'a str>,
    },
}

#[derive(Debug, Serialize)]
struct LastJson<'a> {
    source: &'a str,
    note: &'a str,
}

impl<'a> EventJson<'a> {
    fn of(report: &'a Report) -> Self {
        match report {
            Report::Interrupt {
                event,
                instance,
                action,
                pc,
                tick,
                reason,
                site,
            } => Self::Interrupt {
                event: event.get(),
                instance: instance.get(),
                action,
                pc: *pc,
                tick: *tick,
                reason,
                site: site.as_ref().map(AnchorJson::of),
            },
            Report::Round {
                event,
                tick,
                round,
                source,
                note,
                actions,
            } => Self::Round {
                event: event.get(),
                tick: *tick,
                round: *round,
                source,
                note,
                actions,
            },
            Report::Finished {
                event,
                tick,
                rounds,
                last,
                note,
                actions,
                verdict,
            } => Self::Finished {
                event: event.get(),
                tick: *tick,
                rounds: *rounds,
                last: last.as_ref().map(LastJson::of),
                note,
                actions,
                verdict: *verdict,
            },
            Report::Unknown { event } => Self::Unknown { event: event.get() },
            Report::Abandoned { event } => Self::Abandoned { event: event.get() },
            Report::Action { event, message } => Self::Action {
                event: event.get(),
                message,
            },
            Report::Carry { event, message } => Self::Carry {
                event: event.get(),
                message,
            },
            Report::Verdict { message } => Self::Verdict { message },
            Report::Failed { cause } => Self::Failed {
                code: cause.code.as_str(),
                message: &cause.message,
                help: cause.help.as_deref(),
            },
        }
    }
}

impl<'a> LastJson<'a> {
    fn of(last: &'a RoundTrace) -> Self {
        Self {
            source: last.source.as_ref(),
            note: last.note.as_ref(),
        }
    }
}
