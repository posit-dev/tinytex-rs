//! The first CI-checked test: an empty crate builds and its test target runs
//! on every leg of the CI matrix.

#[test]
fn crate_builds_and_test_target_runs() {
    assert_eq!(tinytex::CRATE_NAME, "tinytex");
}
