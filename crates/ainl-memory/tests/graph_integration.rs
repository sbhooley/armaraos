//! Integration tests for AINL graph-memory substrate
//!
//! Tests proving the concept works:
//! - Episodes persist and query correctly
//! - Tool execution sequences store properly
//! - Semantic facts and confidence tracking work
//! - Graph traversal via edges functions

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory, GraphStore, SqliteGraphStore};
use uuid::Uuid;

#[test]
fn test_write_episode_and_query() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_integration_episode.db");
    let _ = std::fs::remove_file(&db_path);

    let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

    // Write an episode with delegation
    let episode_id = memory
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
    if let AinlNodeType::Episode {
        delegation_to,
        tool_calls,
        ..
    } = &recent[0].node_type
    {
        assert_eq!(delegation_to, &Some("agent-B".to_string()));
        assert_eq!(tool_calls.len(), 2);
        assert!(tool_calls.contains(&"agent_delegate".to_string()));
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

    let mut fact = AinlMemoryNode::new_fact(
        "Delegation successful".to_string(),
        0.90,
        turn_id,
    );

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
    let patterns =
        ainl_memory::find_patterns(memory.store(), "research").expect("Query failed");

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

    let episode_id = memory
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

    if let AinlNodeType::Episode { trace_event, .. } = &recent[0].node_type {
        assert!(trace_event.is_some());
        let trace = trace_event.as_ref().unwrap();
        assert_eq!(trace["trace_id"], "trace-123");
        println!("✓ Trace event preserved in Episode node");
    } else {
        panic!("Wrong node type");
    }

    println!("\n🎉 Trace event integration test passed!");
}
