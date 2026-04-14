//! Topic relevance + recurrence ranking for [`MemoryContext::relevant_semantic`].

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_runtime::{infer_topic_tags, AinlRuntime, RuntimeConfig};
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("sem_rank.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn rt_cfg(agent_id: &str) -> RuntimeConfig {
    RuntimeConfig {
        agent_id: agent_id.to_string(),
        extraction_interval: 0,
        max_steps: 100,
        ..Default::default()
    }
}

#[test]
fn test_relevant_semantic_nodes_ranks_matching_topic_first() {
    let (_d, store) = open_store();
    let ag = "rank-topic";
    let tid = Uuid::new_v4();

    let mut rust_n = AinlMemoryNode::new_fact("rust-fact".into(), 0.8, tid);
    rust_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = rust_n.node_type {
        semantic.topic_cluster = Some("rust,cargo".into());
        semantic.recurrence_count = 1;
    }
    store.write_node(&rust_n).unwrap();

    let mut trade_n = AinlMemoryNode::new_fact("trade-fact".into(), 0.8, tid);
    trade_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = trade_n.node_type {
        semantic.topic_cluster = Some("trading".into());
        semantic.recurrence_count = 10;
    }
    store.write_node(&trade_n).unwrap();

    let rt = AinlRuntime::new(rt_cfg(ag), store);
    let ctx = rt
        .compile_memory_context_for(Some("help me with my rust crate"))
        .unwrap();
    assert_eq!(
        ctx.relevant_semantic[0].semantic().map(|s| s.fact.as_str()),
        Some("rust-fact")
    );
    let tags = infer_topic_tags("help me with my rust crate");
    assert!(
        !tags.is_empty(),
        "sanity: tagger should infer at least one topic for ranking"
    );
}

#[test]
fn test_relevant_semantic_nodes_falls_back_to_recurrence_on_empty_message() {
    let (_d, store) = open_store();
    let ag = "rank-empty";
    let tid = Uuid::new_v4();

    for (fact, rec) in [("a", 2u32), ("b", 3), ("c", 5)] {
        let mut n = AinlMemoryNode::new_fact(fact.into(), 0.8, tid);
        n.agent_id = ag.into();
        if let AinlNodeType::Semantic { ref mut semantic } = n.node_type {
            semantic.recurrence_count = rec;
        }
        store.write_node(&n).unwrap();
    }

    let rt = AinlRuntime::new(rt_cfg(ag), store);
    let ctx = rt.compile_memory_context_for(None).unwrap();
    let facts: Vec<_> = ctx
        .relevant_semantic
        .iter()
        .filter_map(|n| n.semantic().map(|s| s.fact.as_str()))
        .collect();
    assert_eq!(facts, vec!["c", "b", "a"]);
}

#[test]
fn test_relevant_semantic_nodes_limit_is_respected() {
    let (_d, store) = open_store();
    let ag = "rank-limit";
    let tid = Uuid::new_v4();

    for i in 0..15u32 {
        let mut n = AinlMemoryNode::new_fact(format!("n{i}"), 0.8, tid);
        n.agent_id = ag.into();
        if let AinlNodeType::Semantic { ref mut semantic } = n.node_type {
            semantic.topic_cluster = Some("rust".into());
            semantic.recurrence_count = i + 1;
        }
        store.write_node(&n).unwrap();
    }

    let rt = AinlRuntime::new(rt_cfg(ag), store);
    let ctx = rt
        .compile_memory_context_for(Some("rust programming language"))
        .unwrap();
    assert_eq!(ctx.relevant_semantic.len(), 10);
}

#[test]
fn test_compile_memory_context_for_uses_user_message_for_ranking() {
    let (_d, store) = open_store();
    let ag = "rank-msg";
    let tid = Uuid::new_v4();

    let mut rust_n = AinlMemoryNode::new_fact("rust-fact".into(), 0.8, tid);
    rust_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = rust_n.node_type {
        semantic.topic_cluster = Some("rust,cargo".into());
        semantic.recurrence_count = 1;
    }
    store.write_node(&rust_n).unwrap();

    let mut trade_n = AinlMemoryNode::new_fact("trade-fact".into(), 0.8, tid);
    trade_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = trade_n.node_type {
        semantic.topic_cluster = Some("trading,crypto".into());
        semantic.recurrence_count = 50;
    }
    store.write_node(&trade_n).unwrap();

    let rt = AinlRuntime::new(rt_cfg(ag), store);

    let first_rust = rt
        .compile_memory_context_for(Some("help me with my rust crate"))
        .unwrap()
        .relevant_semantic
        .first()
        .and_then(|n| n.semantic().map(|s| s.fact.clone()))
        .expect("semantic");

    let first_trade = rt
        .compile_memory_context_for(Some("cryptocurrency trading strategy"))
        .unwrap()
        .relevant_semantic
        .first()
        .and_then(|n| n.semantic().map(|s| s.fact.clone()))
        .expect("semantic");

    assert_eq!(first_rust, "rust-fact");
    assert_eq!(first_trade, "trade-fact");
}

#[test]
fn test_no_semantic_nodes_returns_empty() {
    let (_d, store) = open_store();
    let ag = "rank-none";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 0, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let rt = AinlRuntime::new(rt_cfg(ag), store);
    let ctx = rt
        .compile_memory_context_for(Some("anything about rust"))
        .unwrap();
    assert!(ctx.relevant_semantic.is_empty());
}
