//! GraphQuery API tests.

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use uuid::Uuid;

const AGENT: &str = "agent-query-tests";

fn open() -> (SqliteGraphStore, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("ainl_mem_query_{}.db", Uuid::new_v4()));
    let _ = std::fs::remove_file(&path);
    let store = SqliteGraphStore::open(&path).expect("open");
    (store, path)
}

fn set_episode_outcome(path: &std::path::Path, id: Uuid, outcome: &str) {
    let conn = rusqlite::Connection::open(path).expect("conn");
    let payload: String = conn
        .query_row(
            "SELECT payload FROM ainl_graph_nodes WHERE id = ?1",
            [id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .expect("row");
    let mut v: serde_json::Value = serde_json::from_str(&payload).expect("json");
    v["node_type"]["outcome"] = serde_json::Value::String(outcome.to_string());
    let s = v.to_string();
    conn.execute(
        "UPDATE ainl_graph_nodes SET payload = ?1 WHERE id = ?2",
        rusqlite::params![s, id.to_string()],
    )
    .expect("update");
}

#[test]
fn test_query_episodes() {
    let (store, _path) = open();
    for i in 0..3 {
        let mut n =
            AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_000 + i, vec![], None, None);
        n.agent_id = AGENT.into();
        store.write_node(&n).unwrap();
    }
    let q = store.query(AGENT);
    let eps = q.episodes().unwrap();
    assert_eq!(eps.len(), 3);
}

#[test]
fn test_query_recent_episodes_limit() {
    let (store, _path) = open();
    for i in 0..5 {
        let mut n =
            AinlMemoryNode::new_episode(Uuid::new_v4(), 1_800_000_000 + i, vec![], None, None);
        n.agent_id = AGENT.into();
        store.write_node(&n).unwrap();
    }
    let q = store.query(AGENT);
    let recent = q.recent_episodes(3).unwrap();
    assert_eq!(recent.len(), 3);
    let ts: Vec<i64> = recent
        .iter()
        .filter_map(|n| n.episodic().map(|e| e.timestamp))
        .collect();
    assert!(ts[0] >= ts[1] && ts[1] >= ts[2]);
}

#[test]
fn test_query_by_tag() {
    let (store, _path) = open();
    let mut n = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_100, vec![], None, None);
    n.agent_id = AGENT.into();
    if let AinlNodeType::Episode { ref mut episodic } = n.node_type {
        episodic.persona_signals_emitted = vec!["my_signal_tag".into()];
    }
    store.write_node(&n).unwrap();
    let found = store.query(AGENT).by_tag("my_signal_tag").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, n.id);
}

#[test]
fn test_query_successful_episodes() {
    let (store, path) = open();
    let mut ok = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_200, vec![], None, None);
    ok.agent_id = AGENT.into();
    store.write_node(&ok).unwrap();
    set_episode_outcome(&path, ok.id, "success");

    let mut bad = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_201, vec![], None, None);
    bad.agent_id = AGENT.into();
    store.write_node(&bad).unwrap();
    set_episode_outcome(&path, bad.id, "failure");

    let wins = store.query(AGENT).successful_episodes(10).unwrap();
    assert_eq!(wins.len(), 1);
    assert_eq!(wins[0].id, ok.id);
}

#[test]
fn test_query_neighbors() {
    let (store, _path) = open();
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_300, vec![], None, None);
    a.agent_id = AGENT.into();
    let mut b = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_301, vec![], None, None);
    b.agent_id = AGENT.into();
    store.write_node(&a).unwrap();
    store.write_node(&b).unwrap();
    store.insert_graph_edge(a.id, b.id, "REL").unwrap();
    let neigh = store.query(AGENT).neighbors(a.id, "REL").unwrap();
    assert_eq!(neigh.len(), 1);
    assert_eq!(neigh[0].id, b.id);
    let sub = store.query(AGENT).subgraph_edges().unwrap();
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].edge_type, "REL");
}

#[test]
fn test_query_lineage() {
    let (store, _path) = open();
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_400, vec![], None, None);
    a.agent_id = AGENT.into();
    let mut b = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_401, vec![], None, None);
    b.agent_id = AGENT.into();
    let mut c = AinlMemoryNode::new_episode(Uuid::new_v4(), 1_700_000_402, vec![], None, None);
    c.agent_id = AGENT.into();
    store.write_node(&a).unwrap();
    store.write_node(&b).unwrap();
    store.write_node(&c).unwrap();
    store.insert_graph_edge(a.id, b.id, "DERIVED_FROM").unwrap();
    store.insert_graph_edge(b.id, c.id, "DERIVED_FROM").unwrap();
    let chain = store.query(AGENT).lineage(a.id).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].id, b.id);
    assert_eq!(chain[1].id, c.id);
}

#[test]
fn test_query_pattern_by_name() {
    let (store, _path) = open();
    let mut p = AinlMemoryNode::new_pattern("unique_pat_x".into(), vec![]);
    p.agent_id = AGENT.into();
    store.write_node(&p).unwrap();
    let got = store
        .query(AGENT)
        .pattern_by_name("unique_pat_x")
        .unwrap()
        .expect("one");
    assert_eq!(got.id, p.id);
}
