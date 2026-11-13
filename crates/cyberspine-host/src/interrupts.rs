//! The host-side loop: deliver every ticket, take the ruling stream, apply a
//! round's atomic actions, then its completion.
//!
//! The host knows only "this event is not finished yet" (`docs/interrupt.md`
//! §7.1). Rounds never touch the instance, so an unfinished stream leaves the
//! guest parked exactly like a missing verdict did before.

use std::collections::HashMap;

use cyberspine_arbiter::{
    Action, Arbiter, Batch, Completion, Decision, EventId, InstanceRef, InterruptEvent,
    Verdict as ArbiterVerdict,
};
use cyberspine_vm::{
    Backend, InstanceId, Interrupt, Resolution, RigPolicy, Ruling, Session, Verdict,
};

use crate::actions::ActionResolver;
use crate::decisions::{Log, LogError, Record};
use crate::registry::Registry;
use crate::report::{Report, RoundTrace};

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

/// Applies one arbiter's ruling stream to a session.
pub struct Interrupts<A: Arbiter> {
    pub(crate) arbiter: A,
    inflight: HashMap<EventId, InFlight>,
    resolver: Option<Box<dyn ActionResolver>>,
    log: Option<Log>,
    /// What the host answers when the arbiter itself is gone.
    default: ArbiterVerdict,
}

impl<A: Arbiter> Interrupts<A> {
    pub fn new(
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

    /// How many tickets the arbiter still owes an answer.
    pub fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    pub fn pump<B: Backend, P: RigPolicy>(
        &mut self,
        session: &mut Session<B, P>,
        registry: &mut Registry,
    ) -> Result<Vec<Report>, LogError> {
        let mut reports = Vec::new();

        for ticket in session.take_interrupts() {
            let event = EventId::new(ticket.seq());
            let (action, instances, image_hash) = registry.snapshot(ticket.instance);
            let actions = self
                .resolver
                .as_ref()
                .map(|resolver| resolver.actions())
                .unwrap_or_default();
            reports.push(Report::Interrupt {
                event,
                instance: ticket.instance,
                action: action.clone(),
                pc: ticket.pc,
                tick: ticket.tick,
                reason: ticket.reason.clone(),
            });
            self.arbiter.deliver(InterruptEvent {
                event,
                instance: InstanceRef::new(ticket.instance.get()),
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
                reports.push(Report::Unknown { event });
                continue;
            };
            if let Some(log) = self.log.as_mut() {
                log.write(&Record::new(event, now, &ruling))?;
            }
            let actions = ruling.actions.clone();
            apply_actions(
                session,
                registry,
                self.resolver.as_deref(),
                event,
                ruling.actions,
                &mut reports,
            );
            match ruling.completion {
                Completion::Unfinished => {
                    let round = inflight.trail.len() + 1;
                    reports.push(Report::Round {
                        event,
                        tick: now,
                        round,
                        source: ruling.source.clone(),
                        note: ruling.note.clone(),
                        actions,
                    });
                    inflight.trail.push(Round {
                        source: ruling.source,
                        note: ruling.note,
                    });
                    self.repark(session, event, inflight, &mut reports);
                }
                Completion::Finished(verdict) => {
                    let last = inflight.trail.last().map(|round| RoundTrace {
                        source: round.source.clone(),
                        note: round.note.clone(),
                    });
                    reports.push(Report::Finished {
                        event,
                        tick: now,
                        rounds: inflight.trail.len(),
                        last,
                        note: ruling.note.clone(),
                        actions,
                        verdict,
                    });
                    let instance = inflight.ticket.instance;
                    let settled = settle(session, inflight.ticket, verdict, &mut reports);
                    forget(settled, verdict, registry, instance);
                }
            }
        }

        if let Some(error) = failure {
            reports.push(Report::Failed {
                message: error.to_string().into_boxed_str(),
            });
            for (_, inflight) in self.inflight.drain() {
                let instance = inflight.ticket.instance;
                let settled = settle(session, inflight.ticket, self.default, &mut reports);
                forget(settled, self.default, registry, instance);
            }
        }

        Ok(reports)
    }

    fn repark<B: Backend, P: RigPolicy>(
        &mut self,
        session: &mut Session<B, P>,
        event: EventId,
        mut inflight: InFlight,
        reports: &mut Vec<Report>,
    ) {
        match session.resolve(inflight.ticket, Ruling::Unfinished) {
            Ok(Resolution::Parked(ticket)) => {
                inflight.ticket = ticket;
                self.inflight.insert(event, inflight);
            }
            Ok(Resolution::Settled) => {}
            Err(error) => reports.push(Report::Carry {
                event,
                message: error.to_string().into_boxed_str(),
            }),
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
    registry: &mut Registry,
    resolver: Option<&dyn ActionResolver>,
    event: EventId,
    actions: Vec<Action>,
    reports: &mut Vec<Report>,
) {
    for action in actions {
        match action {
            Action::Load { action } => {
                let Some(resolver) = resolver else {
                    reports.push(action_report(
                        event,
                        format!("cannot load `{action}` because no action directory was given"),
                    ));
                    continue;
                };
                let image = match resolver.resolve(&action) {
                    Ok(image) => image,
                    Err(error) => {
                        reports.push(action_report(
                            event,
                            format!("`{action}` cannot load: {error}"),
                        ));
                        continue;
                    }
                };
                let layer = image.header().layer;
                let hash = image.content_hash();
                match session.load(image) {
                    Ok(id) => registry.push(id, layer, hash, action),
                    Err(error) => reports.push(action_report(
                        event,
                        format!("`{action}` does not fit: {error}"),
                    )),
                }
            }
            Action::Unload { instance } => {
                let Some(id) = registry.id_of(instance) else {
                    reports.push(action_report(event, format!("unknown instance {instance}")));
                    continue;
                };
                if session.unload(id) {
                    registry.remove(id);
                } else {
                    reports.push(action_report(
                        event,
                        format!("instance {instance} was already gone"),
                    ));
                }
            }
            Action::Silence { instance } => {
                set_silence(session, registry, event, instance, true, reports);
            }
            Action::Restore { instance } => {
                set_silence(session, registry, event, instance, false, reports);
            }
        }
    }
}

fn set_silence<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    registry: &Registry,
    event: EventId,
    instance: InstanceRef,
    silenced: bool,
    reports: &mut Vec<Report>,
) {
    let Some(id) = registry.id_of(instance) else {
        reports.push(action_report(event, format!("unknown instance {instance}")));
        return;
    };
    session.set_silenced(id, silenced);
}

fn action_report(event: EventId, message: String) -> Report {
    Report::Action {
        event,
        message: message.into_boxed_str(),
    }
}

/// Apply a terminal verdict, returning whether it landed.
fn settle<B: Backend, P: RigPolicy>(
    session: &mut Session<B, P>,
    ticket: Interrupt,
    verdict: ArbiterVerdict,
    reports: &mut Vec<Report>,
) -> bool {
    let verdict = match verdict {
        ArbiterVerdict::Continue => Verdict::Continue,
        ArbiterVerdict::Stop => Verdict::Stop,
    };
    match session.resolve(ticket, Ruling::Finished(verdict)) {
        Ok(_) => true,
        Err(error) => {
            reports.push(Report::Verdict {
                message: error.to_string().into_boxed_str(),
            });
            false
        }
    }
}

/// Keep the host's bookkeeping in step with a verdict that removed an instance.
fn forget(settled: bool, verdict: ArbiterVerdict, registry: &mut Registry, id: InstanceId) {
    if settled && verdict == ArbiterVerdict::Stop {
        registry.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ActionDirectory;
    use cyberspine_arbiter::Ruling as ArbiterRuling;
    use cyberspine_testkit::{action, halt, intr, jump, rig, verified, yield_};
    use cyberspine_vm::{DummyBackend, Host, Strict};

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

    fn parked_session() -> (Session<DummyBackend, Strict>, InstanceId, Registry) {
        let rig = rig(1);
        let mut image = action(vec![intr(0), halt()], &rig);
        image.push_reason("obstacle");
        let image = verified(image);
        let context = Registry::new(&image);
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

        let mut context = Registry::new(&first);
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

        let mut context = Registry::new(&first);
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
        assert_eq!(context.loaded().len(), 1, "and the host tracked it");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
