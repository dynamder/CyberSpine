//! The manifest is the assembler's only view of the body; a typo here must fail
//! at build time, not produce a silently wrong action.

use cyberspine_manifest::{ManifestError, Name, Rig, RigError, RigTable, from_json};

const GOOD: &str = r#"{
  "rig": { "name": "live2d-standard", "version": 1 },
  "muscles": ["mouth_open", "brow_l"],
  "sensors_f": ["blink_speed"],
  "sensors_i": ["gaze_lock"]
}"#;

fn muscles(names: &str) -> String {
    format!(
        r#"{{ "rig": {{ "name": "r", "version": 1 }}, "muscles": {names}, "sensors_f": [], "sensors_i": [] }}"#
    )
}

fn n(text: &str) -> Name {
    Name::new(text).expect("test fixture name")
}

#[test]
fn a_well_formed_manifest_parses_in_order() {
    let rig = from_json(GOOD).expect("valid manifest");

    assert_eq!(&*rig.name, "live2d-standard");
    assert_eq!(rig.version, 1);
    assert_eq!(
        rig.muscles.iter().map(|n| &**n).collect::<Vec<&str>>(),
        ["mouth_open", "brow_l"]
    );
    assert_eq!(
        rig.sensors_f.iter().map(|n| &**n).collect::<Vec<&str>>(),
        ["blink_speed"]
    );
    assert_eq!(
        rig.sensors_i.iter().map(|n| &**n).collect::<Vec<&str>>(),
        ["gaze_lock"]
    );
}

#[test]
fn the_manifest_and_the_code_build_the_same_fingerprint() {
    let from_file = from_json(GOOD).expect("valid manifest");
    let in_code = Rig::new("live2d-standard", 1)
        .muscle(n("mouth_open"))
        .muscle(n("brow_l"))
        .sensor_f(n("blink_speed"))
        .sensor_i(n("gaze_lock"));

    assert_eq!(from_file.fingerprint(), in_code.fingerprint());
}

#[test]
fn a_name_may_not_appear_twice_across_tables() {
    let text = muscles(r#"["mouth_open", "mouth_open"]"#);
    match from_json(&text) {
        Err(ManifestError::Rig(RigError::DuplicateName {
            table: RigTable::Muscles,
            index: 1,
            name,
        })) => assert_eq!(name.as_str(), "mouth_open"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn the_builtin_sensor_names_are_reserved() {
    for name in ["dt", "tick"] {
        let text = muscles(&format!(r#"["{name}"]"#));
        assert!(
            matches!(
                from_json(&text),
                Err(ManifestError::Rig(RigError::ReservedName { .. }))
            ),
            "{name} must be reserved"
        );
    }
}

#[test]
fn a_register_spelling_is_not_a_name() {
    let text = muscles(r#"["f3"]"#);
    assert!(matches!(
        from_json(&text),
        Err(ManifestError::Rig(RigError::ReservedName { .. }))
    ));
}

#[test]
fn a_name_that_is_not_an_identifier_is_refused() {
    for bad in ["9lives", "with-dash", "with space", "café"] {
        let text = muscles(&format!(r#"["{bad}"]"#));
        match from_json(&text) {
            Err(ManifestError::Rig(RigError::InvalidName {
                table: RigTable::Muscles,
                index: 0,
                name,
            })) => assert_eq!(&*name, bad),
            other => panic!("{bad:?} was not refused as a non-identifier: {other:?}"),
        }
    }
}

#[test]
fn an_unknown_field_is_refused() {
    let text = r#"{
      "rig": { "name": "r", "version": 1 },
      "muscles": [], "sensors_f": [], "sensors_i": [],
      "layer": 3
    }"#;
    assert!(matches!(from_json(text), Err(ManifestError::Json(_))));
}

#[test]
fn an_empty_rig_name_is_refused() {
    let text = r#"{
      "rig": { "name": "", "version": 1 },
      "muscles": [], "sensors_f": [], "sensors_i": []
    }"#;
    assert!(matches!(
        from_json(text),
        Err(ManifestError::Rig(RigError::EmptyName))
    ));
}

#[test]
fn a_broken_document_reports_json() {
    assert!(matches!(from_json("{"), Err(ManifestError::Json(_))));
}
