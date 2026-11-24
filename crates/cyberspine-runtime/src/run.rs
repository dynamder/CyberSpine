use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cyberspine_arbiter::{
    Arbiter, ArbiterError, ArbiterSpec, Batch, Decision, InterruptEvent, NoticeSource,
    Ruling as ArbiterRuling, Verdict as ArbiterVerdict,
};
use cyberspine_backend::BackendSpec;
use cyberspine_diag::{Anchor, Code, Diagnostic, Diagnostics, Site};
use cyberspine_host::{
    ActionDirectory, ActionError, ActionResolver, Interrupts, Log, LogError, Registry, Replay,
    Report, into_diagnostic,
};
use cyberspine_report::{InstanceReport, Outcome, RunReport};
use cyberspine_vm::io::{LoadFileError, read_image};
use cyberspine_vm::{Backend, BackendError, Host, LoadError, RigPolicy, Session};
use thiserror::Error;

use crate::linked::{LinkedBody, Notice};
use crate::trace::Trace;

/// When a run ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Ticks(u64),
    UntilHalt,
    Never,
}

/// How the virtual clock is advanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    RealTime(Stop),
    UnpacedTicks(u64),
}

/// What the built-in arbiter does with an interrupt, and what the host falls
/// back to when an external arbiter is gone.
///
/// The built-in is the degenerate case of the stream: it answers every interrupt
/// with one `Finished` round and never asks anyone. An external arbiter
/// (`--interrupt`) may rule in several rounds, and `Stop` / `Continue` here only
/// decide what happens when it cannot answer at all (`docs/interrupt.md` §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnInterrupt {
    Stop,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub mode: RunMode,
    pub trace: bool,
    pub quiet: bool,
    pub on_interrupt: OnInterrupt,
    /// An external arbiter; `None` selects the built-in single-round one.
    pub interrupt: Option<ArbiterSpec>,
    /// The directory an arbiter may `Load` from.
    pub actions: Option<PathBuf>,
    /// Replay a decision log instead of consulting an arbiter.
    pub replay: Option<PathBuf>,
    /// Record every applied ruling here.
    pub decisions: Option<PathBuf>,
}

/// A build capability an external backend or arbiter needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Dylib,
    Rpc,
}

impl std::fmt::Display for Feature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Dylib => "dylib",
            Self::Rpc => "rpc",
        })
    }
}

impl Feature {
    fn action(self) -> &'static str {
        match self {
            Self::Dylib => "load a dynamic library",
            Self::Rpc => "connect to a remote body",
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("no image given")]
    NoImage,
    #[error("{path}: {source}")]
    Image {
        path: PathBuf,
        source: LoadFileError,
    },
    #[error("{path}: {source}")]
    Load { path: PathBuf, source: LoadError },
    #[error("the backend failed: {0}")]
    Backend(#[from] BackendError),
    #[error("the image faulted")]
    Faulted(Vec<Diagnostic>),
    #[error(
        "this `cyinevm` was built without the `{feature}` feature, so it cannot {action}\n\
         help: rebuild with `cargo build -p cyberspine-runtime --features {feature}`"
    )]
    MissingFeature {
        feature: Feature,
        action: &'static str,
    },
    #[error("the trace could not be written: {0}")]
    Trace(#[from] io::Error),
    #[error("the interrupt handler could not be installed: {0}")]
    Signal(#[from] ctrlc::Error),
    #[error("the arbiter could not be installed: {0}")]
    Arbiter(#[from] ArbiterError),
    #[error("the action directory is unusable: {0}")]
    Actions(#[from] ActionError),
    #[error(transparent)]
    Decisions(#[from] LogError),
}

// Exit classes: the input was rejected (3), the run ended abnormally (4), or the
// environment failed (5). `docs/diagnostics.md` §6.
const EXIT_REJECTED: u8 = 3;
const EXIT_FAULTED: u8 = 4;
const EXIT_ENVIRONMENT: u8 = 5;

// Codes for the failures the runtime itself raises. A load failure is the
// loader's code, not one of these: `Error::Load` delegates in `diagnostics`.
const RT_DUMMY_BODY: Code = Code::new("rt.dummy-body");
const RT_NO_IMAGE: Code = Code::new("rt.no-image");
const RT_MISSING_FEATURE: Code = Code::new("rt.missing-feature");
const RT_TRACE: Code = Code::new("rt.trace-write");
const RT_SIGNAL: Code = Code::new("rt.signal");
const RT_ACTIONS: Code = Code::new("rt.actions");
const RT_DECISIONS: Code = Code::new("rt.decisions");

impl Error {
    /// How the CLI should exit for this failure.
    pub fn class(&self) -> u8 {
        match self {
            Self::NoImage | Self::Load { .. } => EXIT_REJECTED,
            Self::Image { source, .. } => match source {
                LoadFileError::Io { .. } => EXIT_ENVIRONMENT,
                LoadFileError::FileTooLarge { .. } | LoadFileError::Image(_) => EXIT_REJECTED,
            },
            Self::Faulted(_) => EXIT_FAULTED,
            _ => EXIT_ENVIRONMENT,
        }
    }

    /// The failure as diagnostics. Rendering is the CLI's job; this only names
    /// what happened and where, and every arm is a `match` the compiler checks.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        match self {
            Self::Faulted(items) => items.clone(),
            Self::NoImage => vec![Diagnostic::error(RT_NO_IMAGE, "no image given")],
            Self::Image { path, source } => {
                vec![source.diagnostic(&path.display().to_string())]
            }
            Self::Load { path, source } => vec![
                Diagnostic::of(source).at(Anchor::whole(path.display().to_string())),
            ],
            Self::Backend(error) => vec![Diagnostic::from(error.cause().clone())],
            Self::MissingFeature { feature, action } => vec![
                Diagnostic::error(
                    RT_MISSING_FEATURE,
                    format!("this `cyinevm` was built without the `{feature}` feature"),
                )
                .with_help(format!(
                    "rebuild with `cargo build -p cyberspine-runtime --features {feature}` to {action}"
                )),
            ],
            Self::Trace(error) => vec![
                Diagnostic::error(RT_TRACE, error.to_string())
                    .note("the trace could not be written"),
            ],
            Self::Signal(error) => vec![Diagnostic::error(
                RT_SIGNAL,
                format!("the interrupt handler could not be installed: {error}"),
            )],
            Self::Arbiter(error) => vec![Diagnostic::from(error.cause().clone())],
            Self::Actions(error) => vec![Diagnostic::error(RT_ACTIONS, error.to_string())],
            Self::Decisions(error) => vec![Diagnostic::error(RT_DECISIONS, error.to_string())],
        }
    }
}

/// Run the body linked into this binary.
pub fn run_linked<L: LinkedBody>(
    paths: &[PathBuf],
    options: &Options,
    sink: &mut Diagnostics,
    on_event: &mut dyn FnMut(&Report),
) -> Result<(RunReport, Option<Error>), Error> {
    if let Notice::Warn(text) = L::NOTICE {
        sink.push(Diagnostic::warning(RT_DUMMY_BODY, text));
    }

    let (first, rest) = paths.split_first().ok_or(Error::NoImage)?;
    let image = read_image(first).map_err(|source| Error::Image {
        path: first.clone(),
        source,
    })?;
    let body = L::open(&image)?;
    let host = L::host(body);
    let mut context = Registry::new(&image);
    let layer = image.header().layer;
    let hash = image.content_hash();
    let debug = image.debug().cloned();
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(
        id,
        layer,
        hash,
        first.display().to_string().into_boxed_str(),
        debug,
    );
    load_rest(&mut session, &mut context, rest)?;

    Ok(run_session(session, context, options, sink, on_event))
}

/// Run a body named by an external spec.
pub fn run_external(
    spec: &BackendSpec,
    paths: &[PathBuf],
    options: &Options,
    sink: &mut Diagnostics,
    on_event: &mut dyn FnMut(&Report),
) -> Result<(RunReport, Option<Error>), Error> {
    let body = open_external(spec)?;
    let host = Host::new(body);

    let (first, rest) = paths.split_first().ok_or(Error::NoImage)?;
    let image = read_image(first).map_err(|source| Error::Image {
        path: first.clone(),
        source,
    })?;
    let mut context = Registry::new(&image);
    let layer = image.header().layer;
    let hash = image.content_hash();
    let debug = image.debug().cloned();
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(
        id,
        layer,
        hash,
        first.display().to_string().into_boxed_str(),
        debug,
    );
    load_rest(&mut session, &mut context, rest)?;

    Ok(run_session(session, context, options, sink, on_event))
}

/// Load every image after the first one into the session.
///
/// The first image fixed the clock and the shape; `Session::load` is what turns
/// "another action" into "another instance of this rig" or into a named
/// mismatch (`docs/isa.md` §8).
fn load_rest<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &mut Registry,
    paths: &[PathBuf],
) -> Result<(), Error> {
    for path in paths {
        let image = read_image(path).map_err(|source| Error::Image {
            path: path.clone(),
            source,
        })?;
        let layer = image.header().layer;
        let hash = image.content_hash();
        let debug = image.debug().cloned();
        let id = session.load(image).map_err(|source| Error::Load {
            path: path.clone(),
            source,
        })?;
        context.push(
            id,
            layer,
            hash,
            path.display().to_string().into_boxed_str(),
            debug,
        );
    }
    Ok(())
}

fn open_external(spec: &BackendSpec) -> Result<Box<dyn Backend>, Error> {
    match spec {
        BackendSpec::Dylib(_) => require(Feature::Dylib)?,
        BackendSpec::Rpc(_) => require(Feature::Rpc)?,
    }
    Ok(spec.open()?)
}

fn require(feature: Feature) -> Result<(), Error> {
    let enabled = match feature {
        Feature::Dylib => cfg!(feature = "dylib"),
        Feature::Rpc => cfg!(feature = "rpc"),
    };
    if enabled {
        Ok(())
    } else {
        Err(Error::MissingFeature {
            feature,
            action: feature.action(),
        })
    }
}

/// Drive the session, then describe what happened.
///
/// The report is produced even when the run ends in an error: "it faulted at
/// tick 7 of 120" is the answer the caller needs, and it only exists after the
/// loop has stopped.
fn run_session<B: Backend, P: RigPolicy>(
    mut session: Session<B, P>,
    mut context: Registry,
    options: &Options,
    sink: &mut Diagnostics,
    on_event: &mut dyn FnMut(&Report),
) -> (RunReport, Option<Error>) {
    let result = run_loop(&mut session, &mut context, options, sink, on_event);

    let outcome = match &result {
        Ok(()) => match options.mode {
            RunMode::RealTime(Stop::Ticks(_)) | RunMode::UnpacedTicks(_) => Outcome::Budget,
            RunMode::RealTime(Stop::UntilHalt) => Outcome::Halted,
            RunMode::RealTime(Stop::Never) => Outcome::Stopped,
        },
        Err(Error::Faulted(_)) => Outcome::Faulted,
        Err(_) => Outcome::Environment,
    };

    let report = RunReport {
        outcome,
        ticks: session.now(),
        dt: session.dt(),
        overruns: session.overruns(),
        instances: context
            .loaded()
            .iter()
            .map(|loaded| InstanceReport {
                id: loaded.id.get(),
                layer: loaded.layer,
                action: loaded.action.to_string(),
            })
            .collect(),
    };

    (report, result.err())
}

fn run_loop<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &mut Registry,
    options: &Options,
    sink: &mut Diagnostics,
    on_event: &mut dyn FnMut(&Report),
) -> Result<(), Error> {
    if !options.quiet {
        let shape = session.shape();
        eprintln!(
            "body: {} muscle(s), {} float sensor(s), {} integer sensor(s)",
            shape.muscles, shape.sensors_f, shape.sensors_i
        );
        eprintln!(
            "rig: {} at {} Hz, {} instance(s)",
            context.rig(),
            session.hz(),
            context.loaded().len()
        );
        for loaded in context.loaded() {
            eprintln!(
                "  instance {}: layer {}, {}",
                loaded.id, loaded.layer, loaded.action
            );
        }
    }

    let mut trace = options.trace.then(|| Trace::new(context.names()));
    if let Some(trace) = trace.as_mut() {
        trace.header()?;
        session.set_trace_capacity(64);
        session.set_tracing(true);
    }
    if trace.as_ref().is_some_and(Trace::is_broken) {
        return Ok(());
    }
    if matches!(
        options.mode,
        RunMode::RealTime(Stop::Ticks(0)) | RunMode::UnpacedTicks(0)
    ) {
        return Ok(());
    }

    let Installed {
        arbiter,
        mut notices,
    } = open_arbiter(options)?;
    let mut interrupts = Interrupts::new(
        arbiter,
        open_actions(options)?,
        open_decisions(options)?,
        fallback(options.on_interrupt),
    );

    let result = drive(
        session,
        context,
        options,
        &mut interrupts,
        &mut trace,
        on_event,
    );

    // A replay that was cut short did not diverge — it just did not finish.
    // Ctrl-C (`Stop::Never` is only exit-able by Ctrl-C) and a failed run are
    // both situations where "rounds were never applied" is expected noise.
    let completed = !matches!(options.mode, RunMode::RealTime(Stop::Never));
    if result.is_ok()
        && completed
        && let Some(source) = notices.as_mut()
    {
        for notice in source.take_notices() {
            sink.push(into_diagnostic(notice));
        }
    }

    result
}

fn drive<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &mut Registry,
    options: &Options,
    interrupts: &mut Interrupts<Box<dyn Arbiter>>,
    trace: &mut Option<Trace>,
    on_event: &mut dyn FnMut(&Report),
) -> Result<(), Error> {
    let stop = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stop = Arc::clone(&stop);
        move || stop.store(true, Ordering::SeqCst)
    })?;

    match options.mode {
        RunMode::UnpacedTicks(ticks) => {
            for _ in 0..ticks {
                if interrupted(&stop) {
                    break;
                }
                session.tick()?;
                if !observe(session, context, trace.as_mut(), interrupts, on_event)? {
                    return Ok(());
                }
            }
        }
        RunMode::RealTime(Stop::Ticks(ticks)) => {
            let frame = Duration::from_secs_f32(session.dt());
            let mut deadline = Instant::now();
            for _ in 0..ticks {
                if interrupted(&stop) {
                    break;
                }
                deadline += frame;
                sleep_until(deadline);
                session.tick()?;
                if !observe(session, context, trace.as_mut(), interrupts, on_event)? {
                    return Ok(());
                }
            }
        }
        RunMode::RealTime(mode) => {
            let frame = Duration::from_secs_f32(session.dt());
            let mut last = Instant::now();
            loop {
                if interrupted(&stop) {
                    break;
                }
                thread::sleep(frame);
                let elapsed = last.elapsed().as_secs_f64();
                last = Instant::now();
                session.advance(elapsed)?;
                if !observe(session, context, trace.as_mut(), interrupts, on_event)? {
                    return Ok(());
                }
                if mode == Stop::UntilHalt && session.is_idle() {
                    break;
                }
            }
        }
    }

    Ok(())
}

fn interrupted(stop: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst)
}

/// The arbiter this run will consult, and the optional handle that reports what
/// it noticed once the run is over.
///
/// `notices` is `None` for an adjudicator with nothing to say; splitting it out
/// of [`Arbiter`] keeps the adjudication contract to the two methods every
/// arbiter must have.
pub struct Installed {
    pub arbiter: Box<dyn Arbiter>,
    pub notices: Option<Box<dyn NoticeSource>>,
}

/// The arbiter this run will consult: a replay, a named one, or the built-in
/// single round.
fn open_arbiter(options: &Options) -> Result<Installed, Error> {
    if let Some(path) = &options.replay {
        let replay = Replay::load(path)?;
        let notices: Box<dyn NoticeSource> = Box::new(replay.report());
        return Ok(Installed {
            arbiter: Box::new(replay),
            notices: Some(notices),
        });
    }
    match &options.interrupt {
        Some(spec) => Ok(Installed {
            arbiter: spec.open()?,
            notices: None,
        }),
        None => Ok(Installed {
            arbiter: Box::new(BuiltinArbiter::new(options.on_interrupt)),
            notices: None,
        }),
    }
}

fn open_actions(options: &Options) -> Result<Option<Box<dyn ActionResolver>>, Error> {
    match &options.actions {
        Some(root) => Ok(Some(Box::new(ActionDirectory::open(root)?))),
        None => Ok(None),
    }
}

fn open_decisions(options: &Options) -> Result<Option<Log>, Error> {
    match &options.decisions {
        Some(path) => Ok(Some(Log::create(path)?)),
        None => Ok(None),
    }
}

/// What the host answers when the arbiter itself cannot.
fn fallback(policy: OnInterrupt) -> ArbiterVerdict {
    match policy {
        OnInterrupt::Stop => ArbiterVerdict::Stop,
        OnInterrupt::Continue => ArbiterVerdict::Continue,
    }
}

fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        thread::sleep(deadline - now);
    }
}

fn observe<B: Backend, P: RigPolicy, A: Arbiter>(
    session: &mut Session<B, P>,
    context: &mut Registry,
    trace: Option<&mut Trace>,
    interrupts: &mut Interrupts<A>,
    on_event: &mut dyn FnMut(&Report),
) -> Result<bool, Error> {
    report_faults(session, context)?;
    for report in interrupts.pump(session, context)? {
        if !report.is_silent() {
            on_event(&report);
        }
    }

    if let Some(trace) = trace {
        trace.drain(session)?;
        if trace.is_broken() {
            return Ok(false);
        }
    }

    Ok(true)
}

/// The built-in single-round arbiter: every interrupt is finished on the spot.
///
/// It is the degenerate case of the stream, not a different mechanism: it never
/// emits `Unfinished`, so the CLI needs no chain. A real host replaces it with a
/// linked, dylib or remote arbiter (`docs/interrupt.md` §7).
struct BuiltinArbiter {
    verdict: ArbiterVerdict,
    queue: Vec<Decision>,
}

impl BuiltinArbiter {
    fn new(policy: OnInterrupt) -> Self {
        let verdict = match policy {
            OnInterrupt::Stop => ArbiterVerdict::Stop,
            OnInterrupt::Continue => ArbiterVerdict::Continue,
        };
        Self {
            verdict,
            queue: Vec::new(),
        }
    }
}

impl Arbiter for BuiltinArbiter {
    fn deliver(&mut self, event: InterruptEvent) {
        self.queue.push(Decision {
            event: event.event,
            ruling: ArbiterRuling::finished(Vec::new(), self.verdict),
        });
    }

    fn take(&mut self, _now: u64) -> Batch {
        Batch {
            decisions: core::mem::take(&mut self.queue),
            failure: None,
        }
    }
}

fn report_faults<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &Registry,
) -> Result<(), Error> {
    let faults = session.take_faults();
    if faults.is_empty() {
        return Ok(());
    }

    let mut diagnostics = Vec::with_capacity(faults.len());
    for record in faults {
        let pc = record.fault.pc();
        let image = context
            .action_of(record.instance)
            .unwrap_or("<unknown image>")
            .to_string();
        let mut diagnostic = Diagnostic::of(&record.fault).at(Anchor::new(
            image,
            Site::Instruction {
                pc: u32::try_from(pc).unwrap_or(u32::MAX),
            },
        ));
        // §8 of asm.md: a debug build carries the source position; a release
        // build says only the pc, and that is the whole difference.
        if let Some(source) = context.source_of(record.instance, pc) {
            diagnostic = diagnostic.label(source, "the instruction came from here");
        }
        diagnostics.push(diagnostic);
    }

    Err(Error::Faulted(diagnostics))
}
