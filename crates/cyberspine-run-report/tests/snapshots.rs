//! The machine contract of the run output, as bytes.
//!
//! An agent reads `--report json` and the event stream, not the prose. A field
//! renamed, an `outcome` spelling changed or a new one added is a breaking
//! change to a consumer, so the snapshot — not a field-by-field assertion that
//! can leave a field uncovered — is what pins it.
//!
//! The diagnostic shape is pinned in `cyberspine-report`; this crate renders
//! the other two records.

use cyberspine_arbiter::EventId;
use cyberspine_diag::{Cause, Code};
use cyberspine_host::Report;
use cyberspine_run_report::{Format, InstanceReport, Outcome, Renderer, RunReport};

fn run_json(report: &RunReport) -> String {
    let mut out = Vec::new();
    Renderer::new(Format::Json)
        .run(report, &mut out)
        .expect("a Vec is writable");
    String::from_utf8(out).expect("the renderer writes UTF-8")
}

fn event_json(reports: &[Report]) -> String {
    let mut renderer = Renderer::new(Format::Json);
    let mut out = Vec::new();
    for report in reports {
        renderer.event(report, &mut out).expect("a Vec is writable");
    }
    String::from_utf8(out).expect("the renderer writes UTF-8")
}

fn instance_report() -> Vec<InstanceReport> {
    vec![
        InstanceReport {
            id: 0,
            layer: 4,
            action: "probe.csp".into(),
        },
        InstanceReport {
            id: 1,
            layer: 4,
            action: "idle.csp".into(),
        },
    ]
}

#[test]
fn a_budget_run_reports_every_field() {
    let report = RunReport {
        outcome: Outcome::Budget,
        ticks: 3,
        dt: 0.5,
        overruns: 1,
        instances: instance_report(),
    };

    insta::assert_snapshot!("run_budget", run_json(&report));
}

#[test]
fn a_faulted_run_reports_the_same_shape() {
    let report = RunReport {
        outcome: Outcome::Faulted,
        ticks: 12,
        dt: 1.0 / 60.0,
        overruns: 0,
        instances: instance_report(),
    };

    insta::assert_snapshot!("run_faulted", run_json(&report));
}

#[test]
fn an_environment_failure_reports_the_same_shape() {
    let report = RunReport {
        outcome: Outcome::Environment,
        ticks: 0,
        dt: 0.5,
        overruns: 0,
        instances: Vec::new(),
    };

    insta::assert_snapshot!("run_environment", run_json(&report));
}

#[test]
fn the_event_stream_is_one_object_per_line() {
    let event = EventId::new(7);
    let reports = [
        Report::Unknown { event },
        Report::Action {
            event,
            message: "loaded `next.csp`".into(),
        },
        Report::Carry {
            event,
            message: "the ticket was stale".into(),
        },
        Report::Verdict {
            message: "continue".into(),
        },
        Report::Failed {
            cause: Cause::new(Code::new("arbiter.failed"), "the arbiter died")
                .with_help("restart the arbiter process"),
        },
    ];

    insta::assert_snapshot!("events", event_json(&reports));

    let lines = event_json(&reports);
    for line in lines.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("every line is one object");
    }
}
