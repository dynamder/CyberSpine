//! Source → image → host: the bytes the assembler writes are what actually run,
//! and they only run against the rig they were assembled for.

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::rig::{Name, Rig};
use cyberspine_vm::{DummyBackend, Host};

fn n(text: &str) -> Name {
    Name::new(text).expect("test fixture name")
}

const SOURCE: &str = r#"
.layer 0
.clock hz = 60
.fuel 4096
.max_wait 600.0

FMOVI   f1, 0.5
FEXCITE mouth, f1
FLUSH
YIELD
FMOVI   f1, 1.0
FEXCITE mouth, f1
FLUSH
HALT
"#;

#[test]
fn an_assembled_source_runs_and_produces_the_expected_commands() {
    let rig = Rig::new("end-to-end", 1).muscle(n("mouth"));

    let asm = parse(SOURCE).expect("the source parses");
    let image = generate_image(&asm, &rig).expect("the source assembles");

    let bytes = image.to_bytes();
    let loaded = VerifiedImage::from_bytes(&bytes).expect("the assembler's own output loads");

    let mut host = Host::new(DummyBackend::new(rig));
    host.set_tracing(true);
    host.load(loaded)
        .expect("the backend implements the rig the image was assembled for");

    for _ in 0..3 {
        host.tick();
    }

    let commands: Vec<Vec<Option<f32>>> = host
        .trace()
        .iter()
        .map(|tick| tick.commands.clone())
        .collect();

    assert_eq!(
        commands,
        vec![vec![Some(0.5)], vec![Some(1.0)], vec![None]],
        "the first tick yields 0.5, the second halts with 1.0, the third has no instance"
    );
}
