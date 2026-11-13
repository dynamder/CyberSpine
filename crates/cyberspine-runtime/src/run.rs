use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cyberspine_arbiter::{
    Action, Arbiter, ArbiterError, ArbiterSpec, Batch, Completion, Decision, EventId, InstanceRef,
    InstanceView, InterruptEvent, Ruling as ArbiterRuling, Verdict as ArbiterVerdict,
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

use crate::actions::{ActionDirectory, ActionError, ActionResolver};
use crate::decisions::{Log, LogError, Record, Replay};
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
    let mut context = RunContext::new(&image);
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
    let mut context = RunContext::new(&image);
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

/// One instance and the action it came from. The session knows the id; the
/// arbiter names instances by reference and actions by an opaque name. The hash
/// is what an arbiter sees as the action's identity.
struct Loaded {
    id: InstanceId,
    layer: u16,
    hash: [u8; 32],
    action: Box<str>,
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

    fn push(&mut self, id: InstanceId, layer: u16, hash: [u8; 32], action: Box<str>) {
        self.instances.push(Loaded {
            id,
            layer,
            hash,
            action,
        });
    }

    fn remove(&mut self, id: InstanceId) {
        self.instances.retain(|loaded| loaded.id != id);
    }

    fn action_of(&self, id: InstanceId) -> Option<&str> {
        self.instances
            .iter()
            .find(|loaded| loaded.id == id)
            .map(|loaded| &*loaded.action)
    }

    fn id_of(&self, reference: InstanceRef) -> Option<InstanceId> {
        self.instances
            .iter()
            .find(|loaded| loaded.id.get() == reference.get())
            .map(|loaded| loaded.id)
    }

    fn loaded(&self, id: InstanceId) -> Option<&Loaded> {
        self.instances.iter().find(|loaded| loaded.id == id)
    }

    /// The interrupted instance's action and the instances visible beside it.
    fn snapshot(&self, id: InstanceId) -> (Box<str>, Vec<InstanceView>, [u8; 32]) {
        let action = self.action_of(id).unwrap_or_default().into();
        let instances = self
            .instances
            .iter()
            .map(|loaded| InstanceView {
                reference: InstanceRef::new(loaded.id.get()),
                action: loaded.action.clone(),
                layer: loaded.layer,
            })
            .collect();
        let hash = self.loaded(id).map(|loaded| loaded.hash).unwrap_or([0; 32]);
        (action, instances, hash)
    }
}

fn run_session<B: Backend, P: RigPolicy>(
    mut session: Session<B, P>,
    mut context: RunContext,
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
                loaded.id, loaded.layer, loaded.action
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
    context: &mut RunContext,
    trace: Option<&mut Trace>,
    interrupts: &mut Interrupts<A>,
) -> Result<bool, Error> {
    report_faults(session)?;
    interrupts.pump(session, context)?;

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
    resolver: Option<Box<dyn ActionResolver>>,
    log: Option<Log>,
    /// What the host answers when the arbiter itself is gone.
    default: ArbiterVerdict,
}

impl<A: Arbiter> Interrupts<A> {
    fn new(
        arbiter: A,
        resolver: Option<Box<dyn ActionResolver>>,
        log: Option<Log>,
        default: ArbiterVerdict,
    ) -> Self {
        Self {
            arbiter,
            inflight: HashMap::new(),
            resolver,
            log,
            default,
        }
    }

    fn pump<B: Backend, P: RigPolicy>(
        &mut self,
        session: &mut Session<B, P>,
        context: &mut RunContext,
    ) -> Result<(), Error> {
        for ticket in session.take_interrupts() {
            let event = EventId::new(ticket.seq());
            let (action, instances, image_hash) = context.snapshot(ticket.instance);
            let actions = self
                .resolver
                .as_ref()
                .map(|resolver| resolver.actions())
                .unwrap_or_default();
            report_interrupt(&ticket, context);
            self.arbiter.deliver(InterruptEvent {
                event,
                pc: ticket.pc,
                tick: ticket.tick,
                format: ticket.format.clone(),
                reason: ticket.reason.clone(),
                action,
                instances,
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

        let now = session.now();
        let Batch { decisions, failure } = self.arbiter.take(now);
        for Decision { event, ruling } in decisions {
            let Some(mut inflight) = self.inflight.remove(&event) else {
                eprintln!("warning: the arbiter answered unknown event {event}");
                continue;
            };
            if let Some(log) = self.log.as_mut() {
                log.write(&Record::new(event, now, &ruling))?;
            }
            apply_actions(
                session,
                context,
                self.resolver.as_deref(),
                event,
                ruling.actions,
            );
            match ruling.completion {
                Completion::Unfinished => {
                    eprintln!("interrupt {event}: {}: {}", ruling.source, ruling.note);
                    inflight.trail.push(Round {
                        source: ruling.source,
                        note: ruling.note,
                    });
                    self.repark(session, event, inflight);
                }
                Completion::Finished(verdict) => {
                    report_trail(event, &inflight.trail);
                    let instance = inflight.ticket.instance;
                    let settled = settle(session, inflight.ticket, verdict);
                    forget(settled, verdict, context, instance);
                }
            }
        }

        if let Some(error) = failure {
            eprintln!("warning: the arbiter failed: {error}");
            for (_, inflight) in self.inflight.drain() {
                let instance = inflight.ticket.instance;
                let settled = settle(session, inflight.ticket, self.default);
                forget(settled, self.default, context, instance);
            }
        }

        Ok(())
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

/// Apply one round's actions, left to right, before its completion.
///
/// Best-effort on purpose: a failed action is reported and skipped, and the
/// completion is still applied. A `Load` that cannot be resolved leaves the
/// world without the new action, which is exactly the fail-closed outcome when
/// the completion is `Stop`.
fn apply_actions<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    context: &mut RunContext,
    resolver: Option<&dyn ActionResolver>,
    event: EventId,
    actions: Vec<Action>,
) {
    for action in actions {
        match action {
            Action::Load { action } => {
                let Some(resolver) = resolver else {
                    eprintln!(
                        "warning: event {event}: cannot load `{action}` \
                         because no action directory was given"
                    );
                    continue;
                };
                let image = match resolver.resolve(&action) {
                    Ok(image) => image,
                    Err(error) => {
                        eprintln!("warning: event {event}: `{action}` cannot load: {error}");
                        continue;
                    }
                };
                let layer = image.header().layer;
                let hash = image.content_hash();
                match session.load(image) {
                    Ok(id) => context.push(id, layer, hash, action),
                    Err(error) => {
                        eprintln!("warning: event {event}: `{action}` does not fit: {error}");
                    }
                }
            }
            Action::Unload { instance } => {
                let Some(id) = context.id_of(instance) else {
                    eprintln!("warning: event {event}: unknown instance {instance}");
                    continue;
                };
                if session.unload(id) {
                    context.remove(id);
                } else {
                    eprintln!("warning: event {event}: instance {instance} was already gone");
                }
            }
            Action::Silence { instance } => {
                let Some(id) = context.id_of(instance) else {
                    eprintln!("warning: event {event}: unknown instance {instance}");
                    continue;
                };
                session.set_silenced(id, true);
            }
            Action::Restore { instance } => {
                let Some(id) = context.id_of(instance) else {
                    eprintln!("warning: event {event}: unknown instance {instance}");
                    continue;
                };
                session.set_silenced(id, false);
            }
        }
    }
}

/// Apply a terminal verdict, returning whether it landed.
fn settle<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    ticket: Interrupt,
    verdict: ArbiterVerdict,
) -> bool {
    let verdict = match verdict {
        ArbiterVerdict::Continue => Verdict::Continue,
        ArbiterVerdict::Stop => Verdict::Stop,
    };
    match session.resolve(ticket, Ruling::Finished(verdict)) {
        Ok(_) => true,
        Err(error) => {
            eprintln!("warning: the interrupt verdict did not apply: {error}");
            false
        }
    }
}

/// Keep the host's bookkeeping in step with a verdict that removed an instance.
fn forget(settled: bool, verdict: ArbiterVerdict, context: &mut RunContext, id: InstanceId) {
    if settled && verdict == ArbiterVerdict::Stop {
        context.remove(id);
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
    let action = context.action_of(ticket.instance).unwrap_or("?");
    eprintln!(
        "interrupt: instance {} ({action}) at pc {} tick {}: {}",
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
    use cyberspine_testkit::{action, halt, intr, jump, rig, verified, yield_};
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

        fn take(&mut self, _now: u64) -> Batch {
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
        ArbiterRuling::unfinished("fallback", note)
    }

    fn finished(verdict: ArbiterVerdict) -> ArbiterRuling {
        ArbiterRuling::finished(Vec::new(), verdict)
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
        let (mut session, id, mut context) = parked_session();
        let mut interrupts = Interrupts::new(
            Scripted::new(vec![
                unfinished("no rule"),
                finished(ArbiterVerdict::Continue),
            ]),
            None,
            None,
            ArbiterVerdict::Stop,
        );

        interrupts
            .pump(&mut session, &mut context)
            .expect("the pump runs");

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
        let (mut session, id, mut context) = parked_session();
        let mut interrupts = Interrupts::new(
            Scripted::new(vec![unfinished("no rule")]),
            None,
            None,
            ArbiterVerdict::Stop,
        );

        interrupts
            .pump(&mut session, &mut context)
            .expect("the pump runs");
        assert_eq!(session.parked(), vec![id], "the first round did not decide");

        interrupts
            .arbiter
            .rounds
            .push(finished(ArbiterVerdict::Stop));
        interrupts
            .pump(&mut session, &mut context)
            .expect("the pump runs");

        assert!(session.instance(id).is_none());
        assert!(session.is_idle());
    }

    #[test]
    fn actions_are_applied_before_the_completion() {
        let rig = rig(1);
        let mut first = action(vec![intr(0), halt()], &rig);
        first.push_reason("obstacle");
        let first = verified(first);

        let mut context = RunContext::new(&first);
        let (mut session, interrupted) = Host::new(DummyBackend::new(rig.clone()))
            .load(first)
            .expect("assembled for this rig");
        context.push(interrupted, 0, [0; 32], "a.csp".into());

        let other = session
            .load(verified(action(vec![yield_(), jump(-1)], &rig)))
            .expect("assembled for this rig");
        context.push(other, 0, [0; 32], "b.csp".into());

        session.tick().expect("the dummy backend cannot fail");
        let mut interrupts = Interrupts::new(
            Scripted::new(vec![ArbiterRuling::finished(
                vec![Action::Unload {
                    instance: InstanceRef::new(other.get()),
                }],
                ArbiterVerdict::Continue,
            )]),
            None,
            None,
            ArbiterVerdict::Stop,
        );

        interrupts
            .pump(&mut session, &mut context)
            .expect("the pump runs");

        assert!(
            session.instance(other).is_none(),
            "the action removed the other instance"
        );
        assert!(session.instance(interrupted).is_some());
        assert!(session.parked().is_empty(), "the verdict still applied");
    }

    #[test]
    fn a_load_action_adds_an_instance_through_the_registry() {
        let rig = rig(1);
        let mut first = action(vec![intr(0), halt()], &rig);
        first.push_reason("obstacle");
        let first = verified(first);

        let mut context = RunContext::new(&first);
        let (mut session, interrupted) = Host::new(DummyBackend::new(rig.clone()))
            .load(first)
            .expect("assembled for this rig");
        context.push(interrupted, 0, [0; 32], "a.csp".into());
        session.tick().expect("the dummy backend cannot fail");

        let dir = std::env::temp_dir().join(format!("csp-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch directory is created");
        let next = action(vec![yield_(), jump(-1)], &rig);
        std::fs::write(dir.join("next.csp"), next.to_bytes()).expect("the action is written");

        let resolver = ActionDirectory::open(&dir).expect("the directory opens");
        let mut interrupts = Interrupts::new(
            Scripted::new(vec![ArbiterRuling::finished(
                vec![Action::Load {
                    action: "next.csp".into(),
                }],
                ArbiterVerdict::Stop,
            )]),
            Some(Box::new(resolver)),
            None,
            ArbiterVerdict::Stop,
        );

        interrupts
            .pump(&mut session, &mut context)
            .expect("the pump runs");

        assert!(
            session.instance(interrupted).is_none(),
            "the old instance stopped"
        );
        assert_eq!(session.instances(), 1, "the loaded action is running");
        assert_eq!(context.instances.len(), 1, "and the host tracked it");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
