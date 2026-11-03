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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub mode: RunMode,
    pub trace: bool,
    pub quiet: bool,
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
pub fn run_linked<L: LinkedBody>(path: &Path, options: &Options) -> Result<(), Error> {
    if let Notice::Warn(text) = L::NOTICE {
        eprintln!("warning: {text}");
    }

    let image = read_image(path).map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    let context = RunContext::new(&image);
    let body = L::open(&image)?;
    let host = L::host(body);
    let (session, _) = host.load(image).map_err(|(_, source)| Error::Load {
        path: path.to_path_buf(),
        source,
    })?;

    run_session(session, context, options)
}

/// Run a body named by an external spec.
pub fn run_external(spec: &BackendSpec, path: &Path, options: &Options) -> Result<(), Error> {
    let body = open_external(spec)?;
    let image = read_image(path).map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    let context = RunContext::new(&image);
    let host = Host::new(body);
    let (session, _) = host.load(image).map_err(|(_, source)| Error::Load {
        path: path.to_path_buf(),
        source,
    })?;

    run_session(session, context, options)
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

struct RunContext {
    names: Vec<String>,
    rig: Fingerprint,
    layer: u16,
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
            layer: image.header().layer,
        }
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
            "image: {} at {} Hz, layer {}, {} instance(s)",
            context.rig,
            session.hz(),
            context.layer,
            session.instances()
        );
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
                if !observe(&mut session, trace.as_mut())? {
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
                if !observe(&mut session, trace.as_mut())? {
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
                if !observe(&mut session, trace.as_mut())? {
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
    trace: Option<&mut Trace>,
) -> Result<bool, Error> {
    report_faults(session)?;

    if let Some(trace) = trace {
        trace.drain(session)?;
        if trace.is_broken() {
            return Ok(false);
        }
    }

    Ok(true)
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
