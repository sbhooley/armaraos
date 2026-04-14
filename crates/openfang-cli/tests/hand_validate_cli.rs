//! Integration tests for `openfang hand validate` (exit code + subprocess wiring).

use std::fs;
use std::io::Write;
use std::process::Command;

fn hand_toml(with_hand_schema: bool) -> String {
    let schema = if with_hand_schema {
        "schema_version = \"1\"\n"
    } else {
        ""
    };
    format!(
        r#"[hand]
id = "cli-test-hand"
name = "CLI Test"
version = "1.0.0"
ainl_ir_version = "1.0.0"
{schema}description = "test"
entrypoint = "graph.ainl.json"
"#
    )
}

fn write_emitter_pack(dir: &std::path::Path, with_hand_schema: bool, ir_has_schema: bool) {
    fs::create_dir_all(dir).unwrap();
    let mut f = fs::File::create(dir.join("HAND.toml")).unwrap();
    f.write_all(hand_toml(with_hand_schema).as_bytes()).unwrap();
    let ir = if ir_has_schema {
        r#"{"schema_version":"1","meta":{}}"#.to_string()
    } else {
        r#"{"meta":{}}"#.to_string()
    };
    fs::write(dir.join("graph.ainl.json"), ir).unwrap();
    fs::write(
        dir.join("security.json"),
        r#"{"schema_version":"1","version":"1.0"}"#,
    )
    .unwrap();
}

#[test]
fn hand_validate_clean_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    write_emitter_pack(tmp.path(), true, true);
    let out = Command::new(env!("CARGO_BIN_EXE_openfang"))
        .args(["hand", "validate", tmp.path().to_str().unwrap()])
        .output()
        .expect("spawn openfang");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("[PASS]"), "stdout={s}");
    assert!(s.contains("0 warning(s), 0 error(s)"), "stdout={s}");
}

#[test]
fn hand_validate_missing_schema_warns_exit_zero() {
    let tmp = tempfile::tempdir().unwrap();
    write_emitter_pack(tmp.path(), false, false);
    let out = Command::new(env!("CARGO_BIN_EXE_openfang"))
        .args(["hand", "validate", tmp.path().to_str().unwrap()])
        .output()
        .expect("spawn openfang");
    assert!(
        out.status.success(),
        "warnings must not change exit code; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("[WARN]"), "stdout={s}");
    assert!(s.contains("warning(s)"), "stdout={s}");
}

#[test]
fn hand_validate_missing_hand_toml_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_openfang"))
        .args(["hand", "validate", tmp.path().to_str().unwrap()])
        .output()
        .expect("spawn openfang");
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("[FAIL]"), "stdout={s}");
}

#[test]
fn hand_validate_missing_required_field_fails() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path()).unwrap();
    fs::write(
        tmp.path().join("HAND.toml"),
        r#"[hand]
id = "x"
name = "x"
version = "1.0.0"
# entrypoint intentionally omitted
"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_openfang"))
        .args(["hand", "validate", tmp.path().to_str().unwrap()])
        .output()
        .expect("spawn openfang");
    assert!(!out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("[FAIL]"), "stdout={s}");
}
