//! Proves legacy `ainl_graph_edges` (no FK metadata) is rebuilt with real REFERENCES on open.

use ainl_memory::SqliteGraphStore;
use rusqlite::Connection;
use uuid::Uuid;

#[test]
fn test_open_migrates_legacy_edges_to_foreign_keys() {
    let path = std::env::temp_dir().join(format!("ainl_fk_mig_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);

    let n1 = Uuid::new_v4();
    let n2 = Uuid::new_v4();
    let ghost = Uuid::new_v4();
    let minimal_payload = "{}";

    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ainl_graph_nodes (
                id TEXT PRIMARY KEY NOT NULL,
                node_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            );
            CREATE TABLE ainl_graph_edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                label TEXT NOT NULL,
                PRIMARY KEY (from_id, to_id, label)
            );
        "#,
        )
        .unwrap();
        for id in [n1, n2] {
            conn.execute(
                "INSERT INTO ainl_graph_nodes (id, node_type, payload, timestamp) VALUES (?1, 'episode', ?2, 0)",
                rusqlite::params![id.to_string(), minimal_payload],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO ainl_graph_edges (from_id, to_id, label) VALUES (?1, ?2, 'ok')",
            rusqlite::params![n1.to_string(), n2.to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ainl_graph_edges (from_id, to_id, label) VALUES (?1, ?2, 'bad')",
            rusqlite::params![n1.to_string(), ghost.to_string()],
        )
        .unwrap();
    }

    let store = SqliteGraphStore::open(&path).unwrap();
    drop(store);

    let conn = Connection::open(&path).unwrap();
    let fk_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('ainl_graph_edges')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        fk_count > 0,
        "expected REFERENCES on ainl_graph_edges after migration"
    );

    let edge_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ainl_graph_edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        edge_count, 1,
        "dangling legacy edge should be dropped during FK migration"
    );

    let store = SqliteGraphStore::open(&path).unwrap();
    let err = store
        .insert_graph_edge(n1, ghost, "after_migrate")
        .expect_err("FK must reject new dangling edge");
    assert!(
        err.to_lowercase().contains("foreign"),
        "unexpected error: {err}"
    );
}
