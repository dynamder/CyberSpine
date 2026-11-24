//! The named limits are a contract: a document that says "4 MiB" and a
//! constant that drifts to 8 MiB is the same class of bug as a wrong exit code.

use cyberspine_binfmt::image::MAX_IMAGE_LEN;
use cyberspine_binfmt::reason::MAX_REASON_LEN;

#[test]
fn the_documented_limits_are_what_the_code_enforces() {
    assert_eq!(MAX_IMAGE_LEN, 4 * 1024 * 1024, "docs/binfmt.md C0");
    assert_eq!(MAX_REASON_LEN, 4096, "docs/binfmt.md reason rows");
}
