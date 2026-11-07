use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cyberspine_arbiter::{
    Arbiter, Batch, Decision, EventId, InterruptEvent, Ruling as ArbiterRuling,
    Verdict as ArbiterVerdict,
};
use cyberspine_backend::BackendSpec;
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::Fingerprint;
use cyberspine_vm::io::{LoadFileError, read_image};
use cyberspine_vm::{
    Backend, BackendError, Host, InstanceId, Interrupt, LoadError, Resolution, RigPolicy, Ruling,
    Session, Verdict,
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
    let hash = image.content_hash();
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(id, layer, hash, first);
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
    let hash = image.content_hash();
    let (mut session, id) = host.load(image).map_err(|(_, source)| Error::Load {
        path: first.clone(),
        source,
    })?;
    context.push(id, layer, hash, first);
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
        let hash = image.content_hash();
        let id = session.load(image).map_err(|source| Error::Load {
            path: path.clone(),
            source,
        })?;
        context.push(id, layer, hash, path);
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
/// operator wants the path; this is the map between them. The hash is what an
/// arbiter sees as the action's identity.
struct Loaded {
    id: InstanceId,
    layer: u16,
    hash: [u8; 32],
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

    fn push(&mut self, id: InstanceId, layer: u16, hash: [u8; 32], path: &Path) {
        self.instances.push(Loaded {
            id,
            layer,
            hash,
            path: path.to_path_buf(),
        });
    }

    fn path_of(&self, id: InstanceId) -> Option<&Path> {
        self.instances
            .iter()
            .find(|loaded| loaded.id == id)
            .map(|loaded| loaded.path.as_path())
    }

    fn loaded(&self, id: InstanceId) -> Option<&Loaded> {
        self.instances.iter().find(|loaded| loaded.id == id)
    }

    /// The interrupted instance's action and the actions visible beside it.
    ///
    /// The CLI has no action resolver yet, so a path is the action reference it
    /// can offer. A host with a registry fills the same fields with registry ids.
    fn snapshot(&self, id: InstanceId) -> (Box<str>, Vec<Box<str>>, [u8; 32]) {
        let action = self
            .path_of(id)
            .map(|path| path.display().to_string().into_boxed_str())
            .unwrap_or_default();
        let actions = self
            .instances
            .iter()
            .map(|loaded| loaded.path.display().to_string().into_boxed_str())
            .collect();
        let hash = self.loaded(id).map(|loaded| loaded.hash).unwrap_or([0; 32]);
        (action, actions, hash)
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

    let mut interrupts = Interrupts::new(BuiltinArbiter::new(options.on_interrupt));

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
                if !observe(&mut session, &context, trace.as_mut(), &mut interrupts)? {
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
                if !observe(&mut session, &context, trace.as_mut(), &mut interrupts)? {
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
                if !observe(&mut session, &context, trace.as_mut(), &mut interrupts)? {
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

fn observe<B: Backend, P: RigPolicy, A: Arbiter>(
    session: &mut Session<B, P>,
    context: &RunContext,
    trace: Option<&mut Trace>,
    interrupts: &mut Interrupts<A>,
) -> Result<bool, Error> {
    report_faults(session)?;
    interrupts.pump(session, context);

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
            ruling: ArbiterRuling::Finished(self.verdict.clone()),
        });
    }

    fn take(&mut self) -> Batch {
        Batch {
            decisions: core::mem::take(&mut self.queue),
            failure: None,
        }
    }
}

/// One round an arbiter already answered.
struct Round {
    source: Box<str>,
    note: Box<str>,
}

/// A ticket the arbiter has not finished with.
struct InFlight {
    ticket: Interrupt,
    trail: Vec<Round>,
}

/// Applies one arbiter's ruling stream to the session.
///
/// The host knows only "this event is not finished yet". Rounds are kept for the
/// log and for replay; they never touch the instance, so an unfinished stream
/// leaves the guest parked exactly like a missing verdict did before.
struct Interrupts<A: Arbiter> {
    arbiter: A,
    inflight: HashMap<EventId, InFlight>,
}

impl<A: Arbiter> Interrupts<A> {
    fn new(arbiter: A) -> Self {
        Self {
            arbiter,
            inflight: HashMap::new(),
        }
    }

    fn pump<B: Backend, P: RigPolicy>(
        &mut self,
        session: &mut Session<B, P>,
        context: &RunContext,
    ) {
        for ticket in session.take_interrupts() {
            let event = EventId::new(ticket.seq());
            let (action, actions, image_hash) = context.snapshot(ticket.instance);
            report_interrupt(&ticket, context);
            self.arbiter.deliver(InterruptEvent {
                event,
                pc: ticket.pc,
                tick: ticket.tick,
                format: ticket.format.clone(),
                reason: ticket.reason.clone(),
                action,
                actions,
                image_hash,
            });
            self.inflight.insert(
                event,
                InFlight {
                    ticket,
                    trail: Vec::new(),
                },
            );
        }

        let Batch { decisions, failure } = self.arbiter.take();
        for Decision { event, ruling } in decisions {
            let Some(mut inflight) = self.inflight.remove(&event) else {
                eprintln!("warning: the arbiter answered unknown event {event}");
                continue;
            };
            match ruling {
                ArbiterRuling::Unfinished { source, note } => {
                    eprintln!("interrupt {event}: {source}: {note}");
                    inflight.trail.push(Round { source, note });
                    self.repark(session, event, inflight);
                }
                ArbiterRuling::Finished(verdict) => {
                    report_trail(event, &inflight.trail);
                    finish(session, event, inflight.ticket, verdict);
                }
            }
        }

        if let Some(error) = failure {
            eprintln!("warning: the arbiter failed: {error}");
            for (_, inflight) in self.inflight.drain() {
                settle(session, inflight.ticket, Verdict::Stop);
            }
        }
    }

    fn repark<B: Backend, P: RigPolicy>(
        &mut self,
        session: &mut Session<B, P>,
        event: EventId,
        mut inflight: InFlight,
    ) {
        match session.resolve(inflight.ticket, Ruling::Unfinished) {
            Ok(Resolution::Parked(ticket)) => {
                inflight.ticket = ticket;
                self.inflight.insert(event, inflight);
            }
            Ok(Resolution::Settled) => {}
            Err(error) => {
                eprintln!("warning: event {event} could not be carried: {error}");
            }
        }
    }
}

fn finish<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    event: EventId,
    ticket: Interrupt,
    verdict: ArbiterVerdict,
) {
    match verdict {
        ArbiterVerdict::Continue => settle(session, ticket, Verdict::Continue),
        ArbiterVerdict::Stop => settle(session, ticket, Verdict::Stop),
        ArbiterVerdict::Switch { action } => {
            eprintln!(
                "warning: event {event}: switching to `{action}` needs an action resolver \
                 this CLI does not have; stopping instead"
            );
            settle(session, ticket, Verdict::Stop);
        }
    }
}

fn settle<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    ticket: Interrupt,
    verdict: Verdict,
) {
    if let Err(error) = session.resolve(ticket, Ruling::Finished(verdict)) {
        eprintln!("warning: the interrupt verdict did not apply: {error}");
    }
}

fn report_trail(event: EventId, trail: &[Round]) {
    let Some(last) = trail.last() else {
        return;
    };
    eprintln!(
        "interrupt {event}: finished after {} round(s), last: {}: {}",
        trail.len(),
        last.source,
        last.note
    );
}

fn report_interrupt(ticket: &Interrupt, context: &RunContext) {
    let path = context
        .path_of(ticket.instance)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "?".to_string());
    eprintln!(
        "interrupt: instance {} ({path}) at pc {} tick {}: {}",
        ticket.instance, ticket.pc, ticket.tick, ticket.reason
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use cyberspine_testkit::{action, halt, intr, rig, verified};
    use cyberspine_vm::{DummyBackend, Strict};

    struct Scripted {
        event: Option<EventId>,
        rounds: Vec<ArbiterRuling>,
    }

    impl Scripted {
        fn new(rounds: Vec<ArbiterRuling>) -> Self {
            Self {
                event: None,
                rounds,
            }
        }
    }

    impl Arbiter for Scripted {
        fn deliver(&mut self, event: InterruptEvent) {
            self.event = Some(event.event);
        }

        fn take(&mut self) -> Batch {
            let Some(event) = self.event else {
                return Batch::default();
            };
            let decisions: Vec<Decision> = self
                .rounds
                .drain(..)
                .map(|ruling| Decision { event, ruling })
                .collect();
            Batch::from(decisions)
        }
    }

    fn unfinished(note: &str) -> ArbiterRuling {
        ArbiterRuling::Unfinished {
            source: "fallback".into(),
            note: note.into(),
        }
    }

    fn parked_session() -> (Session<DummyBackend, Strict>, InstanceId, RunContext) {
        let rig = rig(1);
        let mut image = action(vec![intr(0), halt()], &rig);
        image.push_reason("obstacle");
        let image = verified(image);
        let context = RunContext::new(&image);
        let (mut session, id) = Host::new(DummyBackend::new(rig))
            .load(image)
            .expect("assembled for this rig");
        session.tick().expect("the dummy backend cannot fail");
        (session, id, context)
    }

    #[test]
    fn a_round_and_a_verdict_in_one_batch_are_applied_in_order() {
        let (mut session, id, context) = parked_session();
        let mut interrupts = Interrupts::new(Scripted::new(vec![
            unfinished("no rule"),
            ArbiterRuling::Finished(ArbiterVerdict::Continue),
        ]));

        interrupts.pump(&mut session, &context);

        assert!(session.parked().is_empty());
        assert_eq!(session.instance(id).map(|instance| instance.pc()), Some(1));
        session.tick().expect("the dummy backend cannot fail");
        assert!(
            session.instance(id).is_none(),
            "the HALT after INTR ended the instance"
        );
    }

    #[test]
    fn an_unfinished_round_keeps_the_ticket_until_the_next_one() {
        let (mut session, id, context) = parked_session();
        let mut interrupts = Interrupts::new(Scripted::new(vec![unfinished("no rule")]));

        interrupts.pump(&mut session, &context);
        assert_eq!(session.parked(), vec![id], "the first round did not decide");

        interrupts
            .arbiter
            .rounds
            .push(ArbiterRuling::Finished(ArbiterVerdict::Stop));
        interrupts.pump(&mut session, &context);

        assert!(session.instance(id).is_none());
        assert!(session.is_idle());
    }
}
