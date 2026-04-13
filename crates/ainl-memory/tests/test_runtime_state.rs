//! `RuntimeStateNode` persistence via [`SqliteGraphStore`].

use ainl_memory::{GraphStore, RuntimeStateNode, SqliteGraphStore};
use chrono::Utc;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rs.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn count_runtime_state_rows(store: &SqliteGraphStore, agent_id: &str) -> usize {
    store
        .find_by_type("runtime_state")
        .unwrap()
        .into_iter()
        .filter(|n| n.agent_id == agent_id)
        .count()
}

#[test]
fn test_save_and_load_runtime_state() {
    let (_d, store) = open_store();
    let ag = "agent-rs-1";
    let saved = RuntimeStateNode {
        agent_id: ag.to_string(),
        turn_count: 7,
        last_extraction_turn: 3,
        last_persona_prompt: Some("hello persona".into()),
        updated_at: Utc::now().to_rfc3339(),
    };
    store.save_runtime_state(&saved).unwrap();

    let loaded = store.load_runtime_state(ag).unwrap().expect("some");
    assert_eq!(loaded.agent_id, ag);
    assert_eq!(loaded.turn_count, 7);
    assert_eq!(loaded.last_extraction_turn, 3);
    assert_eq!(loaded.last_persona_prompt.as_deref(), Some("hello persona"));
    assert!(!loaded.updated_at.is_empty());
}

#[test]
fn test_save_is_idempotent() {
    let (_d, store) = open_store();
    let ag = "agent-rs-2";
    let mut s1 = RuntimeStateNode {
        agent_id: ag.to_string(),
        turn_count: 1,
        last_extraction_turn: 0,
        last_persona_prompt: None,
        updated_at: Utc::now().to_rfc3339(),
    };
    store.save_runtime_state(&s1).unwrap();
    assert_eq!(count_runtime_state_rows(&store, ag), 1);

    s1.turn_count = 99;
    s1.updated_at = Utc::now().to_rfc3339();
    store.save_runtime_state(&s1).unwrap();
    assert_eq!(count_runtime_state_rows(&store, ag), 1);

    let loaded = store.load_runtime_state(ag).unwrap().expect("some");
    assert_eq!(loaded.turn_count, 99);
}

#[test]
fn test_load_missing_returns_none() {
    let (_d, store) = open_store();
    assert!(store.load_runtime_state("no-such").unwrap().is_none());
}
