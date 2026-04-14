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
fn test_write_and_read_runtime_state_round_trip() {
    let (_d, store) = open_store();
    let ag = "agent-rs-1";
    let saved = RuntimeStateNode {
        agent_id: ag.to_string(),
        turn_count: 7,
        last_extraction_at_turn: 3,
        persona_snapshot_json: serde_json::to_string("hello persona").ok(),
        updated_at: Utc::now().timestamp(),
    };
    store.write_runtime_state(&saved).unwrap();

    let loaded = store.read_runtime_state(ag).unwrap().expect("some");
    assert_eq!(loaded.agent_id, ag);
    assert_eq!(loaded.turn_count, 7);
    assert_eq!(loaded.last_extraction_at_turn, 3);
    assert_eq!(
        loaded.persona_snapshot_json.as_deref(),
        serde_json::to_string("hello persona").ok().as_deref()
    );
    assert!(loaded.updated_at > 0);
}

#[test]
fn test_write_runtime_state_is_idempotent() {
    let (_d, store) = open_store();
    let ag = "agent-rs-2";
    let mut s1 = RuntimeStateNode {
        agent_id: ag.to_string(),
        turn_count: 1,
        last_extraction_at_turn: 0,
        persona_snapshot_json: None,
        updated_at: Utc::now().timestamp(),
    };
    store.write_runtime_state(&s1).unwrap();
    assert_eq!(count_runtime_state_rows(&store, ag), 1);

    s1.turn_count = 99;
    s1.updated_at = Utc::now().timestamp();
    store.write_runtime_state(&s1).unwrap();
    assert_eq!(count_runtime_state_rows(&store, ag), 1);

    let loaded = store.read_runtime_state(ag).unwrap().expect("some");
    assert_eq!(loaded.turn_count, 99);
}

#[test]
fn test_read_runtime_state_missing_returns_none() {
    let (_d, store) = open_store();
    assert!(store.read_runtime_state("no-such").unwrap().is_none());
}

#[test]
fn test_graph_memory_runtime_state_api() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("gm_rs.db");
    let _ = std::fs::remove_file(&db);
    let mem = ainl_memory::GraphMemory::new(&db).unwrap();
    let ag = "gm-agent";
    let s = RuntimeStateNode {
        agent_id: ag.to_string(),
        turn_count: 42,
        last_extraction_at_turn: 10,
        persona_snapshot_json: None,
        updated_at: 1_700_000_000,
    };
    mem.write_runtime_state(&s).unwrap();
    let got = mem.read_runtime_state(ag).unwrap().expect("row");
    assert_eq!(got.turn_count, 42);
    assert_eq!(got.last_extraction_at_turn, 10);

    let via_query = mem.sqlite_store().query(ag).read_runtime_state().unwrap();
    assert_eq!(via_query.expect("q").turn_count, 42);
}
