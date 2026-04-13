//! validate_graph tests.

use ainl_memory::{
    AgentGraphSnapshot, AinlMemoryNode, GraphStore, SnapshotEdge, SqliteGraphStore,
    SNAPSHOT_SCHEMA_VERSION,
};
use std::borrow::Cow;
use uuid::Uuid;

#[test]
fn test_validate_clean_graph() {
    let path = std::env::temp_dir().join(format!("ainl_val_clean_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let ag = "val-clean";
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_200_000_000, vec![], None, None);
    a.agent_id = ag.into();
    let mut b = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_200_000_001, vec![], None, None);
    b.agent_id = ag.into();
    store.write_node(&a).unwrap();
    store.write_node(&b).unwrap();
    store.insert_graph_edge(a.id, b.id, "x").unwrap();
    let r = store.validate_graph(ag).unwrap();
    assert!(r.is_valid);
    assert!(r.dangling_edges.is_empty());
    assert_eq!(r.node_count, 2);
    assert_eq!(r.edge_count, 1);
}

#[test]
fn test_validate_detects_dangling() {
    let path = std::env::temp_dir().join(format!("ainl_val_dang_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let ag = "val-dang";
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_300_000_000, vec![], None, None);
    a.agent_id = ag.to_string();
    let aid = a.id;
    let ghost = Uuid::new_v4();
    let snap = AgentGraphSnapshot {
        agent_id: ag.to_string(),
        exported_at: chrono::Utc::now(),
        schema_version: Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION),
        nodes: vec![a],
        edges: vec![SnapshotEdge {
            source_id: aid,
            target_id: ghost,
            edge_type: "broken".to_string(),
            weight: 1.0,
            metadata: None,
        }],
    };
    let mut store = SqliteGraphStore::open(&path).unwrap();
    store.import_graph(&snap, true).unwrap();
    let r = store.validate_graph(ag).unwrap();
    assert!(!r.is_valid);
    assert_eq!(r.dangling_edges.len(), 1);
    assert_eq!(r.dangling_edge_details.len(), 1);
    assert_eq!(r.dangling_edge_details[0].edge_type, "broken");
    assert_eq!(r.cross_agent_boundary_edges, 0);
}

#[test]
fn test_validate_cross_agent_boundary_counts() {
    let path = std::env::temp_dir().join(format!("ainl_val_cross_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let a1 = "cross-a1";
    let a2 = "cross-a2";
    let mut n1 = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_400_000_000, vec![], None, None);
    n1.agent_id = a1.into();
    let mut n2 = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_400_000_001, vec![], None, None);
    n2.agent_id = a2.into();
    store.write_node(&n1).unwrap();
    store.write_node(&n2).unwrap();
    store.insert_graph_edge(n1.id, n2.id, "handoff").unwrap();
    let r = store.validate_graph(a1).unwrap();
    assert!(r.is_valid);
    assert_eq!(r.cross_agent_boundary_edges, 1);
    assert!(r.dangling_edge_details.is_empty());
}

#[test]
fn test_insert_graph_edge_checked_rejects_missing_target() {
    let path = std::env::temp_dir().join(format!("ainl_val_chk_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_500_000_000, vec![], None, None);
    a.agent_id = "chk".into();
    store.write_node(&a).unwrap();
    let ghost = Uuid::new_v4();
    let err = store
        .insert_graph_edge_checked(a.id, ghost, "x")
        .expect_err("missing target");
    assert!(err.contains("missing target"));
}

#[test]
fn test_import_strict_rejects_dangling() {
    let path = std::env::temp_dir().join(format!("ainl_val_strict_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let ag = "val-strict";
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_300_000_100, vec![], None, None);
    a.agent_id = ag.to_string();
    let aid = a.id;
    let snap = AgentGraphSnapshot {
        agent_id: ag.to_string(),
        exported_at: chrono::Utc::now(),
        schema_version: Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION),
        nodes: vec![a],
        edges: vec![SnapshotEdge {
            source_id: aid,
            target_id: Uuid::new_v4(),
            edge_type: "broken".to_string(),
            weight: 1.0,
            metadata: None,
        }],
    };
    let mut store = SqliteGraphStore::open(&path).unwrap();
    let err = store.import_graph(&snap, false).expect_err("strict import must fail");
    assert!(
        err.to_lowercase().contains("foreign"),
        "unexpected error: {err}"
    );
}
