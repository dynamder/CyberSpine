use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cyberspine_arbiter::{
    Arbiter, ArbiterError, ArbiterSpec, Batch, Decision, InterruptEvent, Ruling as ArbiterRuling,
    Verdict as ArbiterVerdict,
};
use cyberspine_backend::BackendSpec;
use cyberspine_host::{
    ActionDirectory, ActionError, ActionResolver, Interrupts, Log, LogError, Registry, Replay,
};
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
    #[error("the image faulted:\n{0}")]
    Faulted(String),
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

/// Run the body linked into this binary.
pub fn run_linked<L: LinkedBody>(paths: &[PathBuf], options: &Options) -> Result<(), Error> {
    if let Notice::Warn(text) = L::NOTICE {
        eprintln!("warning: {text}");
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
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(
        id,
        layer,
        hash,
        first.display().to_string().into_boxed_str(),
    );
    load_rest(&mut session, &mut context, rest)?;

    run_session(session, context, options)
}

/// Run a body named by an external spec.
pub fn run_external(spec: &BackendSpec, paths: &[PathBuf], options: &Options) -> Result<(), Error> {
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
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(
        id,
        layer,
        hash,
        first.display().to_string().into_boxed_str(),
    );
    load_rest(&mut session, &mut context, rest)?;

    run_session(session, context, options)
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
        let id = session.load(image).map_err(|source| Error::Load {
            path: path.clone(),
            source,
        })?;
        context.push(id, layer, hash, path.display().to_string().into_boxed_str());
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

fn run_session<B: Backend, P: RigPolicy>(
    mut session: Session<B, P>,
    mut context: Registry,
    options: &Options,
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

    let mut interrupts = Interrupts::new(
        open_arbiter(options)?,
        open_actions(options)?,
        open_decisions(options)?,
        fallback(options.on_interrupt),
    );

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
                if !observe(&mut session, &mut context, trace.as_mut(), &mut interrupts)? {
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
                if !observe(&mut session, &mut context, trace.as_mut(), &mut interrupts)? {
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
                if !observe(&mut session, &mut context, trace.as_mut(), &mut interrupts)? {
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

/// The arbiter this run will consult: a replay, a named one, or the built-in
/// single round.
fn open_arbiter(options: &Options) -> Result<Box<dyn Arbiter>, Error> {
    if let Some(path) = &options.replay {
        return Ok(Box::new(Replay::load(path)?));
    }
    match &options.interrupt {
        Some(spec) => Ok(spec.open()?),
        None => Ok(Box::new(BuiltinArbiter::new(options.on_interrupt))),
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
) -> Result<bool, Error> {
    report_faults(session)?;
    for report in interrupts.pump(session, context)? {
        if !report.is_silent() {
            eprintln!("{report}");
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

fn report_faults<B: Backend, P: RigPolicy>(session: &mut Session<B, P>) -> Result<(), Error> {
    let faults = session.take_faults();
    if faults.is_empty() {
        return Ok(());
    }

    let mut message = String::new();
    for record in faults {
        message.push_str(&format!(
            "  instance {}: {}\n",
            record.instance, record.fault
        ));
    }
    Err(Error::Faulted(message))
}
