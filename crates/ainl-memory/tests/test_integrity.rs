//! write_node_with_edges transactional integrity tests.

use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use uuid::Uuid;

#[test]
fn test_write_with_edges_rejects_dangling() {
    let path = std::env::temp_dir().join(format!("ainl_integ_dang_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let mut store = SqliteGraphStore::open(&path).unwrap();
    let mut n = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_400_000_000, vec![], None, None);
    n.agent_id = "integ".into();
    n.add_edge(Uuid::new_v4(), "to_nowhere");
    let err = store.write_node_with_edges(&n).unwrap_err();
    assert!(err.contains("missing target"));
}

#[test]
fn test_write_with_edges_succeeds_valid() {
    let path = std::env::temp_dir().join(format!("ainl_integ_ok_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let mut store = SqliteGraphStore::open(&path).unwrap();
    let mut target = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_400_000_001, vec![], None, None);
    target.agent_id = "integ".into();
    store.write_node(&target).unwrap();

    let mut src = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_400_000_002, vec![], None, None);
    src.agent_id = "integ".into();
    src.add_edge(target.id, "points");
    store.write_node_with_edges(&src).unwrap();

    let neigh = store.walk_edges(src.id, "points").unwrap();
    assert_eq!(neigh.len(), 1);
    assert_eq!(neigh[0].id, target.id);
}
