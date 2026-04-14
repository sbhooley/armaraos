//! Integration tests for AINL graph-memory substrate
//!
//! Tests proving the concept works:
//! - Episodes persist and query correctly
//! - Tool execution sequences store properly
//! - Semantic facts and confidence tracking work
//! - Graph traversal via edges functions

use ainl_memory::{
    count_by_topic_cluster, recall_delta_by_relevance, recall_flagged_episodes,
    recall_strength_history, AinlMemoryNode, AinlNodeType, GraphMemory, GraphStore, MemoryCategory,
    PersonaLayer, PersonaSource, ProcedureType, Sentiment, SqliteGraphStore, StrengthEvent,
};
use uuid::Uuid;

#[test]
fn test_write_episode_and_query() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_integration_episode.db");
    let _ = std::fs::remove_file(&db_path);

    let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

    // Write an episode with delegation
    let _episode_id = memory
        .write_episode(
            vec!["file_read".to_string(), "agent_delegate".to_string()],
            Some("agent-B".to_string()),
            None,
        )
        .expect("Failed to write episode");

    println!("✓ Created and wrote episode node");

    // Query it back
    let recent = memory.recall_recent(60).expect("Failed to recall");
    assert_eq!(recent.len(), 1);
    println!("✓ Retrieved episode from graph");

    // Verify content
    if let AinlNodeType::Episode { episodic } = &recent[0].node_type {
        assert_eq!(episodic.delegation_to, Some("agent-B".to_string()));
        assert_eq!(episodic.tool_calls.len(), 2);
        assert!(episodic
            .tool_calls
            .contains(&"agent_delegate".to_string()));
        println!("✓ Episode data validated");
    } else {
        panic!("Wrong node type");
    }

    println!("\n🎉 Episode write and query test passed!");
}

#[test]
fn test_semantic_facts_with_confidence() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_integration_facts.db");
    let _ = std::fs::remove_file(&db_path);

    let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

    let turn_id = Uuid::new_v4();

    // Write multiple facts with different confidence levels
    memory
        .write_fact("User prefers Rust over Python".to_string(), 0.95, turn_id)
        .expect("Failed to write fact");

    memory
        .write_fact("User works in fintech domain".to_string(), 0.78, turn_id)
        .expect("Failed to write fact");

    memory
        .write_fact("User might like Go".to_string(), 0.45, turn_id)
        .expect("Failed to write fact");

    println!("✓ Wrote 3 semantic facts");

    // Query high-confidence facts
    let high_conf =
        ainl_memory::find_high_confidence_facts(memory.store(), 0.7).expect("Query failed");

    assert_eq!(high_conf.len(), 2);
    println!("✓ Retrieved high-confidence facts");

    println!("\n🎉 Semantic memory test passed!");
}

#[test]
fn test_graph_traversal_with_edges() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_integration_edges.db");
    let _ = std::fs::remove_file(&db_path);

    let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

    // Create a chain of nodes: Episode -> Semantic fact
    let turn_id = Uuid::new_v4();
    let now = chrono::Utc::now().timestamp();

    let episode = AinlMemoryNode::new_episode(
        turn_id,
        now,
        vec!["agent_delegate".to_string()],
        Some("agent-B".to_string()),
        None,
    );

    let mut fact = AinlMemoryNode::new_fact("Delegation successful".to_string(), 0.90, turn_id);

    // Add edge from fact to episode
    fact.add_edge(episode.id, "learned_from");

    store.write_node(&episode).expect("Failed to write episode");
    store.write_node(&fact).expect("Failed to write fact");

    println!("✓ Created nodes with edges");

    // Walk the edge
    let connected = store
        .walk_edges(fact.id, "learned_from")
        .expect("Walk failed");

    assert_eq!(connected.len(), 1);
    assert_eq!(connected[0].id, episode.id);
    println!("✓ Graph traversal via edge works");

    println!("\n🎉 Graph traversal test passed!");
}

#[test]
fn test_procedural_pattern_storage() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_integration_patterns.db");
    let _ = std::fs::remove_file(&db_path);

    let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

    // Store a compiled pattern
    let pattern_id = memory
        .store_pattern(
            "research_workflow_v1".to_string(),
            vec![0x01, 0x02, 0x03, 0x04],
        )
        .expect("Failed to store pattern");

    assert_ne!(pattern_id, Uuid::nil());
    println!("✓ Stored procedural pattern");

    // Find it
    let patterns = ainl_memory::find_patterns(memory.store(), "research").expect("Query failed");

    assert_eq!(patterns.len(), 1);
    println!("✓ Retrieved pattern by prefix");

    println!("\n🎉 Procedural pattern test passed!");
}

#[test]
fn test_episode_with_trace_event() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_integration_trace.db");
    let _ = std::fs::remove_file(&db_path);

    let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

    // Simulate OrchestrationTraceEvent as JSON
    let trace_event = serde_json::json!({
        "trace_id": "trace-123",
        "orchestrator_id": "agent-A",
        "agent_id": "agent-B",
        "parent_agent_id": "agent-A",
        "event_type": {
            "type": "agent_delegated",
            "target_agent": "agent-B",
            "task": "Research Rust memory models"
        },
        "timestamp": "2026-04-12T00:00:00Z"
    });

    let _episode_id = memory
        .write_episode(
            vec!["agent_delegate".to_string()],
            Some("agent-B".to_string()),
            Some(trace_event),
        )
        .expect("Failed to write episode with trace");

    println!("✓ Wrote episode with OrchestrationTraceEvent");

    // Read it back
    let recent = memory.recall_recent(60).expect("Failed to recall");
    assert_eq!(recent.len(), 1);

    if let AinlNodeType::Episode { episodic } = &recent[0].node_type {
        assert!(episodic.trace_event.is_some());
        let trace = episodic.trace_event.as_ref().unwrap();
        assert_eq!(trace["trace_id"], "trace-123");
        println!("✓ Trace event preserved in Episode node");
    } else {
        panic!("Wrong node type");
    }

    println!("\n🎉 Trace event integration test passed!");
}

#[test]
fn test_metadata_roundtrip_all_categories() {
    let db_path = std::env::temp_dir().join("ainl_integration_roundtrip.db");
    let _ = std::fs::remove_file(&db_path);
    let store = SqliteGraphStore::open(&db_path).expect("open");

    let tid = Uuid::new_v4();
    let mut ep = AinlMemoryNode::new_episode(
        tid,
        1700000000,
        vec!["t1".into()],
        None,
        None,
    );
    ep.importance_score = 0.8;
    ep.agent_id = "ag1".into();
    ep.memory_category = MemoryCategory::Episodic;
    if let AinlNodeType::Episode { ref mut episodic } = ep.node_type {
        episodic.turn_index = 3;
        episodic.user_message_tokens = 10;
        episodic.assistant_response_tokens = 20;
        episodic.persona_signals_emitted = vec!["BrevityPreference".into()];
        episodic.sentiment = Some(Sentiment::Positive);
        episodic.flagged = true;
        episodic.conversation_id = "conv-1".into();
        episodic.follows_episode_id = Some("prior".into());
    }

    let mut sem = AinlMemoryNode::new_fact("fact".into(), 0.82, tid);
    sem.agent_id = "ag1".into();
    if let AinlNodeType::Semantic { ref mut semantic } = sem.node_type {
        semantic.topic_cluster = Some("rust".into());
        semantic.source_episode_id = tid.to_string();
        semantic.contradiction_ids = vec!["other".into()];
        semantic.last_referenced_at = 99;
        semantic.reference_count = 7;
        semantic.decay_eligible = false;
    }

    let mut proc = AinlMemoryNode::new_procedural_tools("p".into(), vec!["a".into()], 0.6);
    proc.agent_id = "ag1".into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.procedure_type = ProcedureType::BehavioralRule;
        procedural.trigger_conditions = vec!["when user asks".into()];
        procedural.success_count = 4;
        procedural.failure_count = 1;
        procedural.recompute_success_rate();
        procedural.last_invoked_at = 1234;
        procedural.reinforcement_episode_ids = vec!["e1".into()];
        procedural.suppression_episode_ids = vec!["e2".into()];
        procedural.trace_id = Some("proc-trace-roundtrip".into());
    }

    let mut per = AinlMemoryNode::new_persona("trait_x".into(), 0.55, vec![tid]);
    per.agent_id = "ag1".into();
    if let AinlNodeType::Persona { ref mut persona } = per.node_type {
        persona.layer = PersonaLayer::Delta;
        persona.source = PersonaSource::Evolved;
        persona.strength_floor = 0.1;
        persona.locked = true;
        persona.relevance_score = 0.91;
        persona.provenance_episode_ids = vec!["ep99".into()];
        persona.evolution_log = vec![StrengthEvent {
            delta: 0.05,
            reason: "Rule5".into(),
            episode_id: "ep1".into(),
            timestamp: 10,
        }];
    }

    store.write_node(&ep).unwrap();
    store.write_node(&sem).unwrap();
    store.write_node(&proc).unwrap();
    store.write_node(&per).unwrap();

    let ep2 = store.read_node(ep.id).unwrap().unwrap();
    let sem2 = store.read_node(sem.id).unwrap().unwrap();
    let proc2 = store.read_node(proc.id).unwrap().unwrap();
    let per2 = store.read_node(per.id).unwrap().unwrap();

    assert_eq!(ep2, ep);
    assert_eq!(sem2, sem);
    assert_eq!(proc2, proc);
    assert_eq!(per2, per);
}

#[test]
fn test_count_by_topic_cluster() {
    let db_path = std::env::temp_dir().join("ainl_integration_clusters.db");
    let _ = std::fs::remove_file(&db_path);
    let store = SqliteGraphStore::open(&db_path).expect("open");
    let tid = Uuid::new_v4();
    for (cluster, fact) in [("rust", "a"), ("rust", "b"), ("python", "c")] {
        let mut n = AinlMemoryNode::new_fact(fact.into(), 0.8, tid);
        n.agent_id = "agent-x".into();
        if let AinlNodeType::Semantic { ref mut semantic } = n.node_type {
            semantic.topic_cluster = Some(cluster.into());
        }
        store.write_node(&n).unwrap();
    }
    let counts = count_by_topic_cluster(&store, "agent-x").unwrap();
    assert_eq!(counts.get("rust").copied(), Some(2));
    assert_eq!(counts.get("python").copied(), Some(1));
}

#[test]
fn test_recall_flagged_episodes_only_flagged() {
    let db_path = std::env::temp_dir().join("ainl_integration_flagged.db");
    let _ = std::fs::remove_file(&db_path);
    let store = SqliteGraphStore::open(&db_path).expect("open");
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 100, vec![], None, None);
    a.agent_id = "a1".into();
    if let AinlNodeType::Episode { ref mut episodic } = a.node_type {
        episodic.flagged = false;
    }
    let mut b = AinlMemoryNode::new_episode(Uuid::new_v4(), 200, vec![], None, None);
    b.agent_id = "a1".into();
    if let AinlNodeType::Episode { ref mut episodic } = b.node_type {
        episodic.flagged = true;
    }
    store.write_node(&a).unwrap();
    store.write_node(&b).unwrap();
    let flagged = recall_flagged_episodes(&store, "a1", 10).unwrap();
    assert_eq!(flagged.len(), 1);
    assert!(flagged[0].flagged);
}

#[test]
fn test_recall_strength_history_sorted() {
    let db_path = std::env::temp_dir().join("ainl_integration_strength.db");
    let _ = std::fs::remove_file(&db_path);
    let store = SqliteGraphStore::open(&db_path).expect("open");
    let mut n = AinlMemoryNode::new_persona("t".into(), 0.5, vec![]);
    if let AinlNodeType::Persona { ref mut persona } = n.node_type {
        persona.evolution_log = vec![
            StrengthEvent {
                delta: 1.0,
                reason: "c".into(),
                episode_id: "c".into(),
                timestamp: 300,
            },
            StrengthEvent {
                delta: 1.0,
                reason: "a".into(),
                episode_id: "a".into(),
                timestamp: 100,
            },
            StrengthEvent {
                delta: 1.0,
                reason: "b".into(),
                episode_id: "b".into(),
                timestamp: 200,
            },
        ];
    }
    store.write_node(&n).unwrap();
    let hist = recall_strength_history(&store, n.id).unwrap();
    assert_eq!(hist.len(), 3);
    assert_eq!(hist[0].timestamp, 100);
    assert_eq!(hist[1].timestamp, 200);
    assert_eq!(hist[2].timestamp, 300);
}

#[test]
fn test_recall_delta_by_relevance_threshold() {
    let db_path = std::env::temp_dir().join("ainl_integration_delta_rel.db");
    let _ = std::fs::remove_file(&db_path);
    let store = SqliteGraphStore::open(&db_path).expect("open");
    let mut hi = AinlMemoryNode::new_persona("hi".into(), 0.9, vec![]);
    hi.agent_id = "z".into();
    if let AinlNodeType::Persona { ref mut persona } = hi.node_type {
        persona.layer = PersonaLayer::Delta;
        persona.relevance_score = 0.8;
    }
    let mut lo = AinlMemoryNode::new_persona("lo".into(), 0.9, vec![]);
    lo.agent_id = "z".into();
    if let AinlNodeType::Persona { ref mut persona } = lo.node_type {
        persona.layer = PersonaLayer::Delta;
        persona.relevance_score = 0.1;
    }
    let mut base = AinlMemoryNode::new_persona("base".into(), 0.9, vec![]);
    base.agent_id = "z".into();
    if let AinlNodeType::Persona { ref mut persona } = base.node_type {
        persona.layer = PersonaLayer::Base;
        persona.relevance_score = 0.99;
    }
    store.write_node(&hi).unwrap();
    store.write_node(&lo).unwrap();
    store.write_node(&base).unwrap();
    let deltas = recall_delta_by_relevance(&store, "z", 0.5).unwrap();
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].trait_name, "hi");
}

#[test]
fn test_legacy_node_json_deserializes() {
    let legacy = r#"{
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "node_type": {
            "type": "semantic",
            "fact": "legacy",
            "confidence": 0.6,
            "source_turn_id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8"
        },
        "edges": []
    }"#;
    let node: AinlMemoryNode = serde_json::from_str(legacy).expect("parse legacy");
    assert_eq!(node.memory_category, MemoryCategory::Semantic);
    assert!((node.importance_score - 0.5).abs() < f32::EPSILON);
    assert!(node.agent_id.is_empty());
    if let AinlNodeType::Semantic { semantic } = &node.node_type {
        assert_eq!(semantic.fact, "legacy");
        assert!((semantic.confidence - 0.6).abs() < 0.001);
        assert!(semantic.topic_cluster.is_none());
        assert!(semantic.source_episode_id.is_empty());
    } else {
        panic!("expected semantic");
    }
}
