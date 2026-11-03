//! `INTR`: the reason template, the instruction, and the parked state
//! (`docs/isa.md` §4.8, `docs/binfmt.md` §6.6).

use cyberspine_core::binfmt::load::{LoadError, VerifiedImage};
use cyberspine_core::binfmt::reason::{ReasonView, Template, TemplateError};
use cyberspine_core::exec::{Instance, InterruptError, Program, Sensors, Status, Step};
use cyberspine_core::isa::OpCode;
use cyberspine_testkit::{action, halt, intr, rig, verified};

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

struct View {
    f1: f32,
    i2: i32,
    vf: f32,
    vi: i32,
}

impl ReasonView for View {
    fn f(&self, reg: u8) -> f32 {
        assert_eq!(reg, 1, "the fixture only reads f1");
        self.f1
    }

    fn i(&self, reg: u8) -> i32 {
        assert_eq!(reg, 2, "the fixture only reads i2");
        self.i2
    }

    fn vf(&self, var: u32) -> Option<f32> {
        (var == 0).then_some(self.vf)
    }

    fn vi(&self, var: u32) -> Option<i32> {
        (var == 0).then_some(self.vi)
    }
}

#[test]
fn a_template_renders_registers_variables_and_braces() {
    let view = View {
        f1: 1.5,
        i2: -3,
        vf: 0.5,
        vi: 7,
    };
    let template =
        Template::parse("x={f1:.2} i={i2} vf={vf:0} vi={vi:0} {{lit}}").expect("valid template");

    assert_eq!(
        template.render(&view),
        "x=1.50 i=-3 vf=0.5 vi=7 {lit}",
        "precision applies to floats only; braces are literals"
    );
}

#[test]
fn a_source_name_resolves_to_a_var_index() {
    let mut template = Template::parse("phase={vf:phase}").expect("valid source template");
    template
        .resolve_vars(|name| (name == "phase").then_some(2), |_| None)
        .expect("phase is declared");

    assert_eq!(template.canonical(), "phase={vf:2}");
}

#[test]
fn a_stored_template_must_not_name_a_variable() {
    let template = Template::parse("{vf:phase}").expect("syntactically valid");
    assert_eq!(
        template.check_stored(1, 0),
        Err(TemplateError::NameInStored {
            name: "phase".to_string()
        })
    );
}

#[test]
fn a_stored_index_must_be_inside_its_table() {
    let template = Template::parse("{vf:5}").expect("syntactically valid");
    assert_eq!(
        template.check_stored(2, 0),
        Err(TemplateError::StoredVarIndexOutOfRange { index: 5, count: 2 })
    );
}

#[test]
fn a_register_outside_0_to_63_is_refused() {
    assert_eq!(
        Template::parse("{f64}"),
        Err(TemplateError::RegisterOutOfRange {
            text: "64".to_string()
        })
    );
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn interrupt_image(reason: &str) -> VerifiedImage {
    let mut image = action(vec![intr(0), halt()], &rig(0));
    image.push_reason(reason);
    verified(image)
}

/// `action` always appends `dt` and `tick`, so an instance needs one of each.
fn sensors() -> Sensors<'static> {
    Sensors::new(&[0.0], &[0])
}

#[test]
fn an_intr_image_loads_and_keeps_its_reason() {
    let image = interrupt_image("state={f1:.2}");
    assert_eq!(image.code()[0].op, OpCode::Intr);
    assert_eq!(image.reasons().len(), 1);
    assert_eq!(image.reason(0), "state={f1:.2}");
}

#[test]
fn an_intr_with_no_reason_row_is_refused() {
    let image = action(vec![intr(0), halt()], &rig(0));
    match VerifiedImage::verify(image) {
        Err(LoadError::ReasonIndexOutOfRange {
            index: 0,
            reason_id: 0,
            count: 0,
        }) => {}
        other => panic!("an INTR past the reason table was not refused: {other:?}"),
    }
}

#[test]
fn an_empty_reason_is_refused() {
    let mut image = action(vec![intr(0), halt()], &rig(0));
    image.push_reason("");
    match VerifiedImage::verify(image) {
        Err(LoadError::EmptyReason { index: 0 }) => {}
        other => panic!("an empty reason was not refused: {other:?}"),
    }
}

#[test]
fn a_stored_template_naming_a_variable_is_refused_at_load() {
    let mut image = action(vec![intr(0), halt()], &rig(0));
    image.push_reason("{vf:phase}");
    match VerifiedImage::verify(image) {
        Err(LoadError::InvalidReason { index: 0, source }) => {
            assert_eq!(
                source,
                TemplateError::NameInStored {
                    name: "phase".to_string()
                }
            );
        }
        other => panic!("a stored name was not refused: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[test]
fn an_intr_parks_until_the_host_answers() {
    let image = interrupt_image("why={f1}");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume(&program, &sensors(), 3),
        Step::Interrupted { reason: 0, pc: 0 }
    );
    assert_eq!(instance.status(), Status::Interrupted);
    assert_eq!(instance.interrupt().map(|pending| pending.reason), Some(0));

    assert_eq!(instance.resume(&program, &sensors(), 4), Step::Parked);
    assert_eq!(
        instance.pc(),
        1,
        "the resume point is the instruction after INTR"
    );

    instance
        .resume_interrupted(0)
        .expect("the host answers the reason it was told about");
    assert_eq!(instance.status(), Status::Suspended);
    assert_eq!(instance.resume(&program, &sensors(), 5), Step::Halted);
}

#[test]
fn answering_the_wrong_reason_is_refused() {
    let image = interrupt_image("why");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);
    instance.resume(&program, &sensors(), 0);

    assert_eq!(
        instance.resume_interrupted(9),
        Err(InterruptError::WrongReason {
            expected: 0,
            given: 9
        })
    );
    assert_eq!(
        instance.status(),
        Status::Interrupted,
        "a refused answer leaves the instance parked"
    );
}

#[test]
fn a_running_instance_cannot_be_answered() {
    let image = interrupt_image("why");
    let program = Program::new(&image);
    let mut instance = Instance::new(&program);

    assert_eq!(
        instance.resume_interrupted(0),
        Err(InterruptError::NotInterrupted)
    );
}
