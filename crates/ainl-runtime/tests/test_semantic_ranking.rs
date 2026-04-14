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

/// Gap L — vitals:pass tag gives a bonus that lifts a node above a zero-score peer.
///
/// Two nodes, neither has a matching topic cluster. The one tagged `vitals:reasoning:pass`
/// with high confidence should score higher than the untagged one.
#[test]
fn test_vitals_pass_tag_boosts_ranking_above_zero_score_peer() {
    let (_d, store) = open_store();
    let ag = "rank-vitals-boost";
    let tid = Uuid::new_v4();

    // Node A: no topic match, no vitals tag → score = 0.
    let mut plain_n = AinlMemoryNode::new_fact("plain-fact".into(), 0.7, tid);
    plain_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = plain_n.node_type {
        semantic.topic_cluster = Some("unrelated".into());
        semantic.recurrence_count = 5; // high recurrence — used as tiebreaker without vitals
    }
    store.write_node(&plain_n).unwrap();

    // Node B: no topic match, has vitals:reasoning:pass tag → score = 0.2 * confidence.
    let mut vitals_n = AinlMemoryNode::new_fact("vitals-fact".into(), 0.9, tid);
    vitals_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = vitals_n.node_type {
        semantic.topic_cluster = Some("unrelated".into());
        semantic.tags = vec!["vitals:reasoning:pass".into()];
        semantic.recurrence_count = 1; // low recurrence — should still win due to vitals bonus
    }
    store.write_node(&vitals_n).unwrap();

    let rt = AinlRuntime::new(rt_cfg(ag), store);
    // Use a query that produces topic tags but matches neither node's cluster.
    let ctx = rt
        .compile_memory_context_for(Some("help me with my rust crate"))
        .unwrap();

    // vitals-fact should rank first because its score (0.2 * 0.9 = 0.18) > plain-fact (0.0).
    let first = ctx
        .relevant_semantic
        .first()
        .and_then(|n| n.semantic().map(|s| s.fact.as_str()));
    assert_eq!(
        first,
        Some("vitals-fact"),
        "vitals:pass should boost score above zero-score peer; got {:?}",
        ctx.relevant_semantic
            .iter()
            .filter_map(|n| n.semantic().map(|s| s.fact.as_str()))
            .collect::<Vec<_>>()
    );
}

/// Gap L — vitals:elevated tag penalises a node below a clean peer.
#[test]
fn test_vitals_elevated_tag_penalises_ranking() {
    let (_d, store) = open_store();
    let ag = "rank-vitals-penalty";
    let tid = Uuid::new_v4();

    // Node A: matching topic, no vitals tag → score = 1.0.
    let mut good_n = AinlMemoryNode::new_fact("good-fact".into(), 0.8, tid);
    good_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = good_n.node_type {
        semantic.topic_cluster = Some("rust".into());
        semantic.recurrence_count = 1;
    }
    store.write_node(&good_n).unwrap();

    // Node B: matching topic + vitals:elevated → score = 1.0 - 0.1 = 0.9.
    let mut elevated_n = AinlMemoryNode::new_fact("elevated-fact".into(), 0.8, tid);
    elevated_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = elevated_n.node_type {
        semantic.topic_cluster = Some("rust".into());
        semantic.tags = vec!["vitals:elevated".into()];
        semantic.recurrence_count = 10; // higher recurrence but lower score due to penalty
    }
    store.write_node(&elevated_n).unwrap();

    let rt = AinlRuntime::new(rt_cfg(ag), store);
    let ctx = rt
        .compile_memory_context_for(Some("help me with rust crate"))
        .unwrap();

    let first = ctx
        .relevant_semantic
        .first()
        .and_then(|n| n.semantic().map(|s| s.fact.as_str()));
    assert_eq!(
        first,
        Some("good-fact"),
        "vitals:elevated penalty should lower score below clean peer; got {:?}",
        ctx.relevant_semantic
            .iter()
            .filter_map(|n| n.semantic().map(|s| s.fact.as_str()))
            .collect::<Vec<_>>()
    );
}
