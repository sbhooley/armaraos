//! Per-phase [`ainl_graph_extractor::ExtractionReport`] fields on [`GraphExtractorTask::run_pass`].

use ainl_graph_extractor::{run_extraction_pass, GraphExtractorTask};
use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use serde_json::json;
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("extraction_report.db");
    let store = SqliteGraphStore::open(&path).expect("open store");
    (dir, store)
}

#[test]
fn test_extract_error_populates_extract_error_field() {
    let (_d, store) = open_store();
    let mut task = GraphExtractorTask::new("agent-x");
    task.test_inject_extract_error = Some("injected extract".into());
    let report = task.run_pass(&store);
    assert_eq!(
        report.extract_error.as_deref(),
        Some("injected extract"),
        "{report:?}"
    );
    assert!(report.pattern_error.is_none(), "{report:?}");
    assert!(report.persona_error.is_none(), "{report:?}");
}

#[test]
fn test_pattern_write_failure_populates_pattern_error_only() {
    let (_d, store) = open_store();
    let mut task = GraphExtractorTask::new("agent-pat");
    task.test_inject_pattern_error = Some("injected pattern".into());
    let report = task.run_pass(&store);
    assert!(report.extract_error.is_none(), "{report:?}");
    assert_eq!(
        report.pattern_error.as_deref(),
        Some("injected pattern"),
        "{report:?}"
    );
    assert!(report.persona_error.is_none(), "{report:?}");
}

#[test]
fn test_persona_write_failure_populates_persona_error_only() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut ep = AinlMemoryNode::new_episode(
        tid,
        1,
        vec!["shell_exec".into()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = "agent-per".into();
    store.write_node(&ep).expect("write");

    let mut task = GraphExtractorTask::new("agent-per");
    task.test_inject_persona_error = Some("injected persona".into());
    let report = task.run_pass(&store);
    assert!(report.extract_error.is_none(), "{report:?}");
    assert!(report.pattern_error.is_none(), "{report:?}");
    assert_eq!(
        report.persona_error.as_deref(),
        Some("injected persona"),
        "{report:?}"
    );
}

#[test]
fn test_clean_run_has_no_errors() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut ep = AinlMemoryNode::new_episode(
        tid,
        1,
        vec!["noop".into()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = "agent-clean".into();
    store.write_node(&ep).expect("write");

    let report = run_extraction_pass(&store, "agent-clean");
    assert!(!report.has_errors(), "{report:?}");
}

#[test]
fn test_cold_graph_empty_signals_no_errors() {
    let (_d, store) = open_store();
    let report = run_extraction_pass(&store, "cold-agent");
    assert!(!report.has_errors(), "{report:?}");
    assert!(report.merged_signals.is_empty(), "{report:?}");
}
