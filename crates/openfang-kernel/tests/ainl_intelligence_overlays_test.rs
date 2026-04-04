//! Integration test for intelligence overlay materialization.

use std::fs;

use tempfile::tempdir;

#[test]
fn materialize_overlays_writes_intelligence_dir() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();
    let n = openfang_kernel::ainl_intelligence_overlays::materialize_intelligence_overlays(home)
        .expect("materialize");
    assert!(n >= 1, "expected at least one overlay written");

    let p = home
        .join("ainl-library")
        .join("intelligence")
        .join("auto_tune_ainl_caps.lang");
    assert!(p.is_file(), "expected {}", p.display());
    let s = fs::read_to_string(&p).expect("read");
    assert!(
        s.contains("auto_tune_ainl_caps"),
        "overlay should contain program name"
    );

    let n2 = openfang_kernel::ainl_intelligence_overlays::materialize_intelligence_overlays(home)
        .expect("materialize idempotent");
    assert_eq!(n2, 0, "second run should be no-op when bytes match");
}
