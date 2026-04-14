//! CLI-oriented validation for Hand directories (emitter packs and dashboard hands).

use crate::{parse_hand_toml, HandDefinition};
use serde_json::Value as JsonValue;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use toml::Table;
use tracing::warn;

/// Structured result from [`validate_hand_path`].
#[derive(Debug, Default, Clone)]
pub struct ValidationReport {
    pub lines: Vec<String>,
    pub errors: usize,
    pub warns: usize,
}

impl ValidationReport {
    fn push_pass(&mut self, msg: impl Display) {
        self.lines.push(format!("[PASS] {msg}"));
    }
    fn push_warn(&mut self, msg: impl Display) {
        self.warns += 1;
        self.lines.push(format!("[WARN] {msg}"));
    }
    fn push_fail(&mut self, msg: impl Display) {
        self.errors += 1;
        self.lines.push(format!("[FAIL] {msg}"));
    }

    pub fn print_summary(&self) {
        for line in &self.lines {
            println!("{line}");
        }
        if self.errors == 0 {
            let tail = if self.warns > 0 {
                " — Hand is loadable but should be regenerated."
            } else {
                " — Hand is valid."
            };
            println!(
                "\nResult: {} warning(s), {} error(s){}",
                self.warns, self.errors, tail
            );
        } else {
            println!(
                "\nResult: {} warning(s), {} error(s) — fix failures before install.",
                self.warns, self.errors
            );
        }
    }
}

/// Alpha policy: log only; loading never fails on schema (see TODO on [`crate::HandError::SchemaMismatch`]).
pub fn validate_hand_schema_version(hand: &HandDefinition) {
    match &hand.schema_version {
        None => {
            warn!(
                hand_id = %hand.id,
                "HAND.toml missing schema_version; assuming legacy format. \
                 Regenerate with `ainl emit --target armaraos` to add schema_version."
            );
        }
        Some(v) if v == crate::HAND_SCHEMA_VERSION => {}
        Some(v) => {
            warn!(
                hand_id = %hand.id,
                found = %v,
                expected = %crate::HAND_SCHEMA_VERSION,
                "HAND.toml schema_version mismatch. Hand may be incompatible. \
                 Regenerate with `ainl emit --target armaraos`."
            );
        }
    }
}

/// When installing from a directory, warn if an emitter-style `[hand].entrypoint` JSON is present
/// and its `schema_version` is missing or not [`AINL_IR_SCHEMA_VERSION`].
pub fn warn_emitter_ir_schema_mismatch(hand_dir: &Path, toml_content: &str) {
    let Ok(tv) = toml::from_str::<Table>(toml_content) else {
        return;
    };
    let Some(hand) = tv.get("hand").and_then(|h| h.as_table()) else {
        return;
    };
    let Some(ep) = hand.get("entrypoint").and_then(|e| e.as_str()) else {
        return;
    };
    if !ep.ends_with(".json") {
        return;
    }
    let ir_path = hand_dir.join(ep);
    let Ok(raw) = std::fs::read_to_string(&ir_path) else {
        return;
    };
    let Ok(j) = serde_json::from_str::<JsonValue>(&raw) else {
        warn!(
            path = %ir_path.display(),
            "entrypoint JSON does not parse; skipping IR schema_version check"
        );
        return;
    };
    match j.get("schema_version").and_then(|x| x.as_str()) {
        None => {
            warn!(
                path = %ir_path.display(),
                "ainl.json missing schema_version; legacy IR. Regenerate with `ainl emit --target armaraos`."
            );
        }
        Some(s) if s == crate::AINL_IR_SCHEMA_VERSION => {}
        Some(s) => {
            warn!(
                path = %ir_path.display(),
                found = %s,
                expected = %crate::AINL_IR_SCHEMA_VERSION,
                "ainl.json schema_version mismatch; IR may be incompatible with this OpenFang build."
            );
        }
    }
}

fn resolve_hand_root(path: &Path) -> (PathBuf, PathBuf) {
    if path.is_file() && path.file_name().is_some_and(|n| n == "HAND.toml") {
        let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        return (root, path.to_path_buf());
    }
    let root = path.to_path_buf();
    (root.clone(), root.join("HAND.toml"))
}

/// Validate a Hand directory or a path to `HAND.toml`. Warnings do not set `errors`.
pub fn validate_hand_path(path: &Path) -> ValidationReport {
    let mut r = ValidationReport::default();
    let (root, hand_path) = resolve_hand_root(path);
    r.lines
        .push(format!("Validating Hand at: {}", root.display()));

    let raw = match std::fs::read_to_string(&hand_path) {
        Ok(s) => s,
        Err(e) => {
            r.push_fail(format!(
                "HAND.toml: cannot read {}: {e}",
                hand_path.display()
            ));
            return r;
        }
    };
    r.push_pass(format!("HAND.toml: found ({})", hand_path.display()));

    let tv: Table = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            r.push_fail(format!("HAND.toml: invalid TOML: {e}"));
            return r;
        }
    };
    r.push_pass("HAND.toml: valid TOML".to_string());

    if let Some(ht) = tv.get("hand").and_then(|h| h.as_table()) {
        let mut missing = Vec::new();
        for k in ["id", "name", "version", "entrypoint"] {
            if !ht.contains_key(k) {
                missing.push(k);
            }
        }
        if missing.is_empty() {
            r.push_pass("Required fields (emitter `[hand]`): id, name, version, entrypoint");
        } else {
            r.push_fail(format!(
                "Required fields (emitter `[hand]`): missing {}",
                missing.join(", ")
            ));
        }

        match ht.get("schema_version").and_then(|x| x.as_str()) {
            None => {
                r.push_warn(
                    "schema_version: missing (legacy format — regenerate with `ainl emit --target armaraos`)",
                );
            }
            Some(s) if s == crate::HAND_SCHEMA_VERSION => {
                r.push_pass(format!(
                    "schema_version: matches expected ({})",
                    crate::HAND_SCHEMA_VERSION
                ));
            }
            Some(s) => {
                r.push_warn(format!(
                    "schema_version: found {s}, expected {} — regenerate with `ainl emit --target armaraos`",
                    crate::HAND_SCHEMA_VERSION
                ));
            }
        }

        if let Some(ep) = ht.get("entrypoint").and_then(|e| e.as_str()) {
            let p = root.join(ep);
            if p.is_file() {
                r.push_pass(format!("Entrypoint: {ep} found"));
                if ep.ends_with(".json") {
                    if let Ok(ir_raw) = std::fs::read_to_string(&p) {
                        if let Ok(j) = serde_json::from_str::<JsonValue>(&ir_raw) {
                            match j.get("schema_version").and_then(|x| x.as_str()) {
                                None => {
                                    r.push_warn(
                                        "ainl.json schema_version: missing (legacy IR — regenerate)",
                                    );
                                }
                                Some(s) if s == crate::AINL_IR_SCHEMA_VERSION => {
                                    r.push_pass(format!(
                                        "ainl.json schema_version: matches ({})",
                                        crate::AINL_IR_SCHEMA_VERSION
                                    ));
                                }
                                Some(s) => {
                                    r.push_warn(format!(
                                        "ainl.json schema_version: found {s}, expected {}",
                                        crate::AINL_IR_SCHEMA_VERSION
                                    ));
                                }
                            }
                        } else {
                            r.push_warn("ainl.json: could not parse as JSON for schema_version check");
                        }
                    } else {
                        r.push_fail(format!("Entrypoint file unreadable: {}", p.display()));
                    }
                }
            } else {
                r.push_fail(format!("Entrypoint: {} not found", p.display()));
            }
        } else if missing.is_empty() {
            r.push_fail("entrypoint: missing under [hand]");
        }
    } else {
        match parse_hand_toml(&raw) {
            Ok(def) => {
                validate_hand_schema_version(&def);
                if def.id.is_empty() || def.name.is_empty() {
                    r.push_fail("dashboard hand: id and name must be non-empty");
                } else {
                    r.push_pass(
                        "Required fields (dashboard HandDefinition): id, name, description, category, agent",
                    );
                }
                match &def.schema_version {
                    None => {
                        r.push_warn(
                            "schema_version: missing (legacy format — regenerate with `ainl emit --target armaraos`)",
                        );
                    }
                    Some(s) if s == crate::HAND_SCHEMA_VERSION => {
                        r.push_pass(format!(
                            "schema_version: matches expected ({})",
                            crate::HAND_SCHEMA_VERSION
                        ));
                    }
                    Some(s) => {
                        r.push_warn(format!(
                            "schema_version: found {s}, expected {}",
                            crate::HAND_SCHEMA_VERSION
                        ));
                    }
                }
                r.push_pass("Entrypoint / ainl.json: N/A (dashboard hand manifest, no IR entrypoint in TOML)");
            }
            Err(e) => {
                r.push_fail(format!(
                    "HAND.toml: not emitter `[hand]` shape and not a valid dashboard hand: {e}"
                ));
            }
        }
    }

    let sec_path = root.join("security.json");
    if sec_path.is_file() {
        match std::fs::read_to_string(&sec_path) {
            Ok(s) => match serde_json::from_str::<JsonValue>(&s) {
                Ok(j) => match j.get("schema_version").and_then(|x| x.as_str()) {
                    None => r.push_warn(format!(
                        "security.json: missing schema_version ({})",
                        sec_path.display()
                    )),
                    Some(v) if v == crate::HAND_SCHEMA_VERSION => {
                        r.push_pass("security.json: schema_version present and matches");
                    }
                    Some(v) => r.push_warn(format!(
                        "security.json: schema_version {v} != expected {}",
                        crate::HAND_SCHEMA_VERSION
                    )),
                },
                Err(e) => r.push_warn(format!(
                    "security.json: invalid JSON ({}): {e}",
                    sec_path.display()
                )),
            },
            Err(e) => r.push_warn(format!(
                "security.json: cannot read {}: {e}",
                sec_path.display()
            )),
        }
    } else {
        r.push_warn(format!(
            "security.json: not found at {} (optional for dashboard hands)",
            sec_path.display()
        ));
    }

    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(p: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(p).unwrap();
        let mut f = std::fs::File::create(p.join(name)).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn validate_emitter_pack_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "HAND.toml",
            r#"[hand]
id = "ainl-test"
name = "Test"
version = "1.0.0"
ainl_ir_version = "1"
schema_version = "1"
description = "d"
author = "a"
entrypoint = "t.ainl.json"
"#,
        );
        write(root, "t.ainl.json", r#"{"schema_version":"1","labels":{}}"#);
        write(root, "security.json", r#"{"schema_version":"1","version":"1.0"}"#);
        let r = validate_hand_path(root);
        assert_eq!(r.errors, 0, "{:?}", r.lines);
    }

    #[test]
    fn validate_emitter_future_hand_schema_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "HAND.toml",
            r#"[hand]
id = "ainl-x"
name = "X"
version = "1.0.0"
schema_version = "999"
entrypoint = "t.ainl.json"
"#,
        );
        write(root, "t.ainl.json", r#"{"schema_version":"1"}"#);
        let r = validate_hand_path(root);
        assert_eq!(r.errors, 0);
        assert!(r.warns >= 1);
        assert!(r.lines.iter().any(|l| l.contains("999")));
    }

    #[test]
    fn validate_emitter_missing_hand_schema_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "HAND.toml",
            r#"[hand]
id = "ainl-x"
name = "X"
version = "1.0.0"
entrypoint = "t.ainl.json"
"#,
        );
        write(root, "t.ainl.json", r#"{"schema_version":"1"}"#);
        let r = validate_hand_path(root);
        assert_eq!(r.errors, 0);
        assert!(r.warns >= 1);
        assert!(r.lines.iter().any(|l| l.contains("schema_version")));
    }

    #[test]
    fn validate_missing_hand_toml_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let r = validate_hand_path(tmp.path());
        assert!(r.errors >= 1);
    }

    #[test]
    fn validate_emitter_missing_required_field_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(
            root,
            "HAND.toml",
            r#"[hand]
name = "X"
version = "1.0.0"
entrypoint = "t.ainl.json"
"#,
        );
        write(root, "t.ainl.json", "{}");
        let r = validate_hand_path(root);
        assert!(r.errors >= 1);
    }

    #[test]
    fn validate_bundled_clip_style_warns_missing_schema_version() {
        let (id, tom, _skill) = crate::bundled::bundled_hands()[0];
        assert_eq!(id, "clip");
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("HAND.toml"), tom).unwrap();
        let r = validate_hand_path(tmp.path());
        assert_eq!(r.errors, 0, "{:?}", r.lines);
        assert!(r.warns >= 1, "{:?}", r.lines);
    }

    #[test]
    fn hand_schema_version_constant_is_one() {
        assert_eq!(crate::HAND_SCHEMA_VERSION, "1");
        assert_eq!(crate::AINL_IR_SCHEMA_VERSION, "1");
    }
}
