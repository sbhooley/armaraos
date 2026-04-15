//! export_graph / import_graph tests.

use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use std::borrow::Cow;
use uuid::Uuid;

fn agent(i: u8) -> String {
    format!("snap-agent-{i}")
}

#[test]
fn test_export_import_roundtrip() {
    let path = std::env::temp_dir().join(format!("ainl_snap_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let ag = agent(1);
    let mut ids = Vec::new();
    for i in 0..5 {
        let mut n =
            AinlMemoryNode::new_episode(Uuid::new_v4(), 1_900_000_000 + i, vec![], None, None);
        n.agent_id = ag.clone();
        store.write_node(&n).unwrap();
        ids.push(n.id);
    }
    for w in ids.windows(2) {
        store.insert_graph_edge(w[0], w[1], "next").unwrap();
    }
    let snap = store.export_graph(&ag).unwrap();
    assert_eq!(snap.nodes.len(), 5);
    assert!(!snap.edges.is_empty());

    let path2 = std::env::temp_dir().join(format!("ainl_snap_import_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path2);
    let mut store2 = SqliteGraphStore::open(&path2).unwrap();
    store2.import_graph(&snap, false).unwrap();

    let snap2 = store2.export_graph(&ag).unwrap();
    assert_eq!(snap2.nodes.len(), 5);
    let idset: std::collections::HashSet<_> = snap2.nodes.iter().map(|n| n.id).collect();
    for id in &ids {
        assert!(idset.contains(id));
    }
}

#[test]
fn test_import_idempotent() {
    let path = std::env::temp_dir().join(format!("ainl_snap_idem_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let mut store = SqliteGraphStore::open(&path).unwrap();
    let ag = agent(2);
    let mut n = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_000_000_000, vec![], None, None);
    n.agent_id = ag.clone();
    store.write_node(&n).unwrap();
    let snap = store.export_graph(&ag).unwrap();
    store.import_graph(&snap, false).unwrap();
    store.import_graph(&snap, false).unwrap();
    let again = store.export_graph(&ag).unwrap();
    assert_eq!(again.nodes.len(), 1);
}

#[test]
fn test_export_agent_scoped() {
    let path = std::env::temp_dir().join(format!("ainl_snap_scope_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let a1 = agent(3);
    let a2 = agent(4);
    let mut n1 = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_100_000_000, vec![], None, None);
    n1.agent_id = a1.clone();
    let mut n2 = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_100_000_001, vec![], None, None);
    n2.agent_id = a2.clone();
    store.write_node(&n1).unwrap();
    store.write_node(&n2).unwrap();
    let snap = store.export_graph(&a1).unwrap();
    assert_eq!(snap.nodes.len(), 1);
    assert_eq!(snap.nodes[0].id, n1.id);
}

#[test]
fn test_agent_subgraph_edges_matches_export_edges() {
    let path = std::env::temp_dir().join(format!("ainl_snap_edges_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let ag = agent(5);
    let mut n1 = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_200_000_100, vec![], None, None);
    n1.agent_id = ag.clone();
    let mut n2 = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_200_000_101, vec![], None, None);
    n2.agent_id = ag.clone();
    store.write_node(&n1).unwrap();
    store.write_node(&n2).unwrap();
    store.insert_graph_edge(n1.id, n2.id, "next").unwrap();
    let direct = store.agent_subgraph_edges(&ag).unwrap();
    let snap = store.export_graph(&ag).unwrap();
    assert_eq!(direct.len(), snap.edges.len());
    assert_eq!(snap.edges.len(), 1);
}

#[test]
fn test_graph_memory_forwards_validate_and_edges() {
    use ainl_memory::GraphMemory;
    let path = std::env::temp_dir().join(format!("ainl_gm_fwd_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let mem = GraphMemory::new(&path).unwrap();
    let ag = "gm-fwd";
    let mut n = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_200_000_200, vec![], None, None);
    n.agent_id = ag.into();
    mem.write_node(&n).unwrap();
    let r = mem.validate_graph(ag).unwrap();
    assert!(r.is_valid);
    let edges = mem.agent_subgraph_edges(ag).unwrap();
    assert!(edges.is_empty());
}

#[test]
fn test_import_rejects_unknown_snapshot_schema_version() {
    let path = std::env::temp_dir().join(format!("ainl_snap_schema_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).unwrap();
    let ag = agent(6);

    let mut n = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_200_000_300, vec![], None, None);
    n.agent_id = ag.clone();
    store.write_node(&n).unwrap();

    let mut snap = store.export_graph(&ag).unwrap();
    snap.schema_version = Cow::Owned("999.0".to_string());

    let path2 = std::env::temp_dir().join(format!("ainl_snap_schema_import_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path2);
    let mut store2 = SqliteGraphStore::open(&path2).unwrap();

    let err = store2
        .import_graph(&snap, false)
        .expect_err("unknown schema version must fail import");
    assert!(err.contains("schema_version"), "unexpected error: {err}");
}
