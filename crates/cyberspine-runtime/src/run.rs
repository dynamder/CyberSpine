use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cyberspine_backend::BackendSpec;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Fingerprint;
use cyberspine_vm::io::{LoadFileError, read_image};
use cyberspine_vm::{
    Backend, BackendError, Host, InstanceId, LoadError, RigPolicy, Session, Verdict,
};
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

/// What the built-in arbiter does with an interrupt.
///
/// A real host replaces this with a plugin (`docs/interrupt.md`); the CLI has no
/// way to ask an external program, so it offers the two decisions that need no
/// state: stop the instance, or let it continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnInterrupt {
    Stop,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub mode: RunMode,
    pub trace: bool,
    pub quiet: bool,
    pub on_interrupt: OnInterrupt,
}

/// A build capability an external backend needs.
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
    let mut context = RunContext::new(&image);
    let layer = image.header().layer;
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(id, layer, first);
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
    let mut context = RunContext::new(&image);
    let layer = image.header().layer;
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(id, layer, first);
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
    context: &mut RunContext,
    paths: &[PathBuf],
) -> Result<(), Error> {
    for path in paths {
        let image = read_image(path).map_err(|source| Error::Image {
            path: path.clone(),
            source,
        })?;
        let layer = image.header().layer;
        let id = session.load(image).map_err(|source| Error::Load {
            path: path.clone(),
            source,
        })?;
        context.push(id, layer, path);
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

/// One instance and the file it came from. The session knows the id, the
/// operator wants the path; this is the map between them.
struct Loaded {
    id: InstanceId,
    layer: u16,
    path: PathBuf,
}

struct RunContext {
    names: Vec<String>,
    rig: Fingerprint,
    instances: Vec<Loaded>,
}

impl RunContext {
    fn new(image: &VerifiedImage) -> Self {
        Self {
            names: image
                .muscles()
                .iter()
                .map(|muscle| image.name(muscle.name).to_owned())
                .collect(),
            rig: image.header().rig,
            instances: Vec::new(),
        }
    }

    fn push(&mut self, id: InstanceId, layer: u16, path: &Path) {
        self.instances.push(Loaded {
            id,
            layer,
            path: path.to_path_buf(),
        });
    }

    fn path_of(&self, id: InstanceId) -> Option<&Path> {
        self.instances
            .iter()
            .find(|loaded| loaded.id == id)
            .map(|loaded| loaded.path.as_path())
    }
}

fn run_session<B: Backend, P: RigPolicy>(
    mut session: Session<B, P>,
    context: RunContext,
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
            context.rig,
            session.hz(),
            context.instances.len()
        );
        for loaded in &context.instances {
            eprintln!(
                "  instance {}: layer {}, {}",
                loaded.id,
                loaded.layer,
                loaded.path.display()
            );
        }
    }

    let mut trace = options.trace.then(|| Trace::new(&context.names));
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
                if !observe(&mut session, &context, options, trace.as_mut())? {
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
                if !observe(&mut session, &context, options, trace.as_mut())? {
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
                if !observe(&mut session, &context, options, trace.as_mut())? {
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

fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        thread::sleep(deadline - now);
    }
}

fn observe<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &RunContext,
    options: &Options,
    trace: Option<&mut Trace>,
) -> Result<bool, Error> {
    report_faults(session)?;
    report_interrupts(session, context, options);

    if let Some(trace) = trace {
        trace.drain(session)?;
        if trace.is_broken() {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Answer every interrupt this tick produced with the built-in policy.
///
/// This is the whole "host decides" story for the CLI: a real host replaces the
/// two-line body with its own arbiter (`docs/interrupt.md`).
fn report_interrupts<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &RunContext,
    options: &Options,
) {
    for interrupt in session.take_interrupts() {
        let path = context
            .path_of(interrupt.instance)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "?".to_string());
        eprintln!(
            "interrupt: instance {} ({path}) at pc {} tick {}: {}",
            interrupt.instance, interrupt.pc, interrupt.tick, interrupt.reason
        );
        let verdict = match options.on_interrupt {
            OnInterrupt::Stop => Verdict::Stop,
            OnInterrupt::Continue => Verdict::Continue,
        };
        if let Err(error) = session.resolve(interrupt, verdict) {
            eprintln!("warning: the built-in interrupt policy did not apply: {error}");
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
