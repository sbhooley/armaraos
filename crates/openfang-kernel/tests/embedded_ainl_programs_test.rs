//! Embedded `programs/` materialization writes under `ainl-library/armaraos-programs/`.

use std::fs;

use tempfile::tempdir;

#[test]
fn materialize_writes_expected_layout() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();
    let n = openfang_kernel::embedded_ainl_programs::materialize_embedded_programs(home)
        .expect("materialize");
    assert!(
        n > 0,
        "expected at least one embedded file written on first run"
    );

    let ping = home
        .join("ainl-library")
        .join("armaraos-programs")
        .join("armaraos_health_ping")
        .join("armaraos_health_ping.ainl");
    assert!(ping.is_file(), "expected health ping at {}", ping.display());

    let s = fs::read_to_string(&ping).expect("read");
    assert!(
        s.contains("armaraos_health_ping"),
        "graph name present in source"
    );

    let n2 = openfang_kernel::embedded_ainl_programs::materialize_embedded_programs(home)
        .expect("materialize idempotent");
    assert_eq!(n2, 0, "second run should not rewrite unchanged files");

    openfang_kernel::embedded_ainl_programs::ensure_ainl_library_pointer_files(home)
        .expect("pointers");
    let lib = home.join("ainl-library");
    assert!(lib.join("README.md").is_file());
    assert!(lib.join(".embedded-revision.txt").is_file());
}
