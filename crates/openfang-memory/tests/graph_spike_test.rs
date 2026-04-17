//! Integration test: AINL graph-memory spike
//!
//! This test proves the concept works end-to-end:
//! 1. Write a delegation episode node
//! 2. Query it back
//! 3. Walk edges
//!
//! Once this passes, we extract to `ainl-memory` crate.

use openfang_memory::graph::{AinlMemoryNode, GraphStore, SqliteGraphStore};

#[test]
fn test_delegation_episode_write_and_query() {
    // Setup: Create temp DB
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_spike_integration.db");
    let _ = std::fs::remove_file(&db_path);

    let store = SqliteGraphStore::open(&db_path).expect("Failed to open graph store");

    // Simulate orchestration: Agent A delegates to Agent B
    let agent_a = "agent-orchestrator";
    let agent_b = "agent-code-helper";
    let trace_id = "trace-12345";

    let mut node = AinlMemoryNode::new_delegation_episode(
        agent_a.to_string(),
        agent_b.to_string(),
        trace_id.to_string(),
        1,
    );

    println!("✓ Created delegation episode node: {:?}", node.id);

    // Write to graph
    store
        .write_node(&node)
        .expect("Failed to write delegation node");

    println!("✓ Wrote node to graph storage");

    // Query it back
    let retrieved = store
        .read_node(node.id)
        .expect("Failed to query")
        .expect("Node not found");

    println!("✓ Retrieved node from graph: {:?}", retrieved.id);

    // Verify the data
    assert_eq!(retrieved.id, node.id);
    match retrieved.node_type {
        openfang_memory::graph::AinlNodeType::Episode {
            ref agent_id,
            ref delegation_to,
            ref trace_id,
            depth,
            ..
        } => {
            assert_eq!(agent_id, agent_a);
            assert_eq!(delegation_to, &Some(agent_b.to_string()));
            assert_eq!(trace_id, &Some("trace-12345".to_string()));
            assert_eq!(depth, 1);
            println!("✓ Episode data validated");
        }
        _ => panic!("Expected Episode node type"),
    }

    // Query recent episodes for agent_a
    let episodes = store
        .query_recent_episodes(agent_a, 10)
        .expect("Failed to query recent");

    assert_eq!(episodes.len(), 1);
    println!("✓ Query recent episodes returned 1 result");

    // Now add a second delegation (Agent B delegates to Agent C)
    let agent_c = "agent-researcher";
    let child_node = AinlMemoryNode::new_delegation_episode(
        agent_b.to_string(),
        agent_c.to_string(),
        trace_id.to_string(),
        2,
    );

    store
        .write_node(&child_node)
        .expect("Failed to write child node");

    // Add edge from parent to child
    node.add_edge(child_node.id, "delegated_to");
    store
        .write_node(&node)
        .expect("Failed to update parent with edge");

    println!("✓ Created delegation chain: A → B → C");

    // Walk the edge
    let targets = store
        .walk_edges(node.id, "delegated_to")
        .expect("Failed to walk edges");

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].id, child_node.id);
    println!("✓ Graph traversal works: found child node via edge");

    println!("\n🎉 SPIKE SUCCESSFUL! Graph-memory concept proven.");
    println!("   - Delegation episodes written as graph nodes");
    println!("   - Query by agent ID works");
    println!("   - Edge traversal works");
    println!("\nNext step: Extract to `ainl-memory` crate");
}

#[test]
fn test_multiple_tool_calls_in_episode() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_spike_tools.db");
    let _ = std::fs::remove_file(&db_path);

    let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

    // Create episode with multiple tool uses
    let agent_id = "agent-multi-tool";
    let mut node1 = AinlMemoryNode::new_tool_episode(agent_id.to_string(), "web_fetch".to_string());
    let mut node2 =
        AinlMemoryNode::new_tool_episode(agent_id.to_string(), "file_write".to_string());
    let node3 = AinlMemoryNode::new_tool_episode(agent_id.to_string(), "shell_exec".to_string());

    // Link them in sequence
    node1.add_edge(node2.id, "next");
    node2.add_edge(node3.id, "next");

    store.write_node(&node1).expect("Write failed");
    store.write_node(&node2).expect("Write failed");
    store.write_node(&node3).expect("Write failed");

    // Walk the tool execution chain
    let step2 = store.walk_edges(node1.id, "next").expect("Walk failed");
    assert_eq!(step2.len(), 1);
    assert_eq!(step2[0].id, node2.id);

    let step3 = store.walk_edges(step2[0].id, "next").expect("Walk failed");
    assert_eq!(step3.len(), 1);
    assert_eq!(step3[0].id, node3.id);

    println!("✓ Multi-tool episode chain works: web_fetch → file_write → shell_exec");
}

#[test]
fn test_semantic_memory_nodes() {
    use openfang_memory::graph::{AinlEdge, AinlMemoryNode, AinlNodeType};
    use uuid::Uuid;

    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("ainl_spike_semantic.db");
    let _ = std::fs::remove_file(&db_path);

    let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

    // Episode where we learned a fact
    let source_turn = Uuid::new_v4();

    // Create semantic memory node: "User prefers Python over JavaScript"
    let fact_node = AinlMemoryNode {
        id: Uuid::new_v4(),
        node_type: AinlNodeType::Semantic {
            fact: "User prefers Python over JavaScript".to_string(),
            confidence: 0.92,
            source_turn,
        },
        timestamp: chrono::Utc::now().timestamp(),
        edges: vec![AinlEdge {
            target_id: source_turn,
            label: "learned_from".to_string(),
        }],
    };

    store.write_node(&fact_node).expect("Write failed");

    // Query semantic nodes
    let facts = store.query_by_type("semantic").expect("Query failed");

    assert_eq!(facts.len(), 1);
    match &facts[0].node_type {
        AinlNodeType::Semantic {
            fact, confidence, ..
        } => {
            assert!(fact.contains("Python"));
            assert!(*confidence > 0.9);
            println!(
                "✓ Semantic memory node: {} (confidence: {})",
                fact, confidence
            );
        }
        _ => panic!("Expected Semantic node"),
    }
}
