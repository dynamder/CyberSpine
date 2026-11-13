//! The default demo: an unsigned junction, a no-progress interrupt, a back-off,
//! then a left/right image chosen by the arbiter.

use cyberspine_arbiter::{Action, Verdict};
use cyberspine_demo_maze::{MAX_TICKS, Session};
use cyberspine_host::Report;

fn run(mut session: Session) -> Session {
    while session.running() && session.ticks() < MAX_TICKS {
        session
            .advance(f64::from(session.dt()))
            .expect("the linked body cannot fail");
    }
    session
}

#[test]
fn the_default_demo_backs_off_then_turns_at_the_unsigned_junction() {
    let mut session = run(Session::handover().expect("the demo assembles and loads"));
    let reports = session.reports().to_vec();

    let interrupts: Vec<_> = reports
        .iter()
        .filter_map(|report| match report {
            Report::Interrupt { event, reason, .. } => Some((*event, reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(interrupts.len(), 1, "{reports:#?}");
    assert!(
        interrupts[0]
            .1
            .contains("no progress: x=4 y=11 heading=1 bumps=4"),
        "{:?}",
        interrupts[0].1
    );

    let rounds: Vec<_> = reports
        .iter()
        .filter_map(|report| match report {
            Report::Round { actions, note, .. } => Some((actions.clone(), note.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(rounds.len(), 1, "{reports:#?}");
    assert!(
        matches!(
            rounds[0].0.as_slice(),
            [Action::Silence { .. }, Action::Load { action }] if action.as_ref() == "reverse.csa"
        ),
        "{:?}",
        rounds[0].0
    );
    assert!(rounds[0].1.contains("backing off"), "{:?}", rounds[0].1);

    let finished: Vec<_> = reports
        .iter()
        .filter_map(|report| match report {
            Report::Finished {
                actions,
                verdict,
                note,
                ..
            } => Some((actions.clone(), *verdict, note.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(finished.len(), 1, "{reports:#?}");
    assert_eq!(finished[0].1, Verdict::Stop);
    assert!(finished[0].0.iter().any(
        |action| matches!(action, Action::Load { action } if action.as_ref() == "maze_left.csa")
    ));
    assert!(
        finished[0].2.contains("turning left"),
        "{:?}",
        finished[0].2
    );

    let outcome = session.outcome().expect("the linked body cannot fail");
    assert!(outcome.solved(), "{outcome:?}");
    assert_eq!((outcome.moves, outcome.turns, outcome.bumps), (26, 6, 4));
    assert!(outcome.ticks <= 45, "took {} ticks", outcome.ticks);
}
