//! The host's timing contract (`docs/isa.md` §8.4): what happens inside one
//! tick, in what order, and what a frame may do to it.
//!
//! The images here are bound to the rig they are built for
//! (`docs/isa.md` §13.3), which is what a real image does: the assembler writes
//! the manifest's fingerprint into the header. `action` keeps the two sides in
//! step — it emits exactly the metadata tables `generate_image` would emit for
//! the same rig, so the fixtures stay legal images, not just acceptable ones.

use cyberspine_core::binfmt::image::{Image, Sensor, SensorSource, VarF};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::exec::Fault;
use cyberspine_core::isa::register::RegId;
use cyberspine_core::isa::{Insn, OpCode};
use cyberspine_core::rig::Name;
use cyberspine_core::rig::{Fingerprint, Rig};
use cyberspine_vm::{Backend, Call, DummyBackend, FaultRecord, Host, LoadError, Snapshot};

fn n(text: &str) -> Name {
    Name::new(text).expect("test fixture name")
}

fn reg(id: u8) -> RegId {
    RegId::new(id).expect("id < RegId::COUNT")
}

fn halt() -> Insn {
    Insn::new(OpCode::Halt)
}

fn nop() -> Insn {
    Insn::new(OpCode::Nop)
}

fn yield_() -> Insn {
    Insn::new(OpCode::Yield)
}

fn flush() -> Insn {
    Insn::new(OpCode::Flush)
}

fn fmovi(rd: u8, value: f32) -> Insn {
    Insn {
        rd: reg(rd),
        imm: value.to_bits(),
        ..Insn::new(OpCode::Fmovi)
    }
}

fn imovi(rd: u8, value: i32) -> Insn {
    Insn {
        rd: reg(rd),
        imm: value as u32,
        ..Insn::new(OpCode::Imovi)
    }
}

fn binary(op: OpCode, rd: u8, ra: u8, rb: u8) -> Insn {
    Insn {
        rd: reg(rd),
        ra: reg(ra),
        rb: reg(rb),
        ..Insn::new(op)
    }
}

fn jump(off: i32) -> Insn {
    Insn {
        imm: off as u32,
        ..Insn::new(OpCode::Jmp)
    }
}

fn wait(seconds: f32) -> Insn {
    Insn {
        imm: seconds.to_bits(),
        ..Insn::new(OpCode::Wait)
    }
}

fn excite(muscle: usize, ra: u8) -> Insn {
    Insn {
        ra: reg(ra),
        imm: muscle as u32,
        ..Insn::new(OpCode::Fexcite)
    }
}

fn sense(rd: u8, index: usize) -> Insn {
    Insn {
        rd: reg(rd),
        imm: index as u32,
        ..Insn::new(OpCode::Fsense)
    }
}

fn isense(rd: u8, index: usize) -> Insn {
    Insn {
        rd: reg(rd),
        imm: index as u32,
        ..Insn::new(OpCode::Isense)
    }
}

fn var_set(index: usize, ra: u8) -> Insn {
    Insn {
        ra: reg(ra),
        imm: index as u32,
        ..Insn::new(OpCode::Fvset)
    }
}

/// A rig of `count` muscles named `m0`.., and no observables: the tests that do
/// not care about sensors only ever read a muscle back.
fn rig(count: usize) -> Rig {
    let mut rig = Rig::new("test", 1);
    for index in 0..count {
        rig = rig.muscle(n(&format!("m{index}")));
    }
    rig
}

fn verified(image: Image) -> VerifiedImage {
    VerifiedImage::verify(image).expect("the fixture is a legal image")
}

fn backend(rig: &Rig) -> DummyBackend {
    DummyBackend::new(rig.clone())
}

/// An image bound to `rig`, carrying the tables the assembler would have
/// generated for it (`docs/asm.md` §3.3).
///
/// A legal image ends in `HALT` (§5 puts the implicit one on the binder), so
/// this appends it when the fixture does not: the tests below that loop forever
/// simply never reach it.
fn action(mut code: Vec<Insn>, rig: &Rig) -> Image {
    if code.last().map(|insn| insn.op) != Some(OpCode::Halt) {
        code.push(halt());
    }
    let mut image = Image::new(code);
    image.header.rig = rig.fingerprint();

    for muscle in &rig.muscles {
        image.push_muscle(muscle.clone());
    }
    for name in &rig.sensors_f {
        image.sensors_f.push(Sensor {
            name: name.clone(),
            source: SensorSource::Backend,
        });
    }
    image.sensors_f.push(Sensor {
        name: n("dt"),
        source: SensorSource::BuiltinDt,
    });
    for name in &rig.sensors_i {
        image.sensors_i.push(Sensor {
            name: name.clone(),
            source: SensorSource::Backend,
        });
    }
    image.sensors_i.push(Sensor {
        name: n("tick"),
        source: SensorSource::BuiltinTick,
    });
    image
}

/// `FEXCITE` once, `FLUSH` once, then yield forever without touching b2 again.
fn publish_then_idle(muscle: usize, value: f32, rig: &Rig) -> Image {
    action(
        vec![
            fmovi(1, value),
            excite(muscle, 1),
            flush(),
            yield_(),
            jump(-2),
        ],
        rig,
    )
}

// ---------------------------------------------------------------------------
// The shape of one tick
// ---------------------------------------------------------------------------

#[test]
fn the_first_tick_sees_the_rigs_state() {
    let rig = rig(1);
    let mut backend = backend(&rig);
    backend.set_muscle(0, 0.75);
    let mut host = Host::new(backend);

    let id = host
        .load(verified(action(
            vec![sense(1, 0), yield_(), jump(-2)],
            &rig,
        )))
        .expect("assembled for this rig");
    host.tick();

    assert_eq!(
        host.instance(id)
            .expect("still suspended")
            .registers()
            .get_f(reg(1)),
        0.75,
        "tick 0 samples the rig instead of starting from zeros"
    );
    assert_eq!(host.now(), 1);
}

#[test]
fn a_tick_is_sample_commit_step_in_that_order() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));
    host.load(verified(action(vec![yield_(), jump(-2)], &rig)))
        .expect("empty rig");

    host.tick();

    assert_eq!(
        host.backend().calls(),
        &[Call::Sample, Call::Commit, Call::Step]
    );
}

/// The trait's contract, and the template for holding any backend to it:
/// `sample` reports the body, it does not advance it.
#[test]
fn sampling_twice_reports_the_same_body_state() {
    let rig = rig(2);
    let mut backend = backend(&rig);
    backend.set_muscle(0, 0.5);

    let mut no_f: [f32; 0] = [];
    let mut no_i: [i32; 0] = [];
    let mut first = [0.0; 2];
    let mut second = [0.0; 2];

    backend.sample(&mut Snapshot {
        f: &mut no_f,
        i: &mut no_i,
        muscles: &mut first,
    });
    backend.sample(&mut Snapshot {
        f: &mut no_f,
        i: &mut no_i,
        muscles: &mut second,
    });

    assert_eq!(first, second, "sampling is a read, not a step");
    assert_eq!(
        first,
        [0.5, 0.0],
        "the read-back is the body's state, not the last command"
    );
}

#[test]
fn a_terminated_instance_contributes_on_its_last_tick_only() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));
    host.load(verified(action(
        vec![fmovi(1, 0.5), excite(0, 1), flush(), halt()],
        &rig,
    )))
    .expect("same shape");

    host.tick();
    assert_eq!(
        host.backend().committed(),
        &[Some(0.5)],
        "the halt happens after the command was published"
    );
    assert_eq!(host.instances(), 0, "removed after the step");
    assert!(host.faults().is_empty(), "a halt is not a fault");

    host.tick();
    assert_eq!(
        host.backend().committed(),
        &[None],
        "nothing is left to contribute"
    );
}

#[test]
fn a_fault_is_recorded_and_outlives_the_instance() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));

    let mut faulty = action(vec![nop(), nop(), yield_()], &rig);
    faulty.header.fuel = 1;
    let id = host.load(verified(faulty)).expect("faulty");

    host.tick();

    assert!(host.instance(id).is_none(), "a fault removes the instance");
    assert_eq!(
        host.faults(),
        &[FaultRecord {
            instance: id,
            fault: Fault::FuelExhausted { pc: 1 }
        }],
        "the reason survives the slot that held it"
    );

    assert_eq!(host.take_faults().len(), 1);
    assert!(host.faults().is_empty(), "draining empties the log");
}

/// G12: a fault's contributions vanish in the same tick. A `HALT` on the last
/// tick is different — that command was legitimately published
/// (`docs/isa.md` §8.4 step 3).
#[test]
fn a_faulted_instance_contributes_nothing_that_tick() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));

    let mut faulty = action(
        vec![fmovi(1, 0.5), excite(0, 1), flush(), nop(), halt()],
        &rig,
    );
    // Three instructions of fuel: `FLUSH` publishes, then the fuel runs out
    // before `NOP` can run.
    faulty.header.fuel = 3;
    let id = host.load(verified(faulty)).expect("faulty");

    host.tick();

    assert_eq!(
        host.backend().committed(),
        &[None],
        "the excitation died with the instance"
    );
    assert_eq!(
        host.faults(),
        &[FaultRecord {
            instance: id,
            fault: Fault::FuelExhausted { pc: 3 }
        }],
        "and it was reported, not swallowed"
    );
    assert!(host.instance(id).is_none());
}

#[test]
fn excite_without_flush_never_reaches_the_backend() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));
    host.load(verified(action(
        vec![fmovi(1, 0.9), excite(0, 1), halt()],
        &rig,
    )))
    .expect("same shape");

    host.tick();

    assert_eq!(host.backend().committed(), &[None]);
    assert_eq!(host.backend().muscles(), &[0.0]);
}

#[test]
fn a_published_value_is_held_by_the_instance_not_the_host() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));
    host.load(verified(publish_then_idle(0, 0.25, &rig)))
        .expect("same shape");

    for _ in 0..3 {
        host.tick();
        assert_eq!(host.backend().committed(), &[Some(0.25)]);
    }
    assert_eq!(host.instances(), 1, "the instance is still yielding");
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

#[test]
fn merge_is_decided_per_muscle() {
    let rig = rig(2);
    let mut host = Host::new(backend(&rig));
    host.load(verified(publish_then_idle(0, 0.25, &rig)))
        .expect("first");
    host.load(verified(publish_then_idle(1, 0.5, &rig)))
        .expect("second");

    host.tick();

    assert_eq!(
        host.backend().committed(),
        &[Some(0.25), Some(0.5)],
        "each muscle is covered by the instance that published it"
    );
}

#[test]
fn a_later_instance_of_the_same_layer_wins() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));
    host.load(verified(publish_then_idle(0, 0.25, &rig)))
        .expect("first");
    host.load(verified(publish_then_idle(0, 0.5, &rig)))
        .expect("second");

    host.tick();

    assert_eq!(host.backend().committed(), &[Some(0.5)]);
}

#[test]
fn a_higher_layer_wins_even_when_it_was_loaded_first() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));

    let mut high = publish_then_idle(0, 0.75, &rig);
    high.header.layer = 2;
    host.load(verified(high)).expect("high layer");

    let mut low = publish_then_idle(0, 0.25, &rig);
    low.header.layer = 1;
    host.load(verified(low)).expect("low layer");

    host.tick();

    assert_eq!(host.backend().committed(), &[Some(0.75)]);
}

#[test]
fn a_higher_layer_that_publishes_nothing_does_not_shadow_a_lower_one() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));

    let mut high = action(vec![yield_(), jump(-2)], &rig);
    high.header.layer = 2;
    host.load(verified(high)).expect("high layer, silent");

    let mut low = publish_then_idle(0, 0.25, &rig);
    low.header.layer = 1;
    host.load(verified(low)).expect("low layer, publishing");

    host.tick();

    assert_eq!(host.backend().committed(), &[Some(0.25)]);
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

#[test]
fn a_frame_runs_exactly_the_ticks_its_time_contains() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));
    host.load(verified(action(vec![yield_(), jump(-2)], &rig)))
        .expect("empty rig");
    let dt = f64::from(host.dt());

    host.advance(3.0 * dt);
    assert_eq!(host.now(), 3);
    assert_eq!(
        host.backend().calls().len(),
        9,
        "three ticks of three calls"
    );

    host.advance(0.5 * dt);
    assert_eq!(host.now(), 3, "a partial tick waits in the remainder");

    host.advance(0.5 * dt);
    assert_eq!(host.now(), 4, "the remainder is not lost");
}

#[test]
fn a_waiting_instance_is_not_resumed_early() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));
    let mut image = action(vec![wait(0.25), imovi(1, 1), yield_(), jump(-2)], &rig);
    image.header.hz = 64;
    let id = host.load(verified(image)).expect("64 Hz image");

    host.tick();
    assert_eq!(
        host.instance(id).expect("waiting").wake_at(),
        16,
        "0.25 s at 64 Hz is exactly 16 ticks"
    );

    for _ in 0..15 {
        host.tick();
    }
    assert_eq!(
        host.instance(id)
            .expect("still waiting")
            .registers()
            .get_i(reg(1)),
        0,
        "now = 15 is one tick early"
    );

    host.tick();
    assert_eq!(
        host.instance(id)
            .expect("resumed")
            .registers()
            .get_i(reg(1)),
        1
    );
}

#[test]
fn wait_zero_runs_once_per_tick() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));
    let id = host
        .load(verified(action(
            vec![
                imovi(2, 1),
                binary(OpCode::Iadd, 1, 1, 2),
                wait(0.0),
                jump(-3),
            ],
            &rig,
        )))
        .expect("empty rig");

    for _ in 0..3 {
        host.tick();
    }

    assert_eq!(
        host.instance(id)
            .expect("still running")
            .registers()
            .get_i(reg(1)),
        3,
        "one iteration per tick, not a loop inside the tick"
    );
}

#[test]
fn a_slow_frame_drops_its_remainder_instead_of_spiralling() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));
    host.load(verified(action(vec![yield_(), jump(-2)], &rig)))
        .expect("empty rig");
    host.set_max_ticks_per_frame(8);
    let dt = f64::from(host.dt());

    host.advance(1000.0 * dt);

    assert_eq!(host.now(), 8);
    assert_eq!(host.overruns(), 1);
    assert_eq!(host.backend().calls().len(), 24);
}

// ---------------------------------------------------------------------------
// Cross-instance rules
// ---------------------------------------------------------------------------

#[test]
fn an_excitation_is_invisible_to_the_tick_that_made_it() {
    let rig = rig(1);
    let mut host = Host::new(backend(&rig));

    host.load(verified(action(
        vec![fmovi(1, 1.0), excite(0, 1), flush(), yield_(), jump(-2)],
        &rig,
    )))
    .expect("writer");

    let mut reader = action(vec![sense(1, 0), var_set(0, 1), yield_(), jump(-4)], &rig);
    reader.vars_f.push(VarF {
        name: n("seen"),
        init: 0.0,
    });
    let reader_id = host.load(verified(reader)).expect("reader");

    host.tick();
    assert_eq!(
        host.instance(reader_id).expect("suspended").vars_f()[0],
        0.0,
        "the snapshot was taken before the writer ran"
    );

    host.tick();
    assert_eq!(
        host.instance(reader_id).expect("suspended").vars_f()[0],
        1.0,
        "one tick later the excitation is observed"
    );
}

#[test]
fn a_fault_removes_only_the_faulting_instance() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));

    let mut faulty = action(vec![nop(), nop(), yield_()], &rig);
    faulty.header.fuel = 1;
    let faulty_id = host.load(verified(faulty)).expect("faulty");

    let healthy_id = host
        .load(verified(action(vec![yield_(), jump(-2)], &rig)))
        .expect("healthy");

    host.tick();

    assert!(
        host.instance(faulty_id).is_none(),
        "fuel exhaustion removed it"
    );
    assert!(host.instance(healthy_id).is_some(), "the other one runs on");
}

// ---------------------------------------------------------------------------
// Observables: image the k-th backend source, the k-th backend observable
// ---------------------------------------------------------------------------

#[test]
fn the_kth_backend_source_reads_the_kth_observable() {
    let rig = Rig::new("observe", 1)
        .muscle(n("m"))
        .sensor_f(n("up"))
        .sensor_f(n("down"))
        .sensor_i(n("ticks"));

    let mut backend = backend(&rig);
    backend.set_sensor_f(0, 0.25);
    backend.set_sensor_f(1, 0.75);
    backend.set_sensor_i(0, 7);

    let mut host = Host::new(backend);
    let image = action(
        // `sensors_f` is [m, up, down, dt] and `sensors_i` is [ticks, tick].
        vec![
            sense(1, 2),
            sense(2, 3),
            isense(3, 0),
            yield_(),
            jump(-5),
            halt(),
        ],
        &rig,
    );
    VerifiedImage::from_bytes(&image.to_bytes()).expect("the fixture is a legal image");
    let id = host.load(verified(image)).expect("assembled for this rig");

    host.tick();

    let registers = host.instance(id).expect("suspended").registers();
    assert_eq!(
        registers.get_f(reg(1)),
        0.75,
        "sensor 2 is the second backend observable, not the second sensor"
    );
    assert_eq!(registers.get_f(reg(2)), host.dt(), "dt is still appended");
    assert_eq!(registers.get_i(reg(3)), 7, "the first integer observable");
}

// ---------------------------------------------------------------------------
// Load checks
// ---------------------------------------------------------------------------

#[test]
fn an_image_assembled_for_another_rig_is_refused() {
    let mine = rig(1);
    let other = rig(2);
    let mut host = Host::new(backend(&mine));

    let error = host
        .load(verified(action(vec![yield_(), jump(-2)], &other)))
        .expect_err("two muscles against a one-muscle rig");

    assert!(
        matches!(
            error,
            LoadError::RigMismatch { declared, backend }
                if declared == other.fingerprint() && backend == mine.fingerprint()
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn an_unbound_image_is_refused_unless_allowed() {
    let rig = rig(1);
    let mut unbound = action(vec![yield_(), jump(-2)], &rig);
    unbound.header.rig = Fingerprint::UNBOUND;

    let mut host = Host::new(backend(&rig));
    let error = host
        .load(verified(unbound))
        .expect_err("no fingerprint, no proof");
    assert!(matches!(error, LoadError::Unbound), "unexpected: {error}");

    let mut host = Host::new(backend(&rig));
    host.allow_unbound();
    let mut unbound = action(vec![yield_(), jump(-2)], &rig);
    unbound.header.rig = Fingerprint::UNBOUND;
    host.load(verified(unbound))
        .expect("an unbound image is allowed when the host opts out");
}

#[test]
fn an_unbound_image_still_has_to_fit_the_hosts_shape() {
    let mut host = Host::new(backend(&rig(1)));
    host.allow_unbound();

    let mut first = action(vec![yield_(), jump(-2)], &rig(1));
    first.header.rig = Fingerprint::UNBOUND;
    host.load(verified(first))
        .expect("the first image fixes the shape");

    let mut wider = action(vec![yield_(), jump(-2)], &rig(2));
    wider.header.rig = Fingerprint::UNBOUND;
    let error = host
        .load(verified(wider))
        .expect_err("two muscles where the host holds one");

    assert!(
        matches!(
            error,
            LoadError::ShapeMismatch {
                table: "muscles",
                declared: 2,
                host: 1
            }
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn images_with_different_clocks_are_refused() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));

    let mut first = action(vec![yield_(), jump(-2)], &rig);
    first.header.hz = 60;
    host.load(verified(first)).expect("first clock");

    let mut second = action(vec![yield_(), jump(-2)], &rig);
    second.header.hz = 30;
    let error = host.load(verified(second)).expect_err("clock mismatch");

    assert!(
        matches!(
            error,
            LoadError::Hz {
                declared: 30,
                host: 60
            }
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn a_halted_instance_is_removed_after_its_tick() {
    let rig = rig(0);
    let mut host = Host::new(backend(&rig));
    let id = host
        .load(verified(action(vec![halt()], &rig)))
        .expect("halt");

    host.tick();

    assert_eq!(host.instances(), 0);
    assert!(host.instance(id).is_none());
}

#[test]
fn builtin_sensors_are_filled_by_the_host() {
    let rig = Rig::new("builtins", 1);
    let mut host = Host::new(backend(&rig));
    let image = action(vec![sense(1, 0), isense(2, 0), yield_(), jump(-3)], &rig);
    // `sensors_f` is [dt] and `sensors_i` is [tick]: the rig has no observables.
    let id = host.load(verified(image)).expect("builtin rig");

    host.tick();
    let instance = host.instance(id).expect("suspended");
    assert_eq!(instance.registers().get_f(reg(1)), host.dt());
    assert_eq!(instance.registers().get_i(reg(2)), 0);

    host.tick();
    assert_eq!(
        host.instance(id)
            .expect("suspended")
            .registers()
            .get_i(reg(2)),
        1
    );
}

#[test]
fn the_same_inputs_reproduce_the_same_trace() {
    let run = || {
        let rig = rig(1);
        let mut host = Host::new(backend(&rig));
        host.set_tracing(true);
        host.load(verified(publish_then_idle(0, 0.5, &rig)))
            .expect("same shape");
        for _ in 0..4 {
            host.tick();
        }
        host.trace().to_vec()
    };

    assert_eq!(run(), run());
    assert_eq!(run()[0].commands, vec![Some(0.5)]);
}
