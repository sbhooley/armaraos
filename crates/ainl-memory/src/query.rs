//! Graph traversal and querying utilities.
//!
//! Higher-level query functions built on top of GraphStore.

use crate::node::{AinlMemoryNode, AinlNodeType};
use crate::store::GraphStore;
use uuid::Uuid;

/// Walk the graph from a starting node, following edges with a specific label
///
/// # Arguments
/// * `store` - The graph store to query
/// * `start_id` - Node ID to start from
/// * `edge_label` - Label of edges to follow
/// * `max_depth` - Maximum depth to traverse (prevents infinite loops)
///
/// # Returns
/// Vector of nodes encountered during the walk, in breadth-first order
pub fn walk_from(
    store: &dyn GraphStore,
    start_id: Uuid,
    edge_label: &str,
    max_depth: usize,
) -> Result<Vec<AinlMemoryNode>, String> {
    let mut visited = std::collections::HashSet::new();
    let mut result = Vec::new();
    let mut current_level = vec![start_id];

    for _ in 0..max_depth {
        if current_level.is_empty() {
            break;
        }

        let mut next_level = Vec::new();

        for node_id in current_level {
            if visited.contains(&node_id) {
                continue;
            }
            visited.insert(node_id);

            if let Some(node) = store.read_node(node_id)? {
                result.push(node.clone());

                // Follow edges with the specified label
                for next_node in store.walk_edges(node_id, edge_label)? {
                    if !visited.contains(&next_node.id) {
                        next_level.push(next_node.id);
                    }
                }
            }
        }

        current_level = next_level;
    }

    Ok(result)
}

/// Recall recent episodes, optionally filtered by tool usage
///
/// # Arguments
/// * `store` - The graph store to query
/// * `since_timestamp` - Only return episodes after this timestamp (Unix seconds)
/// * `limit` - Maximum number of episodes to return
/// * `tool_filter` - If Some, only return episodes that used this tool
pub fn recall_recent(
    store: &dyn GraphStore,
    since_timestamp: i64,
    limit: usize,
    tool_filter: Option<&str>,
) -> Result<Vec<AinlMemoryNode>, String> {
    let episodes = store.query_episodes_since(since_timestamp, limit)?;

    if let Some(tool_name) = tool_filter {
        Ok(episodes
            .into_iter()
            .filter(|node| match &node.node_type {
                AinlNodeType::Episode { tool_calls, .. } => tool_calls.contains(&tool_name.to_string()),
                _ => false,
            })
            .collect())
    } else {
        Ok(episodes)
    }
}

/// Find procedural patterns by name prefix
///
/// # Arguments
/// * `store` - The graph store to query
/// * `name_prefix` - Pattern name prefix to match
///
/// # Returns
/// Vector of procedural nodes whose pattern_name starts with the prefix
pub fn find_patterns(
    store: &dyn GraphStore,
    name_prefix: &str,
) -> Result<Vec<AinlMemoryNode>, String> {
    let all_procedural = store.find_by_type("procedural")?;

    Ok(all_procedural
        .into_iter()
        .filter(|node| match &node.node_type {
            AinlNodeType::Procedural { pattern_name, .. } => {
                pattern_name.starts_with(name_prefix)
            }
            _ => false,
        })
        .collect())
}

/// Find semantic facts with confidence above a threshold
///
/// # Arguments
/// * `store` - The graph store to query
/// * `min_confidence` - Minimum confidence score (0.0-1.0)
///
/// # Returns
/// Vector of semantic nodes with confidence >= min_confidence
pub fn find_high_confidence_facts(
    store: &dyn GraphStore,
    min_confidence: f32,
) -> Result<Vec<AinlMemoryNode>, String> {
    let all_semantic = store.find_by_type("semantic")?;

    Ok(all_semantic
        .into_iter()
        .filter(|node| match &node.node_type {
            AinlNodeType::Semantic { confidence, .. } => *confidence >= min_confidence,
            _ => false,
        })
        .collect())
}

/// Find persona traits sorted by strength
///
/// # Arguments
/// * `store` - The graph store to query
///
/// # Returns
/// Vector of persona nodes sorted by strength (descending)
pub fn find_strong_traits(store: &dyn GraphStore) -> Result<Vec<AinlMemoryNode>, String> {
    let mut all_persona = store.find_by_type("persona")?;

    all_persona.sort_by(|a, b| {
        let strength_a = match &a.node_type {
            AinlNodeType::Persona { strength, .. } => *strength,
            _ => 0.0,
        };
        let strength_b = match &b.node_type {
            AinlNodeType::Persona { strength, .. } => *strength,
            _ => 0.0,
        };
        strength_b.partial_cmp(&strength_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(all_persona)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::AinlMemoryNode;
    use crate::store::SqliteGraphStore;

    #[test]
    fn test_recall_recent_with_tool_filter() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_query_test_recall.db");
        let _ = std::fs::remove_file(&db_path);

        let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

        let now = chrono::Utc::now().timestamp();

        // Create episodes with different tools
        let node1 = AinlMemoryNode::new_episode(
            uuid::Uuid::new_v4(),
            now,
            vec!["file_read".to_string()],
            None,
            None,
        );

        let node2 = AinlMemoryNode::new_episode(
            uuid::Uuid::new_v4(),
            now + 1,
            vec!["agent_delegate".to_string()],
            Some("agent-B".to_string()),
            None,
        );

        store.write_node(&node1).expect("Write failed");
        store.write_node(&node2).expect("Write failed");

        // Query with tool filter
        let delegations = recall_recent(&store, now - 100, 10, Some("agent_delegate"))
            .expect("Query failed");

        assert_eq!(delegations.len(), 1);
    }

    #[test]
    fn test_find_high_confidence_facts() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_query_test_facts.db");
        let _ = std::fs::remove_file(&db_path);

        let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

        let turn_id = uuid::Uuid::new_v4();

        let fact1 = AinlMemoryNode::new_fact("User prefers Rust".to_string(), 0.95, turn_id);
        let fact2 = AinlMemoryNode::new_fact("User dislikes Python".to_string(), 0.45, turn_id);

        store.write_node(&fact1).expect("Write failed");
        store.write_node(&fact2).expect("Write failed");

        let high_conf = find_high_confidence_facts(&store, 0.7).expect("Query failed");

        assert_eq!(high_conf.len(), 1);
    }
}
