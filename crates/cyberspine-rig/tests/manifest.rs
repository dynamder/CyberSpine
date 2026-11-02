//! The manifest is the assembler's only view of the body; a typo here must fail
//! at build time, not produce a silently wrong action.

use cyberspine_rig::{ManifestError, Rig, from_json};

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
        .muscle("mouth_open")
        .muscle("brow_l")
        .sensor_f("blink_speed")
        .sensor_i("gaze_lock");

    assert_eq!(from_file.fingerprint(), in_code.fingerprint());
}

#[test]
fn a_name_may_not_appear_twice_across_tables() {
    let text = muscles(r#"["mouth_open", "mouth_open"]"#);
    assert!(matches!(
        from_json(&text),
        Err(ManifestError::DuplicateName { name }) if name == "mouth_open"
    ));
}

#[test]
fn the_builtin_sensor_names_are_reserved() {
    for name in ["dt", "tick"] {
        let text = muscles(&format!(r#"["{name}"]"#));
        assert!(
            matches!(from_json(&text), Err(ManifestError::ReservedName { .. })),
            "{name} must be reserved"
        );
    }
}

#[test]
fn a_register_spelling_is_not_a_name() {
    let text = muscles(r#"["f3"]"#);
    assert!(matches!(
        from_json(&text),
        Err(ManifestError::NotAnIdentifier { .. })
    ));
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
    assert!(matches!(from_json(text), Err(ManifestError::EmptyName)));
}

#[test]
fn a_broken_document_reports_json() {
    assert!(matches!(from_json("{"), Err(ManifestError::Json(_))));
}
