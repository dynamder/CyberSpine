//! `Rig::fingerprint` is the only thing standing between an action and a body
//! it was not assembled for (`docs/isa.md` §13), so its inputs are pinned here.

use cyberspine_core::rig::{Fingerprint, Rig};

fn base() -> Rig {
    Rig::new("live2d-standard", 1)
        .muscle("mouth_open")
        .muscle("brow_l")
        .sensor_f("blink_speed")
        .sensor_i("gaze_lock")
}

#[test]
fn the_same_rig_hashes_the_same() {
    assert_eq!(base().fingerprint(), base().fingerprint());
}

#[test]
fn name_order_is_part_of_the_identity() {
    let swapped = Rig::new("live2d-standard", 1)
        .muscle("brow_l")
        .muscle("mouth_open")
        .sensor_f("blink_speed")
        .sensor_i("gaze_lock");

    assert_ne!(base().fingerprint(), swapped.fingerprint());
}

#[test]
fn the_identity_version_is_part_of_the_identity() {
    let bumped = Rig::new("live2d-standard", 2)
        .muscle("mouth_open")
        .muscle("brow_l")
        .sensor_f("blink_speed")
        .sensor_i("gaze_lock");

    assert_ne!(base().fingerprint(), bumped.fingerprint());
}

#[test]
fn moving_a_name_to_another_table_changes_the_fingerprint() {
    let as_sensor = Rig::new("live2d-standard", 1)
        .muscle("mouth_open")
        .muscle("brow_l")
        .sensor_f("gaze_lock")
        .sensor_f("blink_speed");

    assert_ne!(base().fingerprint(), as_sensor.fingerprint());
}

#[test]
fn length_prefixes_stop_adjacent_names_from_being_spliced() {
    let a = Rig::new("r", 1).muscle("ab").muscle("c");
    let b = Rig::new("r", 1).muscle("a").muscle("bc");

    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[test]
fn an_unbound_fingerprint_is_all_zero_and_prints_as_hex() {
    assert!(Fingerprint::UNBOUND.is_unbound());
    assert_eq!(
        Fingerprint::UNBOUND.to_string(),
        "0".repeat(32 * 2),
        "32 bytes, two hex digits each"
    );
    assert_ne!(base().fingerprint(), Fingerprint::UNBOUND);
}
