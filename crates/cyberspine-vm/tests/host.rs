//! The host's timing contract (`docs/isa.md` §8.4): what happens inside one
//! tick, in what order, and what a frame may do to it.

use cyberspine_core::binfmt::image::{Image, Sensor, SensorSource, VarF};
use cyberspine_core::isa::register::RegId;
use cyberspine_core::isa::{Insn, OpCode};
use cyberspine_vm::{Call, DummyBackend, Host, LoadError, RigLayout};

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

fn action(code: Vec<Insn>, muscles: usize) -> Image {
    let mut image = Image::new(code);
    for index in 0..muscles {
        image.push_muscle(&format!("m{index}"));
    }
    image
}

fn rig(muscles: usize) -> DummyBackend {
    DummyBackend::new(RigLayout {
        sensors_f: muscles,
        sensors_i: 0,
        muscles,
    })
}

/// `FEXCITE` once, `FLUSH` once, then yield forever without touching b2 again.
fn publish_then_idle(muscle: usize, value: f32, muscles: usize) -> Image {
    action(
        vec![
            fmovi(1, value),
            excite(muscle, 1),
            flush(),
            yield_(),
            jump(-2),
        ],
        muscles,
    )
}

// ---------------------------------------------------------------------------
// The shape of one tick
// ---------------------------------------------------------------------------

#[test]
fn the_first_tick_sees_the_rigs_state() {
    let mut backend = rig(1);
    backend.set_muscle(0, 0.75);
    let mut host = Host::new(backend);

    let id = host
        .load(action(vec![sense(1, 0), yield_(), jump(-2)], 1))
        .expect("same shape as the rig");
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
    let mut host = Host::new(rig(0));
    host.load(Image::new(vec![yield_(), jump(-2)])).expect("empty rig");

    host.tick();

    assert_eq!(
        host.backend().calls(),
        &[Call::Sample, Call::Commit, Call::Step]
    );
}

#[test]
fn a_terminated_instance_contributes_on_its_last_tick_only() {
    let mut host = Host::new(rig(1));
    host.load(action(
        vec![fmovi(1, 0.5), excite(0, 1), flush(), halt()],
        1,
    ))
    .expect("same shape");

    host.tick();
    assert_eq!(
        host.backend().committed(),
        &[Some(0.5)],
        "the halt happens after the command was published"
    );
    assert_eq!(host.instances(), 0, "removed after the step");

    host.tick();
    assert_eq!(
        host.backend().committed(),
        &[None],
        "nothing is left to contribute"
    );
}

#[test]
fn excite_without_flush_never_reaches_the_backend() {
    let mut host = Host::new(rig(1));
    host.load(action(vec![fmovi(1, 0.9), excite(0, 1), halt()], 1))
        .expect("same shape");

    host.tick();

    assert_eq!(host.backend().committed(), &[None]);
    assert_eq!(host.backend().muscles(), &[0.0]);
}

#[test]
fn a_published_value_is_held_by_the_instance_not_the_host() {
    let mut host = Host::new(rig(1));
    host.load(publish_then_idle(0, 0.25, 1)).expect("same shape");

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
    let mut host = Host::new(rig(2));
    host.load(publish_then_idle(0, 0.25, 2)).expect("first");
    host.load(publish_then_idle(1, 0.5, 2)).expect("second");

    host.tick();

    assert_eq!(
        host.backend().committed(),
        &[Some(0.25), Some(0.5)],
        "each muscle is covered by the instance that published it"
    );
}

#[test]
fn a_later_instance_of_the_same_layer_wins() {
    let mut host = Host::new(rig(1));
    host.load(publish_then_idle(0, 0.25, 1)).expect("first");
    host.load(publish_then_idle(0, 0.5, 1)).expect("second");

    host.tick();

    assert_eq!(host.backend().committed(), &[Some(0.5)]);
}

#[test]
fn a_higher_layer_wins_even_when_it_was_loaded_first() {
    let mut host = Host::new(rig(1));

    let mut high = publish_then_idle(0, 0.75, 1);
    high.header.layer = 2;
    host.load(high).expect("high layer");

    let mut low = publish_then_idle(0, 0.25, 1);
    low.header.layer = 1;
    host.load(low).expect("low layer");

    host.tick();

    assert_eq!(host.backend().committed(), &[Some(0.75)]);
}

#[test]
fn a_higher_layer_that_publishes_nothing_does_not_shadow_a_lower_one() {
    let mut host = Host::new(rig(1));

    let mut high = action(vec![yield_(), jump(-2)], 1);
    high.header.layer = 2;
    host.load(high).expect("high layer, silent");

    let mut low = publish_then_idle(0, 0.25, 1);
    low.header.layer = 1;
    host.load(low).expect("low layer, publishing");

    host.tick();

    assert_eq!(host.backend().committed(), &[Some(0.25)]);
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

#[test]
fn a_frame_runs_exactly_the_ticks_its_time_contains() {
    let mut host = Host::new(rig(0));
    host.load(Image::new(vec![yield_(), jump(-2)])).expect("empty rig");
    let dt = f64::from(host.dt());

    host.advance(3.0 * dt);
    assert_eq!(host.now(), 3);
    assert_eq!(host.backend().calls().len(), 9, "three ticks of three calls");

    host.advance(0.5 * dt);
    assert_eq!(host.now(), 3, "a partial tick waits in the remainder");

    host.advance(0.5 * dt);
    assert_eq!(host.now(), 4, "the remainder is not lost");
}

#[test]
fn a_waiting_instance_is_not_resumed_early() {
    let mut host = Host::new(rig(0));
    let mut image = Image::new(vec![wait(0.25), imovi(1, 1), yield_(), jump(-2)]);
    image.header.hz = 64;
    let id = host.load(image).expect("z64 clock");

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
    let mut host = Host::new(rig(0));
    let id = host
        .load(Image::new(vec![
            imovi(2, 1),
            binary(OpCode::Iadd, 1, 1, 2),
            wait(0.0),
            jump(-3),
        ]))
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
    let mut host = Host::new(rig(0));
    host.load(Image::new(vec![yield_(), jump(-2)])).expect("empty rig");
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
    let mut host = Host::new(rig(1));

    host.load(action(
        vec![
            fmovi(1, 1.0),
            excite(0, 1),
            flush(),
            yield_(),
            jump(-2),
        ],
        1,
    ))
    .expect("writer");

    let mut reader = action(vec![sense(1, 0), var_set(0, 1), yield_(), jump(-4)], 1);
    reader.vars_f.push(VarF {
        name: "seen".into(),
        init: 0.0,
    });
    let reader_id = host.load(reader).expect("reader");

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
    let mut host = Host::new(rig(0));

    let mut faulty = Image::new(vec![nop(), nop(), yield_()]);
    faulty.header.fuel = 1;
    let faulty_id = host.load(faulty).expect("faulty");

    let healthy_id = host.load(Image::new(vec![yield_(), jump(-2)])).expect("healthy");

    host.tick();

    assert!(host.instance(faulty_id).is_none(), "fuel exhaustion removed it");
    assert!(host.instance(healthy_id).is_some(), "the other one runs on");
}

// ---------------------------------------------------------------------------
// Tracing and load checks
// ---------------------------------------------------------------------------

#[test]
fn the_same_inputs_reproduce_the_same_trace() {
    let run = || {
        let mut host = Host::new(rig(1));
        host.set_tracing(true);
        host.load(publish_then_idle(0, 0.5, 1)).expect("same shape");
        for _ in 0..4 {
            host.tick();
        }
        host.trace().to_vec()
    };

    assert_eq!(run(), run());
    assert_eq!(run()[0].commands, vec![Some(0.5)]);
}

#[test]
fn an_image_that_does_not_match_the_rig_is_refused() {
    let mut host = Host::new(rig(2));

    let error = host
        .load(action(vec![yield_(), jump(-2)], 1))
        .expect_err("one muscle against a two-muscle rig");

    assert!(matches!(
        error,
        LoadError::SensorFCount {
            declared: 1,
            rig: 2
        }
    ));
}

#[test]
fn images_with_different_clocks_are_refused() {
    let mut host = Host::new(rig(0));

    let mut first = Image::new(vec![yield_(), jump(-2)]);
    first.header.hz = 60;
    host.load(first).expect("first clock");

    let mut second = Image::new(vec![yield_(), jump(-2)]);
    second.header.hz = 30;
    let error = host.load(second).expect_err("clock mismatch");

    assert!(matches!(
        error,
        LoadError::Hz {
            declared: 30,
            host: 60
        }
    ));
}

#[test]
fn a_halted_instance_is_removed_after_its_tick() {
    let mut host = Host::new(rig(0));
    let id = host.load(Image::new(vec![halt()])).expect("halt");

    host.tick();

    assert_eq!(host.instances(), 0);
    assert!(host.instance(id).is_none());
}

#[test]
fn builtin_sensors_are_filled_by_the_host() {
    let backend = DummyBackend::new(RigLayout {
        sensors_f: 1,
        sensors_i: 1,
        muscles: 0,
    });
    let mut image = Image::new(vec![sense(1, 0), isense(2, 0), yield_(), jump(-3)]);
    image.sensors_f.push(Sensor {
        name: "dt".into(),
        source: SensorSource::BuiltinDt,
    });
    image.sensors_i.push(Sensor {
        name: "tick".into(),
        source: SensorSource::BuiltinTick,
    });

    let mut host = Host::new(backend);
    let id = host.load(image).expect("builtin rig");

    host.tick();
    let instance = host.instance(id).expect("suspended");
    assert_eq!(instance.registers().get_f(reg(1)), host.dt());
    assert_eq!(instance.registers().get_i(reg(2)), 0);

    host.tick();
    assert_eq!(
        host.instance(id).expect("suspended").registers().get_i(reg(2)),
        1
    );
}
